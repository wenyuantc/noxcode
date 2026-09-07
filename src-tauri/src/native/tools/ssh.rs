use std::time::Duration;

use tauri::AppHandle;

use crate::app::ssh::exec::{
    execute_ssh_command, execute_ssh_command_with_input, spawn_ssh_command, SshCommandOutput,
    SshCommandStream, SshStreamEvent,
};
use crate::app::ssh::shell::shell_escape_single_quoted;
use crate::db::models::SshConfigRecord;

use super::cancel::CancelFlag;
use super::file_access::AuthorizedPath;
use super::local::{
    bash_timeout, BoundedOutput, CommandStatus, BASH_DEFAULT_TIMEOUT, BASH_OUTPUT_HARD_LIMIT,
};
use super::paths::{resolve_posix_path, resolve_under_workspace_posix};

#[derive(Clone)]
pub struct SshToolRuntime {
    pub app: AppHandle,
    pub config: SshConfigRecord,
    pub root: String,
    pub authorized_paths: Vec<AuthorizedPath>,
}

impl SshToolRuntime {
    pub fn resolve(&self, path: &str) -> Result<String, String> {
        self.resolve_access(path, false)
    }

    pub fn resolve_for_write(&self, path: &str) -> Result<String, String> {
        self.resolve_access(path, true)
    }

    fn resolve_access(&self, path: &str, write: bool) -> Result<String, String> {
        resolve_ssh_access(&self.root, &self.authorized_paths, path, write)
    }

    pub async fn validate_path(&self, path: &str) -> Result<bool, String> {
        let resolved = resolve_posix_path(&self.root, path)?;
        let command = format!(
            "{}test -d {}",
            ssh_path_guard("/", &resolved)?,
            shell_escape_single_quoted(&resolved)
        );
        let output = execute_ssh_command(&self.app, &self.config, &command, true).await?;
        match output.exit_code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(format!("检查远程路径失败: {}", output.stderr_lossy())),
        }
    }

    pub async fn exists(&self, path: &str) -> Result<bool, String> {
        let command = ssh_exists_command("/", &self.resolve(path)?)?;
        let output = execute_ssh_command(&self.app, &self.config, &command, true).await?;
        match output.exit_code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(format!("检查远程文件失败: {}", output.stderr_lossy())),
        }
    }

    pub async fn read(&self, path: &str) -> Result<String, String> {
        let command = ssh_read_command("/", &self.resolve(path)?)?;
        let output = execute_ssh_command(&self.app, &self.config, &command, true).await?;
        if output.success() {
            Ok(output.stdout_lossy())
        } else {
            stdout_or_err(output)
        }
    }

    pub async fn write(&self, path: &str, content: &str) -> Result<String, String> {
        self.write_checked(path, content, false).await
    }

    pub async fn write_checked(
        &self,
        path: &str,
        content: &str,
        create_only: bool,
    ) -> Result<String, String> {
        let resolved = self.resolve_access(path, true)?;
        let command = if create_only {
            ssh_write_command_checked("/", &resolved, true)?
        } else {
            ssh_write_command("/", &resolved)?
        };
        let output = execute_ssh_command_with_input(
            &self.app,
            &self.config,
            &command,
            content.as_bytes().to_vec(),
            true,
        )
        .await?;
        if !output.success() {
            return Err(output.stderr_lossy().trim().to_string());
        }
        Ok(format!("Wrote {} bytes to {path}", content.len()))
    }

    pub async fn glob(&self, path: Option<&str>) -> Result<String, String> {
        let command = ssh_glob_access_command(&self.root, &self.authorized_paths, path)?;
        stdout_or_err(execute_ssh_command(&self.app, &self.config, &command, true).await?)
    }

    pub async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<String, String> {
        let resolved = self.resolve(path.unwrap_or("."))?;
        let command = ssh_grep_command("/", pattern, Some(&resolved))?;
        stdout_or_err(execute_ssh_command(&self.app, &self.config, &command, true).await?)
    }

    pub async fn bash(&self, command: &str) -> Result<String, String> {
        self.bash_controlled(command, None, BASH_DEFAULT_TIMEOUT, &CancelFlag::new())
            .await
    }

    pub async fn bash_controlled(
        &self,
        command: &str,
        timeout_ms: Option<i64>,
        default_timeout: Duration,
        cancel: &CancelFlag,
    ) -> Result<String, String> {
        let status = self
            .bash_status_controlled(command, bash_timeout(timeout_ms, default_timeout), cancel)
            .await?;
        if status.timed_out {
            return Err("Bash 超时".to_string());
        }
        if status.exit_code != 0 {
            return Err(if status.output.trim().is_empty() {
                format!("command failed: {}", status.exit_code)
            } else {
                status.output
            });
        }
        if status.output.trim().is_empty() {
            Ok("(no output)".to_string())
        } else {
            Ok(status.output)
        }
    }

    pub async fn bash_status_controlled(
        &self,
        command: &str,
        timeout: Duration,
        cancel: &CancelFlag,
    ) -> Result<CommandStatus, String> {
        if cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        let remote = ssh_bash_command(&self.root, command)?;
        let deadline = tokio::time::Instant::now() + timeout;
        let stream = tokio::select! {
            biased;
            _ = wait_cancel(cancel) => return Err("已取消".to_string()),
            _ = tokio::time::sleep_until(deadline) => return Ok(timed_out_status()),
            result = spawn_ssh_command(&self.app, &self.config, &remote, true) => result?,
        };
        collect_bash_output(stream, deadline, cancel).await
    }

    pub async fn delete(&self, path: &str) -> Result<String, String> {
        let command = ssh_delete_command("/", &self.resolve_access(path, true)?)?;
        stdout_or_err(execute_ssh_command(&self.app, &self.config, &command, true).await?)?;
        Ok(format!("Deleted {path}"))
    }
}

fn resolve_ssh_access(
    root: &str,
    grants: &[AuthorizedPath],
    path: &str,
    write: bool,
) -> Result<String, String> {
    let resolved = resolve_posix_path(root, path)?;
    if grants.iter().any(|grant| grant.permits(&resolved, write)) {
        Ok(resolved)
    } else {
        resolve_under_workspace_posix(root, path)
    }
}

fn ssh_glob_access_command(
    root: &str,
    grants: &[AuthorizedPath],
    path: Option<&str>,
) -> Result<String, String> {
    let resolved = resolve_ssh_access(root, grants, path.unwrap_or("."), false)?;
    Ok(format!(
        "{}{}",
        ssh_path_guard("/", &resolved)?,
        ssh_glob_command(&resolved)?
    ))
}

async fn collect_bash_output(
    mut stream: SshCommandStream,
    deadline: tokio::time::Instant,
    cancel: &CancelFlag,
) -> Result<CommandStatus, String> {
    let mut output = BoundedOutput::new(BASH_OUTPUT_HARD_LIMIT);
    let mut exit_code = -1;
    loop {
        let event = tokio::select! {
            biased;
            _ = wait_cancel(cancel) => {
                stream.terminate().await;
                return Err("已取消".to_string());
            }
            _ = tokio::time::sleep_until(deadline) => {
                stream.terminate().await;
                return Ok(timed_out_status());
            }
            event = stream.next() => event,
        };
        match event {
            Some(SshStreamEvent::Stdout(bytes) | SshStreamEvent::Stderr(bytes)) => {
                output.push(&bytes)
            }
            Some(SshStreamEvent::Exit(code)) => exit_code = code,
            Some(SshStreamEvent::Closed) | None => break,
        }
    }
    Ok(CommandStatus {
        exit_code,
        output: output.into_text(),
        timed_out: false,
    })
}

fn timed_out_status() -> CommandStatus {
    CommandStatus {
        exit_code: -1,
        output: "Bash 超时".to_string(),
        timed_out: true,
    }
}

async fn wait_cancel(cancel: &CancelFlag) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn stdout_or_err(output: SshCommandOutput) -> Result<String, String> {
    let stdout = output.stdout_lossy();
    if output.success() {
        if stdout.trim().is_empty() {
            Ok("(no output)".to_string())
        } else {
            Ok(stdout)
        }
    } else {
        let stderr = output.stderr_lossy();
        Err(if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        })
    }
}

pub fn ssh_read_command(root: &str, path: &str) -> Result<String, String> {
    let resolved = resolve_under_workspace_posix(root, path)?;
    Ok(format!(
        "{}cat {}",
        ssh_path_guard(root, &resolved)?,
        shell_escape_single_quoted(&resolved)
    ))
}

pub fn ssh_write_command(root: &str, path: &str) -> Result<String, String> {
    ssh_write_command_checked(root, path, false)
}

fn ssh_exists_command(root: &str, path: &str) -> Result<String, String> {
    let resolved = resolve_under_workspace_posix(root, path)?;
    Ok(format!(
        "{}test -e {}",
        ssh_path_guard(root, &resolved)?,
        shell_escape_single_quoted(&resolved)
    ))
}

fn ssh_write_command_checked(root: &str, path: &str, create_only: bool) -> Result<String, String> {
    let resolved = resolve_under_workspace_posix(root, path)?;
    let parent = resolved
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .filter(|dir| !dir.is_empty())
        .unwrap_or("/");
    Ok(format!(
        "{}mkdir -p {} && {}cat > {}",
        ssh_path_guard(root, &resolved)?,
        shell_escape_single_quoted(parent),
        if create_only { "(set -C; " } else { "" },
        shell_escape_single_quoted(&resolved)
    ) + if create_only { ")" } else { "" })
}

fn ssh_path_guard(root: &str, resolved: &str) -> Result<String, String> {
    let root = resolve_under_workspace_posix(root, ".")?;
    let relative = resolved
        .strip_prefix(&root)
        .unwrap_or(resolved)
        .trim_start_matches('/');
    let mut current = root.trim_end_matches('/').to_string();
    let mut checks = Vec::new();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(part);
        checks.push(format!("[ -L {} ]", shell_escape_single_quoted(&current)));
    }
    if checks.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!(
            "if {}; then printf '%s\\n' '路径包含符号链接，拒绝远程文件访问' >&2; exit 73; fi; ",
            checks.join(" || ")
        ))
    }
}

pub fn ssh_glob_command(root: &str) -> Result<String, String> {
    let root = resolve_under_workspace_posix(root, ".")?;
    Ok(format!(
        "cd {} && find . -type f | sed 's|^./||' | head -n 500",
        shell_escape_single_quoted(&root)
    ))
}

pub fn ssh_grep_command(root: &str, pattern: &str, path: Option<&str>) -> Result<String, String> {
    let target = match path {
        Some(value) => resolve_under_workspace_posix(root, value)?,
        None => resolve_under_workspace_posix(root, ".")?,
    };
    Ok(format!(
        "{}cd {} && (if command -v rg >/dev/null 2>&1; then rg -n --no-heading -e {} -- {}; else find {} -type f -exec grep -n -H -e {} -- {{}} +; fi)",
        ssh_path_guard(root, &target)?,
        shell_escape_single_quoted(root),
        shell_escape_single_quoted(pattern),
        shell_escape_single_quoted(&target),
        shell_escape_single_quoted(&target),
        shell_escape_single_quoted(pattern)
    ))
}

pub fn ssh_delete_command(root: &str, path: &str) -> Result<String, String> {
    let resolved = resolve_under_workspace_posix(root, path)?;
    Ok(format!(
        "{}rm -f {}",
        ssh_path_guard(root, &resolved)?,
        shell_escape_single_quoted(&resolved)
    ))
}

pub fn ssh_bash_command(root: &str, command: &str) -> Result<String, String> {
    let root = resolve_under_workspace_posix(root, ".")?;
    if command.trim().is_empty() {
        return Err("command 不能为空".to_string());
    }
    Ok(format!(
        "cd {} && bash -lc {}",
        shell_escape_single_quoted(&root),
        shell_escape_single_quoted(command)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approved_ssh_glob_uses_requested_directory_and_grants_do_not_leak() {
        use super::super::contract::PermissionCapability;
        use super::super::file_access::PathAccessScope;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("inside.txt"), "inside").unwrap();
        std::fs::write(outside.path().join("outside.txt"), "outside").unwrap();
        let root = std::fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let external = std::fs::canonicalize(outside.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let grants = vec![AuthorizedPath {
            path: external.clone(),
            scope: PathAccessScope::Subtree,
            capability: PermissionCapability::Read,
        }];
        assert!(ssh_glob_access_command(&root, &[], Some(&external)).is_err());
        assert!(
            resolve_ssh_access(&root, &grants, &format!("{external}/outside.txt"), true).is_err()
        );
        let command = ssh_glob_access_command(&root, &grants, Some(&external)).unwrap();
        let output = crate::process_spawn::tokio_command("sh")
            .args(["-c", &command])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        let listing = String::from_utf8_lossy(&output.stdout);
        assert!(listing.contains("outside.txt"));
        assert!(!listing.contains("inside.txt"));
        let default = ssh_glob_access_command(&root, &grants, None).unwrap();
        let output = crate::process_spawn::tokio_command("sh")
            .args(["-c", &default])
            .output()
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("inside.txt"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("outside.txt"));
    }

    #[tokio::test]
    async fn remote_write_commands_create_new_files_without_clobbering_existing_files() {
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy();
        for (content, success) in [("first", true), ("second", false)] {
            let command = ssh_write_command_checked(&root, "nested/new.txt", true).unwrap();
            let mut child = crate::process_spawn::tokio_command("sh")
                .arg("-c")
                .arg(command)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = child.stdin.take().unwrap();
            let _ = input.write_all(content.as_bytes()).await;
            drop(input);
            let output = child.wait_with_output().await.unwrap();
            assert_eq!(output.status.success(), success);
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/new.txt")).unwrap(),
            "first"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remote_file_commands_reject_symlink_paths() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        let root = dir.path().to_string_lossy();
        for command in [
            ssh_read_command(&root, "escape/secret").unwrap(),
            ssh_write_command(&root, "escape/new/nested.txt").unwrap(),
            ssh_exists_command(&root, "escape/secret").unwrap(),
            ssh_delete_command(&root, "escape/secret").unwrap(),
            ssh_grep_command(&root, "secret", Some("escape")).unwrap(),
        ] {
            let result = crate::process_spawn::tokio_command("sh")
                .arg("-c")
                .arg(command)
                .output()
                .await
                .unwrap();
            assert_eq!(result.status.code(), Some(73));
        }
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret")).unwrap(),
            "secret"
        );
        assert!(!outside.path().join("new").exists());
    }

    #[tokio::test]
    async fn native_ssh_stream_honors_deadline_and_cancellation() {
        use crate::app::ssh::client::{AuthMaterial, ConnectParams};
        use crate::app::ssh::known_hosts::{HostTrustBroker, KnownHostsPolicy};
        use crate::app::ssh::test_server::{TestServerOpts, TestSshServer};
        use crate::app::ssh::SshPool;
        use std::sync::Arc;
        let server = TestSshServer::start(TestServerOpts::default()).await;
        let dir = tempfile::tempdir().unwrap();
        let params = ConnectParams {
            ssh_config_id: "native-controlled".to_string(),
            name: "native-controlled".to_string(),
            host: "127.0.0.1".to_string(),
            port: server.port,
            username: "tester".to_string(),
            auth: AuthMaterial::Password("secret".to_string()),
            policy: KnownHostsPolicy::Off,
            known_hosts_path: dir.path().join("known_hosts"),
            algorithms: None,
        };
        let pool = SshPool::new(
            Arc::new(HostTrustBroker::new(Duration::from_secs(5))),
            Duration::from_secs(600),
        );
        let stream = pool.spawn(&params, "hang").await.unwrap();
        let started = tokio::time::Instant::now();
        let status = collect_bash_output(
            stream,
            started + Duration::from_millis(50),
            &CancelFlag::new(),
        )
        .await
        .unwrap();
        assert!(status.timed_out);
        assert!(started.elapsed() < Duration::from_secs(1));
        let stream = pool.spawn(&params, "hang").await.unwrap();
        let cancel = CancelFlag::new();
        let other = cancel.clone();
        let (result, ()) = tokio::join!(
            collect_bash_output(
                stream,
                tokio::time::Instant::now() + Duration::from_secs(10),
                &cancel
            ),
            async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                other.cancel();
            },
        );
        assert_eq!(result.unwrap_err(), "已取消");
        let stream = pool.spawn(&params, "echo still-connected").await.unwrap();
        let status = collect_bash_output(
            stream,
            tokio::time::Instant::now() + Duration::from_secs(1),
            &CancelFlag::new(),
        )
        .await
        .unwrap();
        assert_eq!(status.exit_code, 0);
        assert_eq!(status.output, "still-connected");
    }

    #[test]
    fn commands_stay_inside_workspace() {
        let read = ssh_read_command("/proj", "src/a.rs").unwrap();
        assert!(read.contains("/proj/src/a.rs"));
        assert!(ssh_read_command("/proj", "../etc/passwd").is_err());
        let write = ssh_write_command("/proj", "src/a.rs").unwrap();
        assert!(write.contains("mkdir -p"));
        assert!(write.contains("cat >"));
        let bash = ssh_bash_command("/proj", "ls").unwrap();
        assert!(bash.contains("cd '/proj'"));
        assert!(bash.contains("bash -lc"));
        let delete = ssh_delete_command("/proj", "src/a.rs").unwrap();
        assert!(delete.contains("rm -f"));
        assert!(delete.contains("/proj/src/a.rs"));
        assert!(ssh_delete_command("/proj", "../etc/passwd").is_err());
        let glob = ssh_glob_command("/proj").unwrap();
        assert!(glob.contains("find . -type f"));
        let grep = ssh_grep_command("/proj", "TODO", Some("src")).unwrap();
        assert!(grep.contains("TODO"));
    }

    #[tokio::test]
    async fn ssh_tools_read_write_glob_grep_bash_and_reject_escape() {
        use std::sync::Arc;
        use std::time::Duration;

        use crate::app::ssh::client::{AuthMaterial, ConnectParams};
        use crate::app::ssh::exec::ExecOptions;
        use crate::app::ssh::known_hosts::{HostTrustBroker, KnownHostsPolicy};
        use crate::app::ssh::test_server::{TestServerOpts, TestSshServer};
        use crate::app::ssh::SshPool;

        let dir = tempfile::tempdir().expect("workspace");
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::write(dir.path().join("README.md"), "hello from readme\n").expect("readme");
        std::fs::write(dir.path().join("src/a.rs"), "fn main() { /* TODO */ }\n").expect("src");
        let root = dir.path().to_string_lossy().into_owned();

        assert!(ssh_read_command(&root, "../../etc/passwd").is_err());
        assert!(ssh_write_command(&root, "../../etc/passwd").is_err());
        assert!(ssh_delete_command(&root, "../../etc/passwd").is_err());
        assert!(ssh_grep_command(&root, "x", Some("../../etc/passwd")).is_err());

        let server = TestSshServer::start(TestServerOpts {
            real_shell: true,
            ..TestServerOpts::default()
        })
        .await;
        let known_hosts_dir = tempfile::tempdir().expect("known_hosts dir");
        let params = ConnectParams {
            ssh_config_id: "native-ssh-tools".to_string(),
            name: "native-ssh-tools".to_string(),
            host: "127.0.0.1".to_string(),
            port: server.port,
            username: "tester".to_string(),
            auth: AuthMaterial::Password("secret".to_string()),
            policy: KnownHostsPolicy::Off,
            known_hosts_path: known_hosts_dir.path().join("known_hosts"),
            algorithms: None,
        };
        let pool = SshPool::new(
            Arc::new(HostTrustBroker::new(Duration::from_secs(5))),
            Duration::from_secs(600),
        );

        let read = pool
            .exec(
                &params,
                &ssh_read_command(&root, "README.md").expect("read cmd"),
                ExecOptions::default(),
            )
            .await
            .expect("read");
        assert!(read.success());
        assert!(read.stdout_lossy().contains("hello from readme"));

        // 测试服务器的 real_shell 不转发 channel stdin，写文件走 bash。
        let write = pool
            .exec(
                &params,
                &ssh_bash_command(&root, "printf 'from ssh write\\n' > notes.txt").expect("write"),
                ExecOptions::default(),
            )
            .await
            .expect("write");
        assert!(write.success());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes.txt")).expect("notes"),
            "from ssh write\n"
        );

        let glob = pool
            .exec(
                &params,
                &ssh_glob_command(&root).expect("glob cmd"),
                ExecOptions::default(),
            )
            .await
            .expect("glob");
        assert!(glob.success());
        let glob_text = glob.stdout_lossy();
        assert!(glob_text.contains("README.md"));
        assert!(glob_text.contains("notes.txt"));

        let grep = pool
            .exec(
                &params,
                &ssh_grep_command(&root, "TODO", Some("src")).expect("grep cmd"),
                ExecOptions::default(),
            )
            .await
            .expect("grep");
        assert!(grep.success());
        assert!(grep.stdout_lossy().contains("TODO"));

        let bash = pool
            .exec(
                &params,
                &ssh_bash_command(&root, "pwd").expect("bash cmd"),
                ExecOptions::default(),
            )
            .await
            .expect("bash");
        assert!(bash.success());
        assert!(bash
            .stdout_lossy()
            .contains(dir.path().to_str().expect("utf8")));
    }
}
