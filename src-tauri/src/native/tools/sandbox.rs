//! 本地 Bash 的操作系统级沙箱。
//!
//! Linux 优先 `bwrap`（只读根 + 可写工作区 / 额外写根 / /tmp）。
//! macOS 使用 `sandbox-exec` seatbelt。Windows 与 SSH 不套沙箱。
//! 开启但找不到实现时回退为普通执行，并在结果里注明。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::native::tools::command_path::resolve_program;
use crate::process_spawn::tokio_command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxKind {
    None,
    Bubblewrap,
    Seatbelt,
}

#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy {
    pub enabled: bool,
    pub extra_write_roots: Vec<PathBuf>,
}

impl SandboxPolicy {
    pub fn disabled() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxApply {
    pub kind: SandboxKind,
    pub note: Option<String>,
}

pub fn detect_sandbox_kind() -> SandboxKind {
    if cfg!(target_os = "linux") && resolve_program("bwrap").is_ok() {
        return SandboxKind::Bubblewrap;
    }
    if cfg!(target_os = "macos") && resolve_program("sandbox-exec").is_ok() {
        return SandboxKind::Seatbelt;
    }
    SandboxKind::None
}

pub fn sandbox_status_label(enabled: bool) -> String {
    if !enabled {
        return "关闭".to_string();
    }
    match detect_sandbox_kind() {
        SandboxKind::Bubblewrap => "bubblewrap".to_string(),
        SandboxKind::Seatbelt => "seatbelt".to_string(),
        SandboxKind::None => "已开启但当前系统不可用".to_string(),
    }
}

/// 构造可写路径列表：工作区 + /tmp + 额外写根。
pub fn writable_paths(workspace: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = vec![workspace.to_path_buf(), PathBuf::from("/tmp")];
    if cfg!(target_os = "macos") {
        paths.push(PathBuf::from("/var/folders"));
        paths.push(PathBuf::from("/dev"));
    }
    for extra in extra {
        if extra.as_os_str().is_empty() {
            continue;
        }
        if !paths.iter().any(|item| item == extra) {
            paths.push(extra.clone());
        }
    }
    paths
}

pub fn seatbelt_profile(workspace: &Path, extra: &[PathBuf]) -> String {
    let mut allows = String::from(
        "(version 1)\n\
(deny default)\n\
(allow process-exec)\n\
(allow process-fork)\n\
(allow signal)\n\
(allow sysctl-read)\n\
(allow mach-lookup)\n\
(allow system-socket)\n\
(allow ipc-posix*)\n\
(allow network*)\n\
(allow file-read*)\n\
(allow file-write-data (literal \"/dev/null\"))\n\
(allow file-ioctl (literal \"/dev/null\"))\n",
    );
    for path in writable_paths(workspace, extra) {
        let escaped = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        allows.push_str(&format!("(allow file-write* (subpath \"{escaped}\"))\n"));
    }
    allows
}

pub fn apply_sandbox(
    policy: &SandboxPolicy,
    workspace: &Path,
    cmd: &mut tokio::process::Command,
) -> SandboxApply {
    if !policy.enabled {
        return SandboxApply {
            kind: SandboxKind::None,
            note: None,
        };
    }
    match detect_sandbox_kind() {
        SandboxKind::Bubblewrap => match apply_bwrap(workspace, &policy.extra_write_roots, cmd) {
            Ok(()) => SandboxApply {
                kind: SandboxKind::Bubblewrap,
                note: Some("已在 bubblewrap 沙箱中执行".to_string()),
            },
            Err(error) => SandboxApply {
                kind: SandboxKind::None,
                note: Some(format!("沙箱启动失败，已回退为普通 Bash：{error}")),
            },
        },
        SandboxKind::Seatbelt => match apply_seatbelt(workspace, &policy.extra_write_roots, cmd) {
            Ok(()) => SandboxApply {
                kind: SandboxKind::Seatbelt,
                note: Some("已在 seatbelt 沙箱中执行".to_string()),
            },
            Err(error) => SandboxApply {
                kind: SandboxKind::None,
                note: Some(format!("沙箱启动失败，已回退为普通 Bash：{error}")),
            },
        },
        SandboxKind::None => SandboxApply {
            kind: SandboxKind::None,
            note: Some("当前系统没有 bwrap / sandbox-exec，已回退为普通 Bash".to_string()),
        },
    }
}

fn apply_bwrap(
    workspace: &Path,
    extra: &[PathBuf],
    cmd: &mut tokio::process::Command,
) -> Result<(), String> {
    let bwrap = resolve_program("bwrap")?;
    let inner = take_program_and_args(cmd)?;
    let mut wrapped = tokio_command(&bwrap);
    wrapped
        .arg("--die-with-parent")
        .arg("--ro-bind")
        .arg("/")
        .arg("/")
        .arg("--dev")
        .arg("/dev")
        .arg("--proc")
        .arg("/proc")
        .arg("--tmpfs")
        .arg("/tmp");
    for path in writable_paths(workspace, extra) {
        if path == Path::new("/tmp") || path == Path::new("/dev") {
            continue;
        }
        if path.exists() {
            wrapped.arg("--bind").arg(&path).arg(&path);
        }
    }
    wrapped.arg("--chdir").arg(workspace).arg("--");
    wrapped.arg(inner.program);
    wrapped.args(inner.args);
    copy_stdios(cmd, &mut wrapped);
    *cmd = wrapped;
    Ok(())
}

fn apply_seatbelt(
    workspace: &Path,
    extra: &[PathBuf],
    cmd: &mut tokio::process::Command,
) -> Result<(), String> {
    let sandbox_exec = resolve_program("sandbox-exec")?;
    let profile = seatbelt_profile(workspace, extra);
    let inner = take_program_and_args(cmd)?;
    let mut wrapped = tokio_command(&sandbox_exec);
    wrapped.arg("-p").arg(profile).arg(&inner.program);
    wrapped.args(inner.args);
    copy_stdios(cmd, &mut wrapped);
    *cmd = wrapped;
    Ok(())
}

struct InnerCommand {
    program: PathBuf,
    args: Vec<std::ffi::OsString>,
}

fn take_program_and_args(cmd: &tokio::process::Command) -> Result<InnerCommand, String> {
    let std_cmd: &std::process::Command = cmd.as_std();
    let program = std_cmd.get_program();
    if program.is_empty() {
        return Err("Bash 命令为空".to_string());
    }
    Ok(InnerCommand {
        program: PathBuf::from(program),
        args: std_cmd.get_args().map(|arg| arg.to_os_string()).collect(),
    })
}

fn copy_stdios(from: &tokio::process::Command, to: &mut tokio::process::Command) {
    let std_from = from.as_std();
    to.current_dir(std_from.get_current_dir().unwrap_or(Path::new(".")));
    to.stdout(Stdio::piped());
    to.stderr(Stdio::piped());
    to.kill_on_drop(true);
    #[cfg(unix)]
    {
        to.process_group(0);
    }
    for (key, value) in std_from.get_envs() {
        if let Some(value) = value {
            to.env(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_paths_include_workspace_and_tmp() {
        let extra = [PathBuf::from("/tmp/nox-memory")];
        let paths = writable_paths(Path::new("/repo"), &extra);
        assert!(paths.contains(&PathBuf::from("/repo")));
        assert!(paths.contains(&PathBuf::from("/tmp")));
        assert!(paths.contains(&PathBuf::from("/tmp/nox-memory")));
    }

    #[test]
    fn seatbelt_profile_allows_workspace_writes() {
        let profile = seatbelt_profile(Path::new("/Users/me/proj"), &[]);
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("subpath \"/Users/me/proj\""));
        assert!(profile.contains("subpath \"/tmp\""));
        assert!(profile.contains("(allow network*)"));
    }

    #[test]
    fn disabled_policy_does_not_wrap() {
        let mut cmd = tokio_command("bash");
        cmd.arg("-lc").arg("echo hi");
        let applied = apply_sandbox(&SandboxPolicy::disabled(), Path::new("/repo"), &mut cmd);
        assert_eq!(applied.kind, SandboxKind::None);
        assert!(applied.note.is_none());
        assert_eq!(cmd.as_std().get_program(), "bash");
    }

    #[test]
    fn status_label_explains_unavailable() {
        assert_eq!(sandbox_status_label(false), "关闭");
        let enabled = sandbox_status_label(true);
        assert!(
            enabled.contains("bubblewrap")
                || enabled.contains("seatbelt")
                || enabled.contains("不可用")
        );
    }
}
