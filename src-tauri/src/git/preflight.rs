use std::fmt;
use std::io;
use std::sync::OnceLock;
use std::thread;

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use super::runner;

pub const MIN_GIT_VERSION: GitVersion = GitVersion {
    major: 2,
    minor: 11,
    patch: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GitPreflightError {
    #[error("未检测到系统 git。noxcode 需要 git ≥ {required}，请安装后再启动。", required = MIN_GIT_VERSION)]
    NotFound,
    #[error("探测系统 git 失败：{0}")]
    Spawn(String),
    #[error("无法解析 git 版本输出：{0}")]
    Unparsable(String),
    #[error(
        "系统 git 版本过低（当前 {found}，最低要求 {required}）。请升级 git 后再启动。",
        required = MIN_GIT_VERSION
    )]
    TooOld { found: GitVersion },
}

pub fn parse_git_version(output: &str) -> Option<GitVersion> {
    let trimmed = output.trim();
    let version_part = trimmed
        .strip_prefix("git version ")
        .unwrap_or(trimmed)
        .split_whitespace()
        .next()?;
    let mut parts = version_part.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .and_then(|part| {
            part.chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0);
    Some(GitVersion {
        major,
        minor,
        patch,
    })
}

pub fn evaluate_git_version_output(output: &str) -> Result<GitVersion, GitPreflightError> {
    let version = parse_git_version(output)
        .ok_or_else(|| GitPreflightError::Unparsable(output.trim().to_string()))?;
    if version < MIN_GIT_VERSION {
        return Err(GitPreflightError::TooOld { found: version });
    }
    Ok(version)
}

pub fn check_local_git() -> Result<GitVersion, GitPreflightError> {
    let output = match runner::git_version_output() {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(GitPreflightError::NotFound);
        }
        Err(error) => return Err(GitPreflightError::Spawn(error.to_string())),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("退出码 {}", output.status)
        };
        return Err(GitPreflightError::Spawn(detail));
    }
    evaluate_git_version_output(&String::from_utf8_lossy(&output.stdout))
}

static FATAL_STARTUP_ERROR: OnceLock<String> = OnceLock::new();

pub fn run_startup_check() {
    match check_local_git() {
        Ok(version) => eprintln!("git 预检通过：{version}"),
        Err(error) => {
            eprintln!("git 预检失败：{error}");
            let _ = FATAL_STARTUP_ERROR.set(error.to_string());
        }
    }
}

pub fn show_fatal_dialog_if_needed<R: tauri::Runtime>(app: &AppHandle<R>) {
    let Some(message) = FATAL_STARTUP_ERROR.get() else {
        return;
    };
    spawn_fatal_dialog(app, message);
}

fn spawn_fatal_dialog<R: tauri::Runtime>(app: &AppHandle<R>, message: &str) {
    let handle = app.clone();
    let message = message.to_string();
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(300));
        handle
            .dialog()
            .message(message)
            .title("noxcode 无法启动")
            .kind(MessageDialogKind::Error)
            .blocking_show();
        std::process::exit(1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apple_git_version() {
        let version = parse_git_version("git version 2.39.5 (Apple Git-154)").unwrap();
        assert_eq!(
            version,
            GitVersion {
                major: 2,
                minor: 39,
                patch: 5
            }
        );
    }

    #[test]
    fn parses_windows_git_version() {
        let version = parse_git_version("git version 2.45.1.windows.1").unwrap();
        assert_eq!(
            version,
            GitVersion {
                major: 2,
                minor: 45,
                patch: 1
            }
        );
    }

    #[test]
    fn parses_plain_git_version() {
        let version = parse_git_version("2.11.0").unwrap();
        assert_eq!(version, MIN_GIT_VERSION);
    }

    #[test]
    fn unparsable_output_is_rejected() {
        let error = evaluate_git_version_output("not a version").unwrap_err();
        assert_eq!(
            error,
            GitPreflightError::Unparsable("not a version".to_string())
        );
    }

    #[test]
    fn too_old_version_is_rejected() {
        let error = evaluate_git_version_output("git version 2.10.9").unwrap_err();
        assert_eq!(
            error,
            GitPreflightError::TooOld {
                found: GitVersion {
                    major: 2,
                    minor: 10,
                    patch: 9
                }
            }
        );
    }

    #[test]
    fn minimum_and_current_versions_pass() {
        assert!(evaluate_git_version_output("git version 2.11.0").is_ok());
        assert!(evaluate_git_version_output("git version 2.39.5 (Apple Git-154)").is_ok());
    }

    fn with_path<T>(path: &str, f: impl FnOnce() -> T) -> T {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().expect("path lock");
        let previous = std::env::var_os("PATH");
        std::env::set_var("PATH", path);
        let result = f();
        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        result
    }

    #[test]
    fn check_local_git_rejects_old_binary_on_path() {
        let dir = std::env::temp_dir().join(format!("noxcode-git-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = dir.join("git");
        std::fs::write(&git, "#!/bin/sh\necho 'git version 2.9.0'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&git).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&git, permissions).unwrap();
        }
        let error = with_path(&dir.to_string_lossy(), check_local_git).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            error,
            GitPreflightError::TooOld {
                found: GitVersion {
                    major: 2,
                    minor: 9,
                    patch: 0
                }
            }
        );
    }

    #[test]
    fn check_local_git_reports_not_found_when_missing() {
        let dir = std::env::temp_dir().join(format!("noxcode-git-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = with_path(&dir.to_string_lossy(), check_local_git).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(error, GitPreflightError::NotFound);
    }
}
