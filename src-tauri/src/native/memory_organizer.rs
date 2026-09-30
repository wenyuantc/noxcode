//! 工具驱动的记忆整理（dream）。
//!
//! 整理 Agent 只拿到 `Memory` 工具，在记忆目录的暂存副本上多轮检索、合并、改写；
//! 受轮次、token 和墙钟时间限制，可取消。结束后校验副本，再与原目录原子交换：
//! 原目录在整理期间被改过（例如会话又抽取了新记忆）就放弃本次结果。任何失败都
//! 只丢弃副本，不改动原目录和索引。模型不支持工具调用时回退到一次性 JSON 整理，
//! 同样先写副本再交换。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::native::agent::r#loop::AgentRunner;
use crate::native::memory::{
    self, dir_lock, is_entry_file_name, list_entries, rebuild_index, MemoryEntry,
};
use crate::native::model::call_log::OPERATION_MEMORY_DREAM;
use crate::native::model::client::ChatRequest;
use crate::native::model::types::Message;
use crate::native::model::ModelClient;
use crate::native::tools::cancel::CancelFlag;
use crate::native::tools::memory_tool::MemoryBinding;
use crate::native::tools::LocalWorkspace;

const ORGANIZER_SYSTEM: &str = "你是记忆整理助手，只能用 Memory 工具维护一个记忆目录。目标：\
1. 合并重复或高度重叠的条目：保留信息更完整的一条，把另一条独有的信息并入后删除它。\
2. 纠正过期或互相矛盾的事实：以更新时间较新、内容更具体的一条为准，改写或删除旧的。\
3. 删除一次性任务细节、临时状态和可以直接从仓库读到的内容。\
4. 让名称简短准确，描述是一句话。type 只能是 user / feedback / project / reference。\
不要编造记忆里没有的事实。先 list，需要时 read 或 search，再逐条 write（更新已有条目时传 file）或 delete。\
完成后用一两句话说明做了哪些改动。";

const ORGANIZER_TASK: &str = "请整理当前记忆目录。";
const LEGACY_MAX_ENTRIES: usize = 60;

/// 一次整理的上限。
#[derive(Debug, Clone, Copy)]
pub struct OrganizeLimits {
    pub max_turns: u32,
    pub token_budget: u64,
    pub timeout: Duration,
}

impl Default for OrganizeLimits {
    fn default() -> Self {
        Self {
            max_turns: 12,
            token_budget: 60_000,
            timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizeReport {
    pub before: usize,
    pub after: usize,
    /// `agent`：多轮工具整理；`json`：回退的一次性整理；`none`：没有记忆。
    pub mode: &'static str,
    pub model_calls: u32,
    pub tokens: u64,
}

impl OrganizeReport {
    pub fn summary(&self) -> String {
        match self.mode {
            "none" => "没有记忆可整理".to_string(),
            "json" => format!(
                "记忆整理完成（一次性整理）：{} → {} 条",
                self.before, self.after
            ),
            _ => format!(
                "记忆整理完成：{} → {} 条，模型调用 {} 次，约 {} token",
                self.before, self.after, self.model_calls, self.tokens
            ),
        }
    }
}

/// 暂存副本与备份的位置：与记忆目录同级。
fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "memory".to_string());
    dir.with_file_name(format!("{name}.{suffix}"))
}

/// 记忆条目的内容快照，用于判断整理期间原目录是否被改动。
fn fingerprint(dir: &Path) -> Vec<(String, String)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, String)> = read
        .flatten()
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            if !is_entry_file_name(&name) {
                return None;
            }
            let text = std::fs::read_to_string(item.path()).ok()?;
            Some((name, text))
        })
        .collect();
    files.sort();
    files
}

/// 把事实文件复制到干净的暂存目录（不复制符号链接）。
fn prepare_staging(dir: &Path) -> Result<PathBuf, String> {
    let staging = sibling(dir, "organize");
    if staging.exists() {
        // 上次整理中断留下的副本，直接丢弃。
        std::fs::remove_dir_all(&staging)
            .map_err(|error| format!("清理旧的整理副本失败: {error}"))?;
    }
    std::fs::create_dir_all(&staging).map_err(|error| format!("创建整理副本失败: {error}"))?;
    for entry in list_entries(dir) {
        std::fs::copy(dir.join(&entry.file_name), staging.join(&entry.file_name))
            .map_err(|error| format!("复制记忆到整理副本失败: {error}"))?;
    }
    Ok(staging)
}

/// 校验副本并与原目录交换。副本在任何失败路径上由调用方删除。
fn commit_staging(
    dir: &Path,
    staging: &Path,
    snapshot: &[(String, String)],
    before: usize,
) -> Result<usize, String> {
    let files = std::fs::read_dir(staging)
        .map_err(|error| format!("读取整理副本失败: {error}"))?
        .flatten()
        .filter(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            name.ends_with(".md") && name != memory::MEMORY_INDEX_FILE
        })
        .count();
    let entries = list_entries(staging);
    if entries.len() != files {
        return Err("整理结果里有无法解析的记忆文件，已放弃".to_string());
    }
    if before > 0 && entries.is_empty() {
        return Err("整理结果为空，已放弃并保留原记忆".to_string());
    }
    rebuild_index(staging)?;
    let lock = dir_lock(dir);
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    if fingerprint(dir) != snapshot {
        return Err("整理期间记忆被修改，本次结果未应用".to_string());
    }
    memory::record_dream(dir, staging);
    let backup = sibling(dir, "bak");
    if backup.exists() {
        std::fs::remove_dir_all(&backup).map_err(|error| format!("清理旧备份失败: {error}"))?;
    }
    std::fs::rename(dir, &backup).map_err(|error| format!("备份原记忆失败: {error}"))?;
    if let Err(error) = std::fs::rename(staging, dir) {
        // 交换失败时把原目录放回去。
        let _ = std::fs::rename(&backup, dir);
        return Err(format!("应用整理结果失败: {error}"));
    }
    Ok(entries.len())
}

struct AgentStats {
    model_calls: u32,
    tokens: u64,
}

async fn run_agent(
    client: &ModelClient,
    model: &str,
    staging: &Path,
    cancel: &CancelFlag,
    limits: &OrganizeLimits,
) -> Result<AgentStats, String> {
    // 工作区根也设为副本，白名单只有 Memory：整理 Agent 碰不到任何工作区文件。
    let mut runner = AgentRunner::new(LocalWorkspace::new(staging.to_path_buf()));
    runner.ctx.cancel = cancel.clone();
    runner.ctx.memory = Some(MemoryBinding {
        dir: staging.to_path_buf(),
        writable: true,
    });
    runner.set_allowed_tools(&["Memory"]);
    runner.max_turns = limits.max_turns;
    runner.set_rollout_budget_limit(limits.token_budget);
    runner.messages = vec![Message::system(ORGANIZER_SYSTEM)];
    let result = runner
        .run_restricted(client, ORGANIZER_TASK, model, Some(2_048))
        .await;
    let stats = AgentStats {
        model_calls: runner.turns_used(),
        tokens: runner.budget_snapshot().spent,
    };
    result.map(|_| stats)
}

/// 回退路径：一次性让模型输出整理后的全部条目，写进副本。
async fn legacy_into(
    client: &ModelClient,
    model: &str,
    entries: &[MemoryEntry],
    staging: &Path,
) -> Result<(), String> {
    let dump: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "### {} [{}]\n描述：{}\n{}",
                entry.name,
                entry.kind,
                entry.description,
                entry.body.chars().take(1_200).collect::<String>()
            )
        })
        .collect();
    let prompt = vec![
        Message::system(
            "你整理编程助手的长期记忆：合并重复项、删除互相矛盾里过时的一方、去掉一次性细节、让描述更精确。保持四类 type 不变。只输出 JSON 数组 [{\"name\",\"type\",\"description\",\"body\"}]，条目数不超过 60；输出即为整理后的全部记忆，未包含的条目会被删除。",
        ),
        Message::user(format!("当前记忆：\n\n{}", dump.join("\n\n"))),
    ];
    let message = client
        .chat(ChatRequest {
            messages: &prompt,
            tools: &[],
            model,
            effort: None,
            max_output_tokens: Some(8_192),
            thinking_enabled: false,
        })
        .await?
        .complete_message()?;
    let parsed = memory::parse_entries_json(&message.content);
    if parsed.is_empty() {
        return Err("模型没有返回可用的整理结果，记忆保持不变".to_string());
    }
    for entry in list_entries(staging) {
        std::fs::remove_file(staging.join(&entry.file_name))
            .map_err(|error| format!("清理整理副本失败: {error}"))?;
    }
    for (name, kind, description, body) in parsed.into_iter().take(LEGACY_MAX_ENTRIES) {
        memory::save_entry(staging, &name, &kind, &description, &body)?;
    }
    Ok(())
}

/// 整理记忆目录。成功时原目录被整理结果替换，旧内容留在同级 `.bak`；
/// 失败、超时或取消时原目录保持不变。
pub async fn organize(
    client: &ModelClient,
    main_model: &str,
    lite_model: Option<&str>,
    dir: &Path,
    cancel: &CancelFlag,
    limits: OrganizeLimits,
) -> Result<OrganizeReport, String> {
    let entries = list_entries(dir);
    let before = entries.len();
    if before == 0 {
        return Ok(OrganizeReport {
            before,
            after: 0,
            mode: "none",
            model_calls: 0,
            tokens: 0,
        });
    }
    let snapshot = fingerprint(dir);
    let (model, role) = memory::pick_model(main_model, lite_model);
    let client = memory::lite_client(client, OPERATION_MEMORY_DREAM, role);
    let staging = prepare_staging(dir)?;
    let result = organize_staging(
        &client, model, dir, &staging, &snapshot, &entries, cancel, limits,
    )
    .await;
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn organize_staging(
    client: &ModelClient,
    model: &str,
    dir: &Path,
    staging: &Path,
    snapshot: &[(String, String)],
    entries: &[MemoryEntry],
    cancel: &CancelFlag,
    limits: OrganizeLimits,
) -> Result<OrganizeReport, String> {
    let before = entries.len();
    let agent = tokio::time::timeout(
        limits.timeout,
        run_agent(client, model, staging, cancel, &limits),
    )
    .await;
    let (mode, stats) = match agent {
        Err(_) => {
            cancel.cancel();
            return Err(format!(
                "记忆整理超过 {} 秒已中止，原记忆保持不变",
                limits.timeout.as_secs()
            ));
        }
        Ok(_) if cancel.is_cancelled() => return Err("记忆整理已取消，原记忆保持不变".to_string()),
        Ok(Ok(stats)) => ("agent", stats),
        Ok(Err(error)) => {
            // 模型不支持工具调用等情况：丢弃副本上的部分改动，回退到一次性整理。
            eprintln!("[native] 工具整理失败，改用一次性整理: {error}");
            for entry in list_entries(staging) {
                let _ = std::fs::remove_file(staging.join(&entry.file_name));
            }
            for entry in entries {
                std::fs::copy(dir.join(&entry.file_name), staging.join(&entry.file_name))
                    .map_err(|error| format!("重建整理副本失败: {error}"))?;
            }
            tokio::time::timeout(limits.timeout, legacy_into(client, model, entries, staging))
                .await
                .map_err(|_| "记忆整理超时，原记忆保持不变".to_string())??;
            (
                "json",
                AgentStats {
                    model_calls: 1,
                    tokens: 0,
                },
            )
        }
    };
    let after = commit_staging(dir, staging, snapshot, before)?;
    Ok(OrganizeReport {
        before,
        after,
        mode,
        model_calls: stats.model_calls,
        tokens: stats.tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 模拟的模型回复：正常 JSON、HTTP 错误，或一直不回。
    enum Reply {
        Json(Value),
        Status(u16, &'static str),
        Hang,
    }

    type Hook = Arc<dyn Fn(usize) + Send + Sync>;

    fn tool_calls(calls: &[(&str, Value)]) -> Reply {
        let calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(index, (name, args))| {
                json!({"id": format!("c{index}"), "type": "function",
                    "function": {"name": name, "arguments": args.to_string()}})
            })
            .collect();
        Reply::Json(json!({"choices": [{"finish_reason": "tool_calls",
            "message": {"role": "assistant", "content": null, "tool_calls": calls}}]}))
    }

    fn text(content: &str) -> Reply {
        Reply::Json(json!({"choices": [{"finish_reason": "stop",
            "message": {"role": "assistant", "content": content}}]}))
    }

    async fn serve(replies: Vec<Reply>, hook: Option<Hook>) -> ModelClient {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (index, reply) in replies.into_iter().enumerate() {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 8192];
                loop {
                    let count = stream.read(&mut buffer).await.unwrap_or(0);
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                if let Some(hook) = &hook {
                    hook(index);
                }
                let (status, body) = match reply {
                    Reply::Json(value) => (200, value.to_string()),
                    Reply::Status(status, body) => (status, body.to_string()),
                    Reply::Hang => {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        return;
                    }
                };
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        ModelClient::new(crate::native::model::client::ModelClientConfig {
            protocol: crate::native::protocol::PROTOCOL_OPENAI.to_string(),
            base_url: format!("http://{address}"),
            api_key: "test".to_string(),
            extra_headers: HashMap::new(),
            retry: crate::native::model::RetryConfig::none(),
            timeout: Duration::from_secs(5),
            network: crate::app::network_settings::NetworkSettings::default(),
            responses_continuation: crate::native::model::ResponsesContinuationMode::Auto,
        })
        .unwrap()
    }

    fn memory_dir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "noxcode-organize-{}",
            crate::native::artifacts::unique_suffix()
        ));
        let dir = base.join("project-key");
        memory::save_entry(&dir, "构建命令", "project", "怎么构建", "用 make build").unwrap();
        memory::save_entry(
            &dir,
            "构建方式",
            "project",
            "构建",
            "用 make build，需要先 npm install",
        )
        .unwrap();
        memory::save_entry(&dir, "默认分支", "project", "主分支", "主分支是 master").unwrap();
        dir
    }

    fn file_of(dir: &Path, name: &str) -> String {
        list_entries(dir)
            .into_iter()
            .find(|entry| entry.name == name)
            .expect(name)
            .file_name
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[tokio::test]
    async fn agent_merges_duplicates_and_corrects_stale_facts() {
        let dir = memory_dir();
        let keep = file_of(&dir, "构建命令");
        let duplicate = file_of(&dir, "构建方式");
        let branch = file_of(&dir, "默认分支");
        let client = serve(
            vec![
                tool_calls(&[("Memory", json!({"action": "list"}))]),
                tool_calls(&[
                    (
                        "Memory",
                        json!({"action": "write", "file": keep, "name": "构建命令",
                        "type": "project", "description": "怎么构建",
                        "body": "先 npm install，再 make build"}),
                    ),
                    ("Memory", json!({"action": "delete", "file": duplicate})),
                    (
                        "Memory",
                        json!({"action": "write", "file": branch, "name": "默认分支",
                        "type": "project", "description": "主分支", "body": "主分支已改为 main"}),
                    ),
                ]),
                text("合并了两条构建记忆，更正了默认分支。"),
            ],
            None,
        )
        .await;
        let report = organize(
            &client,
            "m",
            None,
            &dir,
            &CancelFlag::new(),
            OrganizeLimits::default(),
        )
        .await
        .expect("organize");
        assert_eq!((report.before, report.after, report.mode), (3, 2, "agent"));
        assert_eq!(report.model_calls, 3);
        let merged = memory::read_entry(&dir, &keep).unwrap();
        assert!(merged.body.contains("npm install") && merged.body.contains("make build"));
        assert!(memory::read_entry(&dir, &duplicate).is_none());
        assert_eq!(
            memory::read_entry(&dir, &branch).unwrap().body,
            "主分支已改为 main"
        );
        let index = memory::load_index(&dir);
        assert!(index.contains("构建命令") && !index.contains("构建方式"));
        // 旧内容留在同级备份，副本已清理。
        assert!(sibling(&dir, "bak").join(&duplicate).exists());
        assert!(!sibling(&dir, "organize").exists());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn timeout_and_out_of_scope_writes_leave_the_original_untouched() {
        let dir = memory_dir();
        let original = fingerprint(&dir);
        let client = serve(vec![Reply::Hang], None).await;
        let limits = OrganizeLimits {
            timeout: Duration::from_millis(300),
            ..OrganizeLimits::default()
        };
        let error = organize(&client, "m", None, &dir, &CancelFlag::new(), limits)
            .await
            .unwrap_err();
        assert!(error.contains("中止"), "{error}");
        assert_eq!(fingerprint(&dir), original);
        assert!(!sibling(&dir, "organize").exists());

        // 越界写入和白名单外的工具都被拒绝，工作区之外没有新文件。
        let client = serve(
            vec![
                tool_calls(&[
                    (
                        "Memory",
                        json!({"action": "write", "file": "../escape.md",
                        "name": "x", "body": "y"}),
                    ),
                    (
                        "Write",
                        json!({"file_path": "../escape.md", "content": "y"}),
                    ),
                    ("Bash", json!({"command": "touch ../escape.md"})),
                ]),
                text("完成"),
            ],
            None,
        )
        .await;
        let report = organize(
            &client,
            "m",
            None,
            &dir,
            &CancelFlag::new(),
            OrganizeLimits::default(),
        )
        .await
        .expect("organize");
        assert_eq!(report.after, 3);
        assert!(!dir.parent().unwrap().join("escape.md").exists());
        assert!(!dir.join("escape.md").exists());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn concurrent_changes_abort_the_swap() {
        let dir = memory_dir();
        let writer = dir.clone();
        let hook: Hook = Arc::new(move |index| {
            if index == 1 {
                // 整理期间会话又抽取了一条新记忆。
                memory::save_entry(&writer, "新偏好", "user", "d", "喜欢简短回答").unwrap();
            }
        });
        let delete_all = file_of(&dir, "默认分支");
        let client = serve(
            vec![
                tool_calls(&[("Memory", json!({"action": "list"}))]),
                tool_calls(&[("Memory", json!({"action": "delete", "file": delete_all}))]),
                text("完成"),
            ],
            Some(hook),
        )
        .await;
        let error = organize(
            &client,
            "m",
            None,
            &dir,
            &CancelFlag::new(),
            OrganizeLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("整理期间记忆被修改"), "{error}");
        let names: Vec<String> = list_entries(&dir)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(names.contains(&"新偏好".to_string()));
        assert!(
            names.contains(&"默认分支".to_string()),
            "未应用的删除不能生效"
        );
        assert!(!sibling(&dir, "organize").exists());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn tool_rejection_falls_back_to_single_json_pass_via_staging() {
        let dir = memory_dir();
        let client = serve(
            vec![
                Reply::Status(400, r#"{"error":{"message":"tools are not supported"}}"#),
                text(&json!([
                    {"name": "构建命令", "type": "project", "description": "怎么构建",
                     "body": "先 npm install，再 make build"},
                    {"name": "默认分支", "type": "project", "description": "主分支", "body": "main"}
                ]).to_string()),
            ],
            None,
        )
        .await;
        let report = organize(
            &client,
            "m",
            None,
            &dir,
            &CancelFlag::new(),
            OrganizeLimits::default(),
        )
        .await
        .expect("fallback");
        assert_eq!((report.after, report.mode), (2, "json"));
        assert_eq!(list_entries(&dir).len(), 2);

        // 回退结果为空时放弃，原目录不变。
        let before = fingerprint(&dir);
        let client = serve(
            vec![
                Reply::Status(400, r#"{"error":{"message":"no tools"}}"#),
                text("[]"),
            ],
            None,
        )
        .await;
        assert!(organize(
            &client,
            "m",
            None,
            &dir,
            &CancelFlag::new(),
            OrganizeLimits::default()
        )
        .await
        .is_err());
        assert_eq!(fingerprint(&dir), before);
        cleanup(&dir);
    }
}
