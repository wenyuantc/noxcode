//! 本地 Bash 的操作系统级沙箱。
//!
//! Linux 优先 `bwrap`（只读根 + 可写工作区 / 额外写根 / /tmp）。
//! macOS 使用 `sandbox-exec` seatbelt。Windows 与 SSH 不套沙箱。
//! 开启但找不到实现时回退为普通执行，并在结果里注明。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::native::tools::command_path::resolve_program;
use crate::process_spawn::tokio_command;

/// 只用系统自带的 `sandbox-exec`，避免 PATH 上的假二进制。
const MACOS_SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

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

    /// 合并工作区额外写根（记忆目录等），去重后用于生成沙箱配置。
    pub fn with_write_roots(&self, extra: &[PathBuf]) -> Self {
        let mut next = self.clone();
        for root in extra {
            if root.as_os_str().is_empty() {
                continue;
            }
            if !next.extra_write_roots.iter().any(|item| item == root) {
                next.extra_write_roots.push(root.clone());
            }
        }
        next
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
    if cfg!(target_os = "macos") && Path::new(MACOS_SANDBOX_EXEC).is_file() {
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

/// 构造可写路径列表：工作区 + 临时目录 + 额外写根。
/// macOS 同时列入 `/tmp` 与 `/private/tmp`，因为 Seatbelt 的 subpath 不跟随符号链接。
pub fn writable_paths(workspace: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_writable(&mut paths, workspace.to_path_buf());
    push_writable(&mut paths, PathBuf::from("/tmp"));
    if cfg!(target_os = "macos") {
        push_writable(&mut paths, PathBuf::from("/private/tmp"));
        push_writable(&mut paths, PathBuf::from("/var/tmp"));
        push_writable(&mut paths, PathBuf::from("/private/var/tmp"));
        push_writable(&mut paths, PathBuf::from("/var/folders"));
    }
    for extra in extra {
        if extra.as_os_str().is_empty() {
            continue;
        }
        push_writable(&mut paths, extra.clone());
    }
    paths
}

fn push_writable(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if let Ok(canon) = path.canonicalize() {
        if canon != path {
            push_unique(paths, canon);
        }
    }
    push_unique(paths, path);
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|item| item == &path) {
        paths.push(path);
    }
}

pub fn seatbelt_profile(workspace: &Path, extra: &[PathBuf]) -> String {
    let mut allows = String::from(
        "(version 1)\n\
(deny default)\n\
(allow file-read*)\n\
(allow file-map-executable)\n\
(allow process-exec)\n\
(allow process-fork)\n\
(allow process-info* (target same-sandbox))\n\
(allow signal (target same-sandbox))\n\
(allow sysctl-read)\n\
(allow mach-lookup)\n\
(allow system-socket (socket-domain AF_UNIX))\n\
(allow ipc-posix-sem)\n\
(allow network-outbound)\n\
(allow network-inbound)\n\
(allow network-bind)\n\
(allow pseudo-tty)\n\
(allow file-write* (literal \"/dev/null\"))\n\
(allow file-write* (literal \"/dev/stdout\"))\n\
(allow file-write* (literal \"/dev/stderr\"))\n\
(allow file-ioctl (literal \"/dev/null\"))\n\
(allow file-ioctl (literal \"/dev/stdout\"))\n\
(allow file-ioctl (literal \"/dev/stderr\"))\n\
(allow file-ioctl (regex #\"^/dev/tty.*\"))\n\
(allow file-read* file-write* file-ioctl (literal \"/dev/ptmx\"))\n",
    );
    for path in writable_paths(workspace, extra) {
        let escaped = escape_seatbelt_path(&path);
        allows.push_str(&format!("(allow file-write* (subpath \"{escaped}\"))\n"));
    }
    allows
}

fn escape_seatbelt_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

pub fn looks_like_sandbox_denial(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("operation not permitted")
        || output.contains("sandbox-exec:")
        || output.contains("deny(")
        || lower.contains("sandbox:")
}

pub fn sandbox_was_enforced(note: Option<&str>) -> bool {
    note.is_some_and(|note| note.contains("已在") && note.contains("沙箱中执行"))
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
    if !Path::new(MACOS_SANDBOX_EXEC).is_file() {
        return Err("找不到 /usr/bin/sandbox-exec".to_string());
    }
    let profile = seatbelt_profile(workspace, extra);
    let inner = take_program_and_args(cmd)?;
    let program = resolve_inner_program(&inner.program);
    let mut wrapped = tokio_command(MACOS_SANDBOX_EXEC);
    wrapped.arg("-p").arg(profile).arg("--").arg(&program);
    wrapped.args(inner.args);
    copy_stdios(cmd, &mut wrapped);
    *cmd = wrapped;
    Ok(())
}

fn resolve_inner_program(program: &Path) -> PathBuf {
    if program.is_absolute() {
        return program.to_path_buf();
    }
    let name = program.to_string_lossy();
    if name.is_empty() {
        return program.to_path_buf();
    }
    resolve_program(&name).unwrap_or_else(|_| program.to_path_buf())
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
    to.stdin(Stdio::null());
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
        assert!(profile.contains("(allow network-outbound)"));
        assert!(profile.contains("(allow file-map-executable)"));
        assert!(!profile.contains("(allow network*)"));
        assert!(!profile.contains("subpath \"/dev\""));
    }

    #[test]
    fn seatbelt_profile_includes_extra_write_roots() {
        let extra = [PathBuf::from("/tmp/nox-memory")];
        let profile = seatbelt_profile(Path::new("/repo"), &extra);
        assert!(profile.contains("subpath \"/tmp/nox-memory\""));
    }

    #[test]
    fn with_write_roots_dedupes() {
        let policy = SandboxPolicy {
            enabled: true,
            extra_write_roots: vec![PathBuf::from("/a")],
        };
        let merged = policy.with_write_roots(&[PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(
            merged.extra_write_roots,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn denial_hint_matches_kernel_and_sandbox_exec() {
        assert!(looks_like_sandbox_denial(
            "echo: /tmp/x: Operation not permitted"
        ));
        assert!(looks_like_sandbox_denial("sandbox-exec: profile failed"));
        assert!(looks_like_sandbox_denial("deny(1) file-write-data"));
        assert!(!looks_like_sandbox_denial("hello world"));
        assert!(sandbox_was_enforced(Some("已在 seatbelt 沙箱中执行")));
        assert!(!sandbox_was_enforced(Some(
            "沙箱启动失败，已回退为普通 Bash"
        )));
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

    #[cfg(target_os = "macos")]
    #[test]
    fn writable_paths_include_private_tmp_not_dev() {
        let paths = writable_paths(Path::new("/repo"), &[]);
        assert!(paths.contains(&PathBuf::from("/private/tmp")));
        assert!(paths.contains(&PathBuf::from("/var/folders")));
        assert!(!paths.contains(&PathBuf::from("/dev")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn enabled_policy_wraps_with_sandbox_exec() {
        if !Path::new(MACOS_SANDBOX_EXEC).is_file() {
            return;
        }
        let mut cmd = tokio_command("bash");
        cmd.arg("-lc").arg("echo hi");
        let policy = SandboxPolicy {
            enabled: true,
            extra_write_roots: vec![PathBuf::from("/tmp/nox-memory")],
        };
        let applied = apply_sandbox(&policy, Path::new("/repo"), &mut cmd);
        assert_eq!(applied.kind, SandboxKind::Seatbelt);
        assert_eq!(cmd.as_std().get_program(), MACOS_SANDBOX_EXEC);
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|arg| arg == "--"));
        assert!(args.iter().any(|arg| arg.contains("/tmp/nox-memory")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_echo_and_write_boundaries() {
        if !Path::new(MACOS_SANDBOX_EXEC).is_file() {
            return;
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) else {
            return;
        };
        let dir = PathBuf::from(&home).join(format!("noxcode-seatbelt-ws-{stamp}"));
        let outside = PathBuf::from(&home).join(format!("noxcode-sandbox-probe-{stamp}"));
        std::fs::create_dir_all(&dir).expect("mkdir workspace");
        let profile = seatbelt_profile(&dir, &[]);
        let echo = std::process::Command::new(MACOS_SANDBOX_EXEC)
            .arg("-p")
            .arg(&profile)
            .arg("--")
            .arg("/bin/echo")
            .arg("noxcode-sandbox-ok")
            .stdin(Stdio::null())
            .output()
            .expect("echo");
        let echo_err = String::from_utf8_lossy(&echo.stderr);
        assert!(
            echo.status.success(),
            "echo failed status={:?} stderr={echo_err}",
            echo.status
        );
        assert!(String::from_utf8_lossy(&echo.stdout).contains("noxcode-sandbox-ok"));

        let inside = dir.join("inside.txt");
        let write_ok = std::process::Command::new(MACOS_SANDBOX_EXEC)
            .arg("-p")
            .arg(&profile)
            .arg("--")
            .arg("/bin/bash")
            .arg("-c")
            .arg(format!("printf hello > '{}'", inside.display()))
            .stdin(Stdio::null())
            .output()
            .expect("write inside");
        assert!(
            write_ok.status.success(),
            "workspace write failed: {}",
            String::from_utf8_lossy(&write_ok.stderr)
        );
        assert_eq!(std::fs::read_to_string(&inside).unwrap().trim(), "hello");

        let _ = std::fs::remove_file(&outside);
        let write_out = std::process::Command::new(MACOS_SANDBOX_EXEC)
            .arg("-p")
            .arg(&profile)
            .arg("--")
            .arg("/bin/bash")
            .arg("-c")
            .arg(format!("printf leaked > '{}'", outside.display()))
            .stdin(Stdio::null())
            .output()
            .expect("write outside");
        let leaked = outside.exists();
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            !write_out.status.success() || !leaked,
            "sandbox allowed write outside workspace"
        );
    }
}
