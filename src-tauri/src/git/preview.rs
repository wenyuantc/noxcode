use std::path::{Component, Path};

use serde::Serialize;
use tokio::io::AsyncReadExt;

use crate::app::ssh::exec::ExecOptions;
use crate::app::ssh::shell::shell_escape_single_quoted;

use super::diff::{get_file_diff, GitFileDiff, GitFileDiffScope};
use super::runner::{assert_safe_rel_path, git, wrap_ssh_script, GitError, GitTarget, IndexMode};

const MAX_PREVIEW_BYTES: usize = 256 * 1024;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum GitFilePreview {
    Diff {
        scope: GitFileDiffScope,
        diff: GitFileDiff,
    },
    Content {
        path: String,
        content: String,
        is_binary: bool,
        truncated: bool,
        reason: FilePreviewReason,
    },
    Missing {
        path: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FilePreviewReason {
    Ignored,
    Unchanged,
    NotRepository,
}

pub(crate) async fn get_file_preview(
    target: &GitTarget,
    path: &str,
) -> Result<GitFilePreview, GitError> {
    let path = relative_preview_path(target, path)?;
    let args = ["rev-parse", "--is-inside-work-tree"];
    let repo = git(target, &args, &IndexMode::ReadOnly).await?;
    let in_repo = if repo.success() {
        repo.stdout_lossy().trim() == "true"
    } else if repo.stderr_lossy().contains("not a git repository") {
        false
    } else {
        return Err(repo.command_error(&args));
    };

    let reason = if in_repo {
        // 会话文件记录不是 Git 状态：先查未暂存差异，再查已暂存差异。
        for scope in [GitFileDiffScope::Worktree, GitFileDiffScope::Staged] {
            let diff = get_file_diff(target, &path, &scope, None).await?;
            if diff.is_binary || !diff.patch.trim().is_empty() {
                return Ok(GitFilePreview::Diff { scope, diff });
            }
        }
        let args = ["check-ignore", "--quiet", "--", &path];
        let ignored = git(target, &args, &IndexMode::ReadOnly).await?;
        match ignored.exit_code {
            0 => FilePreviewReason::Ignored,
            1 => FilePreviewReason::Unchanged,
            _ => return Err(ignored.command_error(&args)),
        }
    } else {
        FilePreviewReason::NotRepository
    };

    let Some(bytes) = read_preview_bytes(target, &path).await? else {
        return Ok(GitFilePreview::Missing { path });
    };
    Ok(content_preview(path, &bytes, reason))
}

fn relative_preview_path(target: &GitTarget, path: &str) -> Result<String, GitError> {
    let invalid = || GitError::Blocked(format!("路径超出工作区或无效: {path}"));
    let relative = match target {
        GitTarget::Local(root) if Path::new(path).is_absolute() => Path::new(path)
            .strip_prefix(root)
            .or_else(|_| {
                root.canonicalize()
                    .ok()
                    .and_then(|root| Path::new(path).strip_prefix(root).ok())
                    .ok_or(())
            })
            .map_err(|_| invalid())?
            .to_string_lossy()
            .into_owned(),
        GitTarget::Ssh { repo_path, .. } if path.starts_with('/') => path
            .strip_prefix(&format!("{}/", repo_path.trim_end_matches('/')))
            .ok_or_else(invalid)?
            .to_string(),
        _ => path.to_string(),
    };
    assert_safe_rel_path(&relative)?;
    if relative.contains('\0') {
        return Err(invalid());
    }
    let mut parts = Vec::new();
    for component in Path::new(&relative).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(invalid()),
        }
    }
    if parts.is_empty() {
        return Err(invalid());
    }
    Ok(parts.join("/"))
}

async fn read_preview_bytes(target: &GitTarget, path: &str) -> Result<Option<Vec<u8>>, GitError> {
    match target {
        GitTarget::Local(root) => {
            let root = tokio::fs::canonicalize(root).await?;
            let resolved = match tokio::fs::canonicalize(root.join(path)).await {
                Ok(path) => path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            if !resolved.starts_with(&root) {
                return Err(GitError::Blocked("路径超出工作区".to_string()));
            }
            if !tokio::fs::metadata(&resolved).await?.is_file() {
                return Err(GitError::Blocked("只能预览普通文件".to_string()));
            }
            let file = tokio::fs::File::open(resolved).await?;
            let mut bytes = Vec::new();
            file.take((MAX_PREVIEW_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok(Some(bytes))
        }
        GitTarget::Ssh {
            pool,
            params,
            repo_path,
        } => {
            let (parent, name) = path.rsplit_once('/').unwrap_or((".", path));
            let parent = shell_escape_single_quoted(&format!("./{parent}"));
            let name = shell_escape_single_quoted(&format!("./{name}"));
            // 物理父目录必须留在工作区内；不跟随远端文件符号链接。
            let script = format!(
                "cd {} || exit 1\n\
                 root=$(pwd -P) || exit 1\n\
                 if [ ! -e {parent} ]; then exit 3; fi\n\
                 cd -P {parent} || exit 1\n\
                 case \"$(pwd -P)/\" in \"$root/\"*) ;; *) printf '%s' '路径超出工作区' >&2; exit 1;; esac\n\
                 if [ -L {name} ]; then printf '%s' '无法预览远端符号链接' >&2; exit 1; fi\n\
                 if [ ! -e {name} ]; then exit 3; fi\n\
                 if [ ! -f {name} ]; then printf '%s' '只能预览普通文件' >&2; exit 1; fi\n\
                 head -c {} < {name}",
                shell_escape_single_quoted(repo_path),
                MAX_PREVIEW_BYTES + 1,
            );
            let output = pool
                .exec(params, &wrap_ssh_script(&script), ExecOptions::default())
                .await
                .map_err(|error| GitError::Ssh(error.to_string()))?;
            match output.exit_code {
                Some(0) => Ok(Some(output.stdout)),
                Some(3) => Ok(None),
                _ => Err(GitError::Ssh(format!(
                    "读取文件失败: {}",
                    output.stderr_lossy()
                ))),
            }
        }
    }
}

fn content_preview(path: String, bytes: &[u8], reason: FilePreviewReason) -> GitFilePreview {
    let truncated = bytes.len() > MAX_PREVIEW_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_PREVIEW_BYTES)];
    let text = std::str::from_utf8(bytes);
    let (content, is_binary) = match text {
        Ok(text) if !bytes.contains(&0) => (text.to_string(), false),
        Err(error) if truncated && error.error_len().is_none() && !bytes.contains(&0) => (
            String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned(),
            false,
        ),
        _ => (String::new(), true),
    };
    GitFilePreview::Content {
        path,
        content,
        is_binary,
        truncated,
        reason,
    }
}
