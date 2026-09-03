//! Shell 初始化快照。
//!
//! 每个会话只启动一次 login shell，把用户的函数 / 别名 / shell 选项 / PATH
//! 导出到 `$APPCONFIG/shell-snapshots/snapshot-bash-<时间戳>-<随机>.sh`。
//! 之后每次 Bash 只 `source` 这个文件，比每次 `bash -lc` 启动 login shell
//! 更快，也不会因为 profile 里的交互式输出污染工具结果。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::AsyncReadExt;

use crate::process_spawn::tokio_command;

const SNAPSHOT_DIR_NAME: &str = "shell-snapshots";
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(20);
/// 只保留最近的几份快照，避免目录无限增长。
const KEEP_SNAPSHOTS: usize = 5;
/// 环境变量名：Bash 工具用它把命令交给 `eval`，避免二次引号转义。
pub const COMMAND_ENV: &str = "NOXCODE_BASH_COMMAND";
pub const SNAPSHOT_ENV: &str = "NOXCODE_SHELL_SNAPSHOT";

/// 在 login shell 里执行的导出脚本。输出即为快照文件内容。
const CAPTURE_SCRIPT: &str = r##"
echo "# noxcode shell snapshot"
echo "# Unset all aliases to avoid conflicts with functions"
echo "unalias -a 2>/dev/null || true"
echo ""
echo "# Functions"
declare -f 2>/dev/null
echo ""
echo "# Shell Options"
shopt -p 2>/dev/null | head -n 1000
echo "shopt -s expand_aliases"
echo ""
echo "# Aliases"
alias -p 2>/dev/null
echo ""
echo "# Add PATH to the file"
printf 'export PATH=%q\n' "$PATH"
"##;

pub fn snapshot_dir(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join(SNAPSHOT_DIR_NAME)
}

fn snapshot_file_name() -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|item| item.as_millis())
        .unwrap_or(0);
    let random: String = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(6)
        .collect();
    format!("snapshot-bash-{stamp}-{random}.sh")
}

/// 启动 login shell 导出快照。失败时返回错误，调用方应回退到 `bash -lc`。
pub async fn capture_shell_snapshot(app_config_dir: &Path) -> Result<PathBuf, String> {
    let dir = snapshot_dir(app_config_dir);
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建快照目录失败: {error}"))?;
    let mut cmd = tokio_command("bash");
    cmd.arg("-lc")
        .arg(CAPTURE_SCRIPT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("启动 login shell 失败: {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "login shell stdout 不可用".to_string())?;
    let mut buf = Vec::new();
    let status = tokio::select! {
        result = child.wait() => {
            stdout.read_to_end(&mut buf).await.ok();
            result.map_err(|error| format!("login shell 执行失败: {error}"))?
        }
        _ = tokio::time::sleep(CAPTURE_TIMEOUT) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err("导出 shell 快照超时".to_string());
        }
    };
    if !status.success() {
        return Err(format!(
            "login shell 退出码 {}",
            status.code().unwrap_or(-1)
        ));
    }
    let text = String::from_utf8_lossy(&buf);
    if !text.contains("export PATH=") {
        return Err("shell 快照缺少 PATH".to_string());
    }
    let path = dir.join(snapshot_file_name());
    std::fs::write(&path, text.as_bytes()).map_err(|error| format!("写入快照失败: {error}"))?;
    prune_old_snapshots(&dir);
    Ok(path)
}

/// 只保留最新的 `KEEP_SNAPSHOTS` 份。
fn prune_old_snapshots(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !name.starts_with("snapshot-bash-") || !name.ends_with(".sh") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in files.into_iter().skip(KEEP_SNAPSHOTS) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("noxcode-shell-snapshot-{stamp}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[tokio::test]
    async fn capture_writes_path_export_and_prunes_old_files() {
        let root = temp_dir();
        let dir = snapshot_dir(&root);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for index in 0..(KEEP_SNAPSHOTS + 2) {
            let stale = dir.join(format!("snapshot-bash-{index}-old.sh"));
            std::fs::write(&stale, "# old").expect("write");
        }
        let path = capture_shell_snapshot(&root).await.expect("capture");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("export PATH="));
        assert!(text.contains("shopt -s expand_aliases"));
        let remaining = std::fs::read_dir(&dir).expect("dir").flatten().count();
        assert!(remaining <= KEEP_SNAPSHOTS);
        let _ = std::fs::remove_dir_all(root);
    }
}
