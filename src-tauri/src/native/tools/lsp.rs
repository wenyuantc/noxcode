//! 本地 Language Server 客户端：按文件扩展名懒启动，提供导航与诊断。
//! SSH 工作区不启动 language server。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};

use crate::native::tools::command_path::resolve_program;
use crate::process_spawn::tokio_command;

use super::paths::resolve_under_workspace;

#[derive(Clone)]
pub struct LspHub {
    root: PathBuf,
    enabled: bool,
    inner: Arc<Mutex<HubState>>,
}

#[derive(Default)]
struct HubState {
    servers: HashMap<String, Arc<LanguageServer>>,
}

struct LanguageServer {
    language: String,
    language_id: String,
    command: String,
    next_id: AtomicI64,
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Value>>>,
    diagnostics: Mutex<HashMap<String, Vec<LspDiagnostic>>>,
    opened: Mutex<HashMap<String, i64>>,
    _child: Child,
}

#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub path: String,
    pub line: u64,
    pub character: u64,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LspServerStatus {
    pub id: String,
    pub label: String,
    pub commands: Vec<String>,
    pub installed_command: Option<String>,
    pub install_command: Option<String>,
    pub installable: bool,
}

struct InstallCommand {
    program: &'static str,
    args: &'static [&'static str],
    display: &'static str,
}

impl LspHub {
    pub fn new(root: PathBuf, enabled: bool) -> Self {
        Self {
            root,
            enabled,
            inner: Arc::new(Mutex::new(HubState::default())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub async fn shutdown(&self) {
        let mut state = self.inner.lock().await;
        state.servers.clear();
    }

    pub async fn query(&self, arguments: &str) -> Result<String, String> {
        if !self.enabled {
            return Err("LSP 已在设置中关闭".to_string());
        }
        let args: Value = if arguments.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(arguments)
                .map_err(|error| format!("工具参数不是合法 JSON: {error}"))?
        };
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .unwrap_or("diagnostics");
        let file_path = args
            .get("file_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .unwrap_or("");
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let line = args.get("line").and_then(Value::as_u64).unwrap_or(1);
        let character = args.get("character").and_then(Value::as_u64).unwrap_or(1);
        let requested_language = args
            .get("language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(normalize_language)
            .transpose()?;
        if operation == "workspaceSymbol" {
            let file_language = if file_path.is_empty() {
                None
            } else {
                let resolved = resolve_under_workspace(&self.root, file_path)?;
                language_for_path(&resolved)
            };
            let language = select_workspace_language(requested_language, file_language, query)?;
            let server = self.ensure_server(language).await?;
            let result = server
                .request("workspace/symbol", json!({ "query": query }))
                .await?;
            return Ok(format_lsp_json("workspaceSymbol", &result));
        }
        if file_path.is_empty() {
            return Err("file_path 不能为空".to_string());
        }
        let resolved = resolve_under_workspace(&self.root, file_path)?;
        let language = requested_language
            .or_else(|| language_for_path(&resolved))
            .ok_or_else(|| {
                format!(
                    "不支持的文件类型: {}。可通过 language 参数指定已安装的 language server。",
                    resolved.display()
                )
            })?;
        let server = self.ensure_server(language).await?;
        server.did_open(&resolved).await?;
        let uri = path_uri(&resolved);
        let pos = json!({
            "line": line.saturating_sub(1),
            "character": character.saturating_sub(1)
        });
        let result = match operation {
            "goToDefinition" => {
                server
                    .request(
                        "textDocument/definition",
                        json!({ "textDocument": { "uri": uri }, "position": pos }),
                    )
                    .await?
            }
            "findReferences" => {
                server
                    .request(
                        "textDocument/references",
                        json!({
                            "textDocument": { "uri": uri },
                            "position": pos,
                            "context": { "includeDeclaration": true }
                        }),
                    )
                    .await?
            }
            "hover" => {
                server
                    .request(
                        "textDocument/hover",
                        json!({ "textDocument": { "uri": uri }, "position": pos }),
                    )
                    .await?
            }
            "documentSymbol" => {
                server
                    .request(
                        "textDocument/documentSymbol",
                        json!({ "textDocument": { "uri": uri } }),
                    )
                    .await?
            }
            "goToImplementation" => {
                server
                    .request(
                        "textDocument/implementation",
                        json!({ "textDocument": { "uri": uri }, "position": pos }),
                    )
                    .await?
            }
            "diagnostics" => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                return Ok(format_diagnostics(&server.diagnostics_for(&resolved).await));
            }
            other => return Err(format!("未知 LSP operation: {other}")),
        };
        Ok(format_lsp_json(operation, &result))
    }

    pub async fn diagnostics_for_paths(&self, paths: &[String]) -> String {
        if !self.enabled || paths.is_empty() {
            return String::new();
        }
        let mut collected = Vec::new();
        for path in paths {
            let Ok(resolved) = resolve_under_workspace(&self.root, path) else {
                continue;
            };
            let Some(language) = language_for_path(&resolved) else {
                continue;
            };
            let Ok(server) = self.ensure_server(language).await else {
                continue;
            };
            if server.did_open(&resolved).await.is_err() {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            collected.extend(server.diagnostics_for(&resolved).await);
        }
        if collected.is_empty() {
            String::new()
        } else {
            format!("\n\n[LSP 诊断]\n{}", format_diagnostics(&collected))
        }
    }

    async fn ensure_server(&self, language: &str) -> Result<Arc<LanguageServer>, String> {
        let mut state = self.inner.lock().await;
        if let Some(server) = state.servers.get(language) {
            return Ok(server.clone());
        }
        let spec = server_spec(language)
            .ok_or_else(|| format!("没有为 {language} 配置 language server"))?;
        let server = LanguageServer::start(&self.root, language, spec).await?;
        state.servers.insert(language.to_string(), server.clone());
        Ok(server)
    }
}

#[derive(Clone, Copy)]
struct ServerCommand {
    command: &'static str,
    args: &'static [&'static str],
}

struct ServerSpec {
    language_id: String,
    commands: Vec<ServerCommand>,
    workspace_data: bool,
}

fn server_spec(language: &str) -> Option<ServerSpec> {
    match language {
        "rust" => Some(spec("rust", &[command("rust-analyzer", &[])], false)),
        "typescript" | "javascript" => Some(spec(
            language,
            &[command("typescript-language-server", &["--stdio"])],
            false,
        )),
        "python" => Some(spec(
            "python",
            &[command("pyright-langserver", &["--stdio"])],
            false,
        )),
        "go" => Some(spec("go", &[command("gopls", &[])], false)),
        "cpp" => Some(spec("cpp", &[command("clangd", &[])], false)),
        "java" => Some(spec("java", &[command("jdtls", &[])], true)),
        "kotlin" => Some(spec(
            "kotlin",
            &[command("kotlin-language-server", &[])],
            false,
        )),
        "csharp" => Some(spec(
            "csharp",
            &[command("csharp-ls", &[]), command("OmniSharp", &["-lsp"])],
            false,
        )),
        "php" => Some(spec(
            "php",
            &[
                command("intelephense", &["--stdio"]),
                command("phpactor", &["language-server"]),
            ],
            false,
        )),
        "ruby" => Some(spec(
            "ruby",
            &[
                command("ruby-lsp", &["--stdio"]),
                command("solargraph", &["stdio"]),
            ],
            false,
        )),
        "swift" => Some(spec("swift", &[command("sourcekit-lsp", &[])], false)),
        "dart" => Some(spec(
            "dart",
            &[command("dart", &["language-server", "--protocol=lsp"])],
            false,
        )),
        "lua" => Some(spec("lua", &[command("lua-language-server", &[])], false)),
        "html" => Some(spec(
            "html",
            &[command("vscode-html-language-server", &["--stdio"])],
            false,
        )),
        "css" => Some(spec(
            "css",
            &[command("vscode-css-language-server", &["--stdio"])],
            false,
        )),
        "json" => Some(spec(
            "json",
            &[command("vscode-json-language-server", &["--stdio"])],
            false,
        )),
        "yaml" => Some(spec(
            "yaml",
            &[command("yaml-language-server", &["--stdio"])],
            false,
        )),
        "bash" => Some(spec(
            "shellscript",
            &[command("bash-language-server", &["start"])],
            false,
        )),
        "markdown" => Some(spec("markdown", &[command("marksman", &[])], false)),
        "sql" => Some(spec(
            "sql",
            &[command("sql-language-server", &["up", "--method", "stdio"])],
            false,
        )),
        "vue" => Some(spec(
            "vue",
            &[command("vue-language-server", &["--stdio"])],
            false,
        )),
        "svelte" => Some(spec(
            "svelte",
            &[command("svelteserver", &["--stdio"])],
            false,
        )),
        "docker" => Some(spec(
            "dockerfile",
            &[
                command("docker-langserver", &["--stdio"]),
                command("dockerfile-language-server-nodejs", &["start"]),
            ],
            false,
        )),
        "terraform" => Some(spec(
            "terraform",
            &[command("terraform-ls", &["serve"])],
            false,
        )),
        _ => None,
    }
}

const SUPPORTED_LANGUAGES: &[&str] = &[
    "rust",
    "typescript",
    "javascript",
    "python",
    "go",
    "cpp",
    "java",
    "kotlin",
    "csharp",
    "php",
    "ruby",
    "swift",
    "dart",
    "lua",
    "html",
    "css",
    "json",
    "yaml",
    "bash",
    "markdown",
    "sql",
    "vue",
    "svelte",
    "docker",
    "terraform",
];

fn language_label(language: &str) -> &'static str {
    match language {
        "rust" => "Rust",
        "typescript" => "TypeScript",
        "javascript" => "JavaScript",
        "python" => "Python",
        "go" => "Go",
        "cpp" => "C / C++",
        "java" => "Java",
        "kotlin" => "Kotlin",
        "csharp" => "C#",
        "php" => "PHP",
        "ruby" => "Ruby",
        "swift" => "Swift",
        "dart" => "Dart",
        "lua" => "Lua",
        "html" => "HTML",
        "css" => "CSS",
        "json" => "JSON",
        "yaml" => "YAML",
        "bash" => "Bash / Shell",
        "markdown" => "Markdown",
        "sql" => "SQL",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "docker" => "Dockerfile",
        "terraform" => "Terraform",
        _ => "Language",
    }
}

fn install_commands(language: &str) -> Vec<InstallCommand> {
    match language {
        "rust" => vec![InstallCommand {
            program: "rustup",
            args: &["component", "add", "rust-analyzer"],
            display: "rustup component add rust-analyzer",
        }],
        "typescript" | "javascript" => vec![InstallCommand {
            program: "npm",
            args: &[
                "install",
                "--global",
                "typescript",
                "typescript-language-server",
            ],
            display: "npm install --global typescript typescript-language-server",
        }],
        "python" => vec![
            InstallCommand {
                program: "python3",
                args: &["-m", "pip", "install", "--user", "pyright"],
                display: "python3 -m pip install --user pyright",
            },
            InstallCommand {
                program: "python",
                args: &["-m", "pip", "install", "--user", "pyright"],
                display: "python -m pip install --user pyright",
            },
        ],
        "go" => vec![InstallCommand {
            program: "go",
            args: &["install", "golang.org/x/tools/gopls@latest"],
            display: "go install golang.org/x/tools/gopls@latest",
        }],
        "cpp" => vec![InstallCommand {
            program: "brew",
            args: &["install", "llvm"],
            display: "brew install llvm",
        }],
        "java" => vec![InstallCommand {
            program: "brew",
            args: &["install", "jdtls"],
            display: "brew install jdtls",
        }],
        "kotlin" => vec![InstallCommand {
            program: "brew",
            args: &["install", "kotlin-language-server"],
            display: "brew install kotlin-language-server",
        }],
        "csharp" => vec![InstallCommand {
            program: "dotnet",
            args: &["tool", "install", "--global", "csharp-ls"],
            display: "dotnet tool install --global csharp-ls",
        }],
        "php" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "intelephense"],
            display: "npm install --global intelephense",
        }],
        "ruby" => vec![InstallCommand {
            program: "gem",
            args: &["install", "solargraph"],
            display: "gem install solargraph",
        }],
        "swift" => vec![InstallCommand {
            program: "xcode-select",
            args: &["--install"],
            display: "xcode-select --install",
        }],
        "dart" => vec![InstallCommand {
            program: "brew",
            args: &["install", "dart"],
            display: "brew install dart",
        }],
        "lua" => vec![InstallCommand {
            program: "brew",
            args: &["install", "lua-language-server"],
            display: "brew install lua-language-server",
        }],
        "html" | "css" | "json" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "vscode-langservers-extracted"],
            display: "npm install --global vscode-langservers-extracted",
        }],
        "yaml" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "yaml-language-server"],
            display: "npm install --global yaml-language-server",
        }],
        "bash" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "bash-language-server"],
            display: "npm install --global bash-language-server",
        }],
        "markdown" => vec![InstallCommand {
            program: "brew",
            args: &["install", "marksman"],
            display: "brew install marksman",
        }],
        "sql" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "sql-language-server"],
            display: "npm install --global sql-language-server",
        }],
        "vue" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "@vue/language-server"],
            display: "npm install --global @vue/language-server",
        }],
        "svelte" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "svelte-language-server"],
            display: "npm install --global svelte-language-server",
        }],
        "docker" => vec![InstallCommand {
            program: "npm",
            args: &["install", "--global", "dockerfile-language-server-nodejs"],
            display: "npm install --global dockerfile-language-server-nodejs",
        }],
        "terraform" => vec![InstallCommand {
            program: "brew",
            args: &["install", "hashicorp/tap/terraform-ls"],
            display: "brew install hashicorp/tap/terraform-ls",
        }],
        _ => Vec::new(),
    }
}

fn install_command_text(language: &str) -> Option<String> {
    install_commands(language)
        .first()
        .map(|command| command.display.to_string())
}

#[tauri::command]
pub async fn list_lsp_servers() -> Result<Vec<LspServerStatus>, String> {
    let mut servers = Vec::with_capacity(SUPPORTED_LANGUAGES.len());
    for language in SUPPORTED_LANGUAGES {
        let spec = server_spec(language).ok_or_else(|| format!("没有为 {language} 配置 LSP"))?;
        let installed_command = spec.commands.iter().find_map(|candidate| {
            resolve_program(candidate.command)
                .ok()
                .map(|_| candidate.command.to_string())
        });
        let commands = spec
            .commands
            .iter()
            .map(|candidate| candidate.command.to_string())
            .collect();
        let install_command = install_command_text(language);
        servers.push(LspServerStatus {
            id: (*language).to_string(),
            label: language_label(language).to_string(),
            commands,
            installed_command,
            installable: install_command.is_some(),
            install_command,
        });
    }
    Ok(servers)
}

#[tauri::command]
pub async fn install_lsp_server(language: String) -> Result<String, String> {
    let language = normalize_language(&language)?;
    if server_spec(language).is_none() {
        return Err(format!("没有为 {language} 配置 LSP"));
    }
    let commands = install_commands(language);
    if commands.is_empty() {
        let label = language_label(language);
        return Err(format!("{label} 没有可自动执行的安装方式"));
    }
    let mut missing = Vec::new();
    for install in commands {
        let program = match resolve_program(install.program) {
            Ok(program) => program,
            Err(_) => {
                missing.push(install.program);
                continue;
            }
        };
        let output = tokio::time::timeout(
            Duration::from_secs(600),
            tokio_command(&program).args(install.args).output(),
        )
        .await
        .map_err(|_| {
            let label = language_label(language);
            format!("安装 {label} 超时，请重试或手动执行：{}", install.display)
        })?
        .map_err(|error| format!("执行 {} 失败：{error}", install.display))?;
        if output.status.success() {
            let detected = server_spec(language).is_some_and(|spec| {
                spec.commands
                    .iter()
                    .any(|candidate| resolve_program(candidate.command).is_ok())
            });
            if !detected {
                return Err(format!(
                    "安装命令已完成，但仍未在 PATH 中找到 {}。请重启应用或手动检查：{}",
                    language_label(language),
                    install.display
                ));
            }
            return Ok(format!(
                "已执行：{}。请重新打开会话以使用新的 language server。",
                install.display
            ));
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!(
                "安装 {} 失败，请手动执行：{}",
                language_label(language),
                install.display
            )
        } else {
            format!("安装 {} 失败：{detail}", language_label(language))
        });
    }
    Err(format!(
        "找不到安装器（{}）。请安装对应工具后执行：{}",
        missing.join("、"),
        install_command_text(language).unwrap_or_default()
    ))
}

const fn command(command: &'static str, args: &'static [&'static str]) -> ServerCommand {
    ServerCommand { command, args }
}

fn spec(language_id: &str, commands: &[ServerCommand], workspace_data: bool) -> ServerSpec {
    ServerSpec {
        language_id: language_id.to_string(),
        commands: commands.to_vec(),
        workspace_data,
    }
}

pub fn language_for_path(path: &Path) -> Option<&'static str> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("dockerfile"))
    {
        return Some("docker");
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" => Some("cpp"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "cs" | "csx" => Some("csharp"),
        "php" => Some("php"),
        "rb" | "rake" | "gemspec" => Some("ruby"),
        "swift" => Some("swift"),
        "dart" => Some("dart"),
        "lua" => Some("lua"),
        "html" | "htm" => Some("html"),
        "css" | "scss" | "sass" | "less" => Some("css"),
        "json" | "jsonc" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "sh" | "bash" | "zsh" => Some("bash"),
        "md" | "markdown" => Some("markdown"),
        "sql" => Some("sql"),
        "vue" => Some("vue"),
        "svelte" => Some("svelte"),
        "tf" | "tfvars" => Some("terraform"),
        _ => None,
    }
}

fn normalize_language(language: &str) -> Result<&'static str, String> {
    let normalized = language.trim().to_ascii_lowercase();
    let canonical = match normalized.as_str() {
        "rs" | "rust" => "rust",
        "ts" | "tsx" | "typescript" => "typescript",
        "js" | "jsx" | "javascript" => "javascript",
        "py" | "python" => "python",
        "go" | "golang" => "go",
        "c" | "cpp" | "c++" | "cxx" => "cpp",
        "java" => "java",
        "kt" | "kts" | "kotlin" => "kotlin",
        "cs" | "csharp" | "c#" => "csharp",
        "php" => "php",
        "rb" | "ruby" => "ruby",
        "swift" => "swift",
        "dart" => "dart",
        "lua" => "lua",
        "html" => "html",
        "css" | "scss" | "sass" | "less" => "css",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "shell" | "shellscript" => "bash",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "vue" => "vue",
        "svelte" => "svelte",
        "docker" | "dockerfile" => "docker",
        "tf" | "tfvars" | "terraform" => "terraform",
        _ => {
            return Err(format!(
                "不支持的 language `{language}`。请使用文件扩展名对应的已安装 language server。"
            ));
        }
    };
    Ok(canonical)
}

fn infer_language_from_query(query: &str) -> Option<&'static str> {
    if query.contains("::") || query.contains("fn ") {
        Some("rust")
    } else {
        None
    }
}

fn select_workspace_language(
    requested: Option<&'static str>,
    file_language: Option<&'static str>,
    query: &str,
) -> Result<&'static str, String> {
    requested
        .or(file_language)
        .or_else(|| infer_language_from_query(query))
        .ok_or_else(|| {
            "workspaceSymbol 需要 language 或 file_path 来选择 language server".to_string()
        })
}

fn command_args(
    root: &Path,
    language: &str,
    spec: &ServerSpec,
    candidate: &ServerCommand,
) -> Vec<String> {
    let mut args = candidate
        .args
        .iter()
        .map(|item| (*item).to_string())
        .collect::<Vec<_>>();
    if spec.workspace_data {
        let mut hasher = DefaultHasher::new();
        root.to_string_lossy().hash(&mut hasher);
        let data_dir = std::env::temp_dir()
            .join("noxcode-lsp")
            .join(format!("{language}-{hash:016x}", hash = hasher.finish()));
        let _ = std::fs::create_dir_all(&data_dir);
        args.push("-data".to_string());
        args.push(data_dir.to_string_lossy().to_string());
    }
    args
}

impl LanguageServer {
    async fn start(root: &Path, language: &str, spec: ServerSpec) -> Result<Arc<Self>, String> {
        let mut errors = Vec::new();
        for candidate in &spec.commands {
            let program = match resolve_program(candidate.command) {
                Ok(program) => program,
                Err(_) => {
                    errors.push(format!("未找到 `{}`", candidate.command));
                    continue;
                }
            };
            let args = command_args(root, language, &spec, candidate);
            match Self::start_command(root, language, &spec, candidate, &program, &args).await {
                Ok(server) => return Ok(server),
                Err(error) => errors.push(error),
            }
        }
        let commands = spec
            .commands
            .iter()
            .map(|item| item.command)
            .collect::<Vec<_>>()
            .join("、");
        Err(format!(
            "未找到或无法启动 {language} language server。尝试命令：{commands}。请安装对应 language server 并确保命令在 PATH 中。详情：{}",
            errors.join("；")
        ))
    }

    async fn start_command(
        root: &Path,
        language: &str,
        spec: &ServerSpec,
        candidate: &ServerCommand,
        program: &Path,
        args: &[String],
    ) -> Result<Arc<Self>, String> {
        let mut cmd = tokio_command(program);
        cmd.args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("启动 {} 失败: {error}", candidate.command))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "language server stdin 不可用".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "language server stdout 不可用".to_string())?;
        let server = Arc::new(Self {
            language: language.to_string(),
            language_id: spec.language_id.to_string(),
            command: candidate.command.to_string(),
            next_id: AtomicI64::new(1),
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            opened: Mutex::new(HashMap::new()),
            _child: child,
        });
        let reader_server = server.clone();
        tauri::async_runtime::spawn(async move {
            read_loop(stdout, reader_server).await;
        });
        let root_uri = path_uri(root);
        let root_path = root.to_string_lossy().to_string();
        let root_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace");
        let init = server
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootPath": root_path,
                    "rootUri": root_uri.clone(),
                    "workspaceFolders": [{ "uri": root_uri, "name": root_name }],
                    "capabilities": {
                        "textDocument": {
                            "hover": { "contentFormat": ["markdown", "plaintext"] },
                            "publishDiagnostics": {}
                        },
                        "workspace": { "symbol": {} }
                    }
                }),
            )
            .await;
        if let Err(error) = init {
            return Err(format!("{} 初始化失败: {error}", candidate.command));
        }
        let _ = server.notify("initialized", json!({})).await;
        Ok(server)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        let response = tokio::time::timeout(Duration::from_secs(12), rx)
            .await
            .map_err(|_| format!("{} 请求超时: {method}", self.command))?
            .map_err(|_| format!("{} 已关闭: {method}", self.command))?;
        if let Some(error) = response.get("error") {
            return Err(format!("{} 请求 {method} 失败: {error}", self.command));
        }
        Ok(response)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn write(&self, body: &Value) -> Result<(), String> {
        let bytes = encode_rpc(body);
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| format!("写入 {} 失败: {error}", self.command))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("写入 {} 失败: {error}", self.command))
    }

    async fn did_open(&self, path: &Path) -> Result<(), String> {
        let uri = path_uri(path);
        let mut opened = self.opened.lock().await;
        if opened.contains_key(&uri) {
            return Ok(());
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        opened.insert(uri.clone(), 1);
        drop(opened);
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": self.language_id,
                    "version": 1,
                    "text": text
                }
            }),
        )
        .await
    }

    async fn diagnostics_for(&self, path: &Path) -> Vec<LspDiagnostic> {
        let uri = path_uri(path);
        self.diagnostics
            .lock()
            .await
            .get(&uri)
            .cloned()
            .unwrap_or_default()
    }
}

async fn read_loop(stdout: tokio::process::ChildStdout, server: Arc<LanguageServer>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_rpc(&mut reader).await {
            Ok(Some(value)) => handle_rpc(&server, value).await,
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

async fn handle_rpc(server: &LanguageServer, value: Value) {
    if let Some(id) = value.get("id").and_then(Value::as_i64) {
        if let Some(result) = value.get("result").cloned() {
            if let Some(tx) = server.pending.lock().await.remove(&id) {
                let _ = tx.send(result);
            }
            return;
        }
        if let Some(error) = value.get("error") {
            if let Some(tx) = server.pending.lock().await.remove(&id) {
                let _ = tx.send(json!({ "error": error }));
            }
        }
        return;
    }
    if value.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
        if let Some(params) = value.get("params") {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let items = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let parsed = items
                .iter()
                .filter_map(|item| parse_diagnostic(&uri, item))
                .collect();
            server.diagnostics.lock().await.insert(uri, parsed);
        }
    }
}

fn parse_diagnostic(uri: &str, item: &Value) -> Option<LspDiagnostic> {
    let message = item.get("message")?.as_str()?.to_string();
    let severity = match item.get("severity").and_then(Value::as_u64).unwrap_or(1) {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    };
    let line = item
        .pointer("/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    let character = item
        .pointer("/range/start/character")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    Some(LspDiagnostic {
        path: uri_to_path(uri),
        line,
        character,
        severity: severity.to_string(),
        message,
    })
}

pub fn encode_rpc(body: &Value) -> Vec<u8> {
    let json = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    let mut out = header.into_bytes();
    out.extend(json);
    out
}

async fn read_rpc<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Value>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|error| error.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    let mut buf = vec![0_u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&buf)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn path_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.starts_with("file:") {
        return raw.into_owned();
    }
    format!("file://{raw}")
}

fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

fn format_diagnostics(items: &[LspDiagnostic]) -> String {
    if items.is_empty() {
        return "没有诊断。".to_string();
    }
    items
        .iter()
        .take(40)
        .map(|item| {
            format!(
                "{}:{}:{} [{}] {}",
                item.path, item.line, item.character, item.severity, item.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_lsp_json(operation: &str, value: &Value) -> String {
    format!(
        "{operation}: {}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    )
}

pub fn mutation_paths(name: &str, arguments: &str) -> Vec<String> {
    let Ok(args) = serde_json::from_str::<Value>(arguments) else {
        return Vec::new();
    };
    match name {
        "Write" | "Edit" => args
            .get("file_path")
            .and_then(Value::as_str)
            .map(|path| vec![path.to_string()])
            .unwrap_or_default(),
        "ApplyPatch" => super::patch::parse_patch(
            &super::patch::extract_patch_text(arguments).unwrap_or_default(),
        )
        .map(|actions| {
            actions
                .into_iter()
                .map(|action| match action {
                    super::patch::PatchAction::Add { path, .. }
                    | super::patch::PatchAction::Delete { path }
                    | super::patch::PatchAction::Update { path, .. } => path,
                })
                .collect()
        })
        .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_mapping() {
        for (path, language) in [
            ("src/lib.rs", "rust"),
            ("app.tsx", "typescript"),
            ("main.py", "python"),
            ("Main.java", "java"),
            ("build.gradle.kts", "kotlin"),
            ("Program.cs", "csharp"),
            ("index.php", "php"),
            ("app.rb", "ruby"),
            ("main.swift", "swift"),
            ("lib.dart", "dart"),
            ("init.lua", "lua"),
            ("index.html", "html"),
            ("styles.scss", "css"),
            ("settings.json", "json"),
            ("config.yaml", "yaml"),
            ("script.sh", "bash"),
            ("README.md", "markdown"),
            ("query.sql", "sql"),
            ("App.vue", "vue"),
            ("Component.svelte", "svelte"),
            ("main.tf", "terraform"),
        ] {
            assert_eq!(language_for_path(Path::new(path)), Some(language), "{path}");
        }
        assert_eq!(language_for_path(Path::new("Dockerfile")), Some("docker"));
        assert_eq!(language_for_path(Path::new("notes.txt")), None);
    }

    #[test]
    fn server_specs_include_fallbacks_and_workspace_data() {
        let java = server_spec("java").expect("java server");
        assert_eq!(java.language_id, "java");
        assert!(java.workspace_data);
        assert_eq!(java.commands[0].command, "jdtls");

        let csharp = server_spec("csharp").expect("csharp server");
        assert_eq!(csharp.commands.len(), 2);
        assert_eq!(csharp.commands[1].args, &["-lsp"]);
    }

    #[test]
    fn command_args_isolates_java_workspaces() {
        let spec = server_spec("java").expect("java server");
        let args = command_args(
            Path::new("/tmp/noxcode-java-workspace"),
            "java",
            &spec,
            &spec.commands[0],
        );
        assert_eq!(args.first().map(String::as_str), Some("-data"));
        assert!(args
            .get(1)
            .is_some_and(|path| path.contains("noxcode-lsp/java-")));
    }

    #[test]
    fn normalizes_language_aliases() {
        assert_eq!(normalize_language("C#"), Ok("csharp"));
        assert_eq!(normalize_language("tsx"), Ok("typescript"));
        assert!(normalize_language("unknown").is_err());
    }

    #[test]
    fn workspace_symbol_language_precedence() {
        assert_eq!(
            select_workspace_language(Some("java"), Some("typescript"), ""),
            Ok("java")
        );
        assert_eq!(
            select_workspace_language(None, Some("python"), ""),
            Ok("python")
        );
        assert_eq!(
            select_workspace_language(None, None, "crate::module"),
            Ok("rust")
        );
        assert!(select_workspace_language(None, None, "Controller").is_err());
    }

    #[test]
    fn install_registry_has_commands_for_supported_servers() {
        for language in SUPPORTED_LANGUAGES {
            assert!(
                server_spec(language).is_some(),
                "missing spec for {language}"
            );
            assert!(
                install_command_text(language).is_some(),
                "missing install command for {language}"
            );
        }
        assert_eq!(
            install_command_text("java").as_deref(),
            Some("brew install jdtls")
        );
    }

    #[test]
    fn encode_rpc_has_content_length() {
        let bytes = encode_rpc(&json!({"jsonrpc":"2.0","id":1}));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("Content-Length:"));
        assert!(text.contains("\r\n\r\n{"));
    }

    #[test]
    fn mutation_paths_read_write_and_patch() {
        assert_eq!(
            mutation_paths("Write", r#"{"file_path":"a.rs","content":"x"}"#),
            vec!["a.rs"]
        );
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-a\n+b\n*** End Patch\n";
        let args = serde_json::json!({ "patch": patch }).to_string();
        assert!(mutation_paths("ApplyPatch", &args).contains(&"src/a.rs".to_string()));
    }
}
