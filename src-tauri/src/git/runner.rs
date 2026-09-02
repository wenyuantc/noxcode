use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::app::ssh::client::ConnectParams;
use crate::app::ssh::exec::ExecOptions;
use crate::app::ssh::shell::{remote_shell_bootstrap, shell_escape_single_quoted};
use crate::app::ssh::SshPool;
use crate::process_spawn;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const INDEX_WRITING_SUBCOMMANDS: &[&str] = &[
    "add",
    "rm",
    "mv",
    "update-index",
    "read-tree",
    "reset",
    "checkout",
    "switch",
    "stash",
    "commit",
    "apply",
    "am",
    "cherry-pick",
    "merge",
    "rebase",
    "revert",
];

static REPO_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, thiserror::Error)]
pub(crate) enum GitError {
    #[error("BUG: {0}")]
    Bug(String),
    #[error("当前目录不是 git 仓库: {0}")]
    NotARepo(String),
    #[error("git 命令失败（exit {exit_code}）: {stderr}")]
    CommandFailed {
        args: String,
        exit_code: i32,
        stderr: String,
    },
    #[error("git 命令超时")]
    Timeout,
    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),
    #[error("SSH 执行失败: {0}")]
    Ssh(String),
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Blocked(String),
    #[error("远端 git 版本过低（当前 {found}，最低要求 {required}）")]
    VersionTooOld { found: String, required: String },
}

impl From<GitError> for String {
    fn from(value: GitError) -> Self {
        value.to_string()
    }
}

#[derive(Clone)]
pub(crate) enum GitTarget {
    Local(PathBuf),
    Ssh {
        pool: SshPool,
        params: ConnectParams,
        repo_path: String,
    },
}

impl GitTarget {
    pub(crate) fn working_dir(&self) -> &str {
        match self {
            GitTarget::Local(path) => path.to_str().unwrap_or(""),
            GitTarget::Ssh { repo_path, .. } => repo_path,
        }
    }
}

pub(crate) struct UserIndexToken {
    _private: (),
}

impl UserIndexToken {
    pub(super) const fn new() -> Self {
        Self { _private: () }
    }
}

#[derive(Clone)]
pub(crate) struct ScratchIndex {
    inner: Arc<ScratchState>,
}

struct ScratchState {
    location: ScratchLocation,
    cleaned: AtomicBool,
}

enum ScratchLocation {
    Local(PathBuf),
    Remote {
        path: String,
        pool: SshPool,
        params: ConnectParams,
    },
}

impl ScratchIndex {
    pub(crate) fn path(&self) -> String {
        match &self.inner.location {
            ScratchLocation::Local(path) => path.to_string_lossy().into_owned(),
            ScratchLocation::Remote { path, .. } => path.clone(),
        }
    }

    pub(crate) async fn from_user_index_copy(target: &GitTarget) -> Result<Self, GitError> {
        let git_dir = match git(
            target,
            &["rev-parse", "--absolute-git-dir"],
            &IndexMode::ReadOnly,
        )
        .await
        {
            Ok(output) if output.success() => output.stdout_lossy().trim().to_string(),
            _ => match target {
                GitTarget::Local(path) => path.join(".git").to_string_lossy().into_owned(),
                GitTarget::Ssh { repo_path, .. } => format!("{repo_path}/.git"),
            },
        };

        match target {
            GitTarget::Local(_) => {
                let src = Path::new(&git_dir).join("index");
                let dest = std::env::temp_dir().join(format!("noxcode-index-{}", Uuid::new_v4()));
                if src.is_file() {
                    std::fs::copy(&src, &dest)?;
                } else {
                    std::fs::write(&dest, [])?;
                }
                Ok(Self {
                    inner: Arc::new(ScratchState {
                        location: ScratchLocation::Local(dest),
                        cleaned: AtomicBool::new(false),
                    }),
                })
            }
            GitTarget::Ssh { pool, params, .. } => {
                let tmp_name = Uuid::new_v4().to_string();
                let script = format!(
                    "DIR=\"$HOME/.noxcode/tmp-index\"; mkdir -p \"$DIR\"; find \"$DIR\" -type f -mmin +60 -delete 2>/dev/null || true; TMP=\"$DIR/{tmp_name}\"; if [ -f {git_dir}/index ]; then cp -- {git_dir}/index \"$TMP\"; else : > \"$TMP\"; fi; printf '%s' \"$TMP\"",
                    git_dir = shell_escape_single_quoted(&git_dir),
                );
                let output = ssh_exec(pool, params, &script, GitRunOptions::default()).await?;
                if !output.success() {
                    return Err(output.to_error(&["scratch-index-copy"]));
                }
                let path = output.stdout_lossy().trim().to_string();
                if path.is_empty() {
                    return Err(GitError::Parse("远端临时索引路径为空".to_string()));
                }
                Ok(Self {
                    inner: Arc::new(ScratchState {
                        location: ScratchLocation::Remote {
                            path,
                            pool: pool.clone(),
                            params: params.clone(),
                        },
                        cleaned: AtomicBool::new(false),
                    }),
                })
            }
        }
    }

    pub(crate) async fn cleanup(&self) -> Result<(), GitError> {
        if self.inner.cleaned.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        match &self.inner.location {
            ScratchLocation::Local(path) => {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
                Ok(())
            }
            ScratchLocation::Remote { path, pool, params } => {
                let script = format!("rm -f -- {}", shell_escape_single_quoted(path));
                let output = ssh_exec(pool, params, &script, GitRunOptions::default()).await?;
                if output.success() {
                    Ok(())
                } else {
                    Err(output.to_error(&["rm", "-f", path]))
                }
            }
        }
    }
}

impl Drop for ScratchState {
    fn drop(&mut self) {
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }
        match &self.location {
            ScratchLocation::Local(path) => {
                let _ = std::fs::remove_file(path);
            }
            ScratchLocation::Remote { path, pool, params } => {
                let path = path.clone();
                let pool = pool.clone();
                let params = params.clone();
                tauri::async_runtime::spawn(async move {
                    eprintln!("warn: ScratchIndex Drop 补发远端清理: {path}");
                    let script = format!("rm -f -- {}", shell_escape_single_quoted(&path));
                    let _ = ssh_exec(&pool, &params, &script, GitRunOptions::default()).await;
                });
            }
        }
    }
}

pub(crate) enum IndexMode {
    ReadOnly,
    UserIndex(UserIndexToken),
    Scratch(ScratchIndex),
}

impl IndexMode {
    pub(super) fn user() -> Self {
        IndexMode::UserIndex(UserIndexToken::new())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

impl GitOutput {
    pub(crate) fn success(&self) -> bool {
        self.exit_code == 0
    }

    pub(crate) fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub(crate) fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    pub(crate) fn require_success(&self, args: &[&str]) -> Result<&Self, GitError> {
        if self.success() {
            Ok(self)
        } else {
            Err(self.to_error(args))
        }
    }

    pub(crate) fn command_error(&self, args: &[&str]) -> GitError {
        self.to_error(args)
    }

    fn to_error(&self, args: &[&str]) -> GitError {
        let stderr = self.stderr_lossy();
        let stdout = self.stdout_lossy();
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        GitError::CommandFailed {
            args: args.join(" "),
            exit_code: self.exit_code,
            stderr: detail,
        }
    }
}

#[derive(Clone)]
pub(crate) struct GitRunOptions {
    pub timeout: Option<Duration>,
    pub stdin: Option<Vec<u8>>,
    pub extra_env: Vec<(String, String)>,
}

impl Default for GitRunOptions {
    fn default() -> Self {
        Self {
            timeout: Some(DEFAULT_TIMEOUT),
            stdin: None,
            extra_env: Vec::new(),
        }
    }
}

fn git_program() -> PathBuf {
    static GIT_PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    GIT_PROGRAM
        .get_or_init(|| {
            for candidate in [
                "/opt/homebrew/bin/git",
                "/usr/local/bin/git",
                "/usr/bin/git",
            ] {
                if Path::new(candidate).is_file() {
                    return PathBuf::from(candidate);
                }
            }
            PathBuf::from("git")
        })
        .clone()
}

pub(crate) fn git_version_output() -> io::Result<Output> {
    process_spawn::std_command("git").arg("--version").output()
}

pub(crate) async fn git(
    target: &GitTarget,
    args: &[&str],
    mode: &IndexMode,
) -> Result<GitOutput, GitError> {
    git_with(target, args, mode, GitRunOptions::default()).await
}

pub(crate) async fn git_with(
    target: &GitTarget,
    args: &[&str],
    mode: &IndexMode,
    options: GitRunOptions,
) -> Result<GitOutput, GitError> {
    guard(args, mode)?;
    run_git(target, args, mode, options, false).await
}

async fn run_git(
    target: &GitTarget,
    args: &[&str],
    mode: &IndexMode,
    options: GitRunOptions,
    skip_guard: bool,
) -> Result<GitOutput, GitError> {
    if !skip_guard {
        guard(args, mode)?;
    }
    match target {
        GitTarget::Local(path) => run_local(path, args, mode, options).await,
        GitTarget::Ssh {
            pool,
            params,
            repo_path,
        } => {
            let script = build_ssh_git_script(repo_path, args, mode, &options.extra_env);
            ssh_exec(pool, params, &script, options).await
        }
    }
}

#[cfg(test)]
pub(crate) async fn fixture_git(target: &GitTarget, args: &[&str]) -> Result<GitOutput, GitError> {
    let output = run_git(
        target,
        args,
        &IndexMode::user(),
        GitRunOptions::default(),
        true,
    )
    .await?;
    output.require_success(args)?;
    Ok(output)
}

pub(crate) async fn with_repo_lock<F, Fut, T>(target: &GitTarget, f: F) -> Result<T, GitError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, GitError>>,
{
    let key = lock_key(target).await;
    let mutex = {
        let mut map = REPO_LOCKS.lock().expect("repo locks");
        map.entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = mutex.lock().await;
    f().await
}

async fn lock_key(target: &GitTarget) -> String {
    let dir = match git(
        target,
        &["rev-parse", "--absolute-git-dir"],
        &IndexMode::ReadOnly,
    )
    .await
    {
        Ok(output) if output.success() => output.stdout_lossy().trim().to_string(),
        _ => target.working_dir().to_string(),
    };
    match target {
        GitTarget::Local(_) => dir,
        GitTarget::Ssh { params, .. } => format!("ssh:{}:{dir}", params.ssh_config_id),
    }
}

pub(crate) fn first_non_flag<'a>(args: &'a [&'a str]) -> Option<&'a str> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if matches!(
            arg,
            "-c" | "-C" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(arg);
    }
    None
}

pub(crate) fn writes_index(args: &[&str]) -> bool {
    let Some(sub) = first_non_flag(args) else {
        return false;
    };
    INDEX_WRITING_SUBCOMMANDS.contains(&sub) || (sub == "restore" && args.contains(&"--staged"))
}

fn guard(args: &[&str], mode: &IndexMode) -> Result<(), GitError> {
    match (writes_index(args), mode) {
        (true, IndexMode::ReadOnly) => Err(GitError::Bug(format!(
            "写 index 的命令用了 ReadOnly 模式: {}",
            args.join(" ")
        ))),
        _ => Ok(()),
    }
}

pub(crate) fn build_ssh_git_script(
    repo_path: &str,
    args: &[&str],
    mode: &IndexMode,
    extra_env: &[(String, String)],
) -> String {
    let mut assigns = vec!["GIT_TERMINAL_PROMPT=0".to_string(), "LC_ALL=C".to_string()];
    if let IndexMode::Scratch(index) = mode {
        assigns.push(format!(
            "GIT_INDEX_FILE={}",
            shell_escape_single_quoted(&index.path())
        ));
    }
    for (key, value) in extra_env {
        assigns.push(format!("{key}={}", shell_escape_single_quoted(value)));
    }
    let mut argv = vec!["git".to_string()];
    if matches!(mode, IndexMode::ReadOnly) {
        argv.push("--no-optional-locks".to_string());
    }
    argv.extend(args.iter().map(|arg| shell_escape_single_quoted(arg)));
    format!(
        "cd {} && {} {}",
        shell_escape_single_quoted(repo_path),
        assigns.join(" "),
        argv.join(" ")
    )
}

pub(crate) fn wrap_ssh_script(script: &str) -> String {
    format!(
        "sh -c {}",
        shell_escape_single_quoted(&format!("{}{}", remote_shell_bootstrap(), script))
    )
}

async fn ssh_exec(
    pool: &SshPool,
    params: &ConnectParams,
    script: &str,
    options: GitRunOptions,
) -> Result<GitOutput, GitError> {
    let command = wrap_ssh_script(script);
    let output = pool
        .exec(
            params,
            &command,
            ExecOptions {
                stdin: options.stdin,
                timeout: options.timeout.or(Some(DEFAULT_TIMEOUT)),
            },
        )
        .await
        .map_err(|error| GitError::Ssh(error.to_string()))?;
    Ok(GitOutput {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.exit_code.unwrap_or(1),
    })
}

async fn run_local(
    repo: &Path,
    args: &[&str],
    mode: &IndexMode,
    options: GitRunOptions,
) -> Result<GitOutput, GitError> {
    let mut command = process_spawn::tokio_command(git_program());
    command.current_dir(repo);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("LC_ALL", "C");
    command.kill_on_drop(true);
    command.stdin(if options.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if matches!(mode, IndexMode::ReadOnly) {
        command.arg("--no-optional-locks");
    }
    if let IndexMode::Scratch(index) = mode {
        command.env("GIT_INDEX_FILE", index.path());
    }
    for (key, value) in &options.extra_env {
        command.env(key, value);
    }
    command.args(args);

    let mut child = command.spawn()?;
    if let Some(bytes) = options.stdin {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&bytes).await?;
        }
    }

    let timeout = options.timeout.unwrap_or(DEFAULT_TIMEOUT);
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(GitOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code().unwrap_or(1),
        }),
        Ok(Err(error)) => Err(error.into()),
        Err(_) => Err(GitError::Timeout),
    }
}

pub(crate) async fn remove_worktree_path(target: &GitTarget, rel: &str) -> Result<(), GitError> {
    assert_safe_rel_path(rel)?;
    match target {
        GitTarget::Local(dir) => {
            let path = dir.join(rel);
            if path.is_symlink() || path.is_file() {
                std::fs::remove_file(path)?;
            } else if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            }
            Ok(())
        }
        GitTarget::Ssh {
            pool,
            params,
            repo_path,
        } => {
            let script = format!(
                "cd {} && rm -f -- {}",
                shell_escape_single_quoted(repo_path),
                shell_escape_single_quoted(rel)
            );
            let output = ssh_exec(pool, params, &script, GitRunOptions::default()).await?;
            output.require_success(&["rm", "-f", rel])?;
            Ok(())
        }
    }
}

pub(crate) fn assert_safe_rel_path(path: &str) -> Result<(), GitError> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(GitError::Bug(format!("非法路径: {path}")));
    }
    Ok(())
}

pub(crate) fn split_nul_strings(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_guard_rejects_add_and_staged_restore() {
        let err = guard(&["add", "-A"], &IndexMode::ReadOnly).unwrap_err();
        assert!(matches!(err, GitError::Bug(_)));
        let err = guard(&["restore", "--staged", "--", "a"], &IndexMode::ReadOnly).unwrap_err();
        assert!(matches!(err, GitError::Bug(_)));
        assert!(guard(&["restore", "--worktree", "--", "a"], &IndexMode::ReadOnly).is_ok());
        assert!(guard(&["status"], &IndexMode::ReadOnly).is_ok());
        assert!(writes_index(&["-c", "foo.bar=1", "commit", "-m", "x"]));
        assert!(!writes_index(&["--no-optional-locks", "status"]));
    }

    #[test]
    fn ssh_script_escapes_repo_and_injects_readonly_flag() {
        let script = build_ssh_git_script(
            "/tmp/it's repo",
            &["status", "--porcelain=v2"],
            &IndexMode::ReadOnly,
            &[],
        );
        assert!(script.contains("cd '/tmp/it'\"'\"'s repo'"));
        assert!(script.contains("GIT_TERMINAL_PROMPT=0"));
        assert!(script.contains("LC_ALL=C"));
        assert!(script.contains("--no-optional-locks"));
        assert!(script.contains("status"));
        assert!(!script.contains("GIT_INDEX_FILE"));
    }

    #[test]
    fn wrap_ssh_script_uses_non_login_shell() {
        let wrapped = wrap_ssh_script("true");
        assert!(wrapped.starts_with("sh -c "));
        assert!(!wrapped.contains("sh -lc"));
    }

    #[test]
    fn only_runner_spawns_git() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        collect_git_spawns(&root, &mut offenders);
        assert!(
            offenders.is_empty(),
            "git 只能由 runner.rs spawn:\n{}",
            offenders.join("\n")
        );
    }

    fn collect_git_spawns(dir: &Path, offenders: &mut Vec<String>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_git_spawns(&path, offenders);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("runner.rs")
                && path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    == Some("git")
            {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (index, line) in source.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                if trimmed.contains("Command::new(\"git\")")
                    || trimmed.contains("std_command(\"git\")")
                    || trimmed.contains("tokio_command(\"git\")")
                {
                    offenders.push(format!("{}:{}:{trimmed}", path.display(), index + 1));
                }
            }
        }
    }
}
