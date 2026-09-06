use super::runner::{git, GitError, GitTarget, IndexMode};
use super::status::{get_status, GitStatusEntry};

pub(crate) const COMMIT_MESSAGE_CONTEXT_LIMIT: usize = 32 * 1024;

fn xy_changed(byte: Option<u8>) -> bool {
    matches!(byte, Some(value) if !matches!(value, b' ' | b'.' | b'?' | b'!'))
}

fn is_untracked(entry: &GitStatusEntry) -> bool {
    entry.kind == "untracked" || entry.xy.starts_with('?')
}

fn is_staged(entry: &GitStatusEntry) -> bool {
    !is_untracked(entry) && xy_changed(entry.xy.as_bytes().first().copied())
}

fn is_unstaged(entry: &GitStatusEntry) -> bool {
    !is_untracked(entry) && xy_changed(entry.xy.as_bytes().get(1).copied())
}

fn truncate_context(mut text: String) -> String {
    if text.len() <= COMMIT_MESSAGE_CONTEXT_LIMIT {
        return text;
    }
    let mut end = COMMIT_MESSAGE_CONTEXT_LIMIT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n…（已截断）");
    text
}

async fn collect_scope_diff(target: &GitTarget, staged: bool) -> Result<String, GitError> {
    let mut args = vec![
        "diff".to_string(),
        "--no-ext-diff".to_string(),
        "--no-color".to_string(),
        "--find-renames".to_string(),
    ];
    if staged {
        args.push("--cached".to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git(target, &refs, &IndexMode::ReadOnly).await?;
    if !(output.success() || output.exit_code == 1) {
        return Err(output.command_error(&refs));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 优先暂存区；暂存为空则用工作区改动 + 未跟踪文件列表。干净仓库返回错误。
pub(crate) async fn collect_commit_message_context(target: &GitTarget) -> Result<String, GitError> {
    let status = get_status(target, Some("all")).await?;
    let staged: Vec<&GitStatusEntry> = status
        .entries
        .iter()
        .filter(|item| is_staged(item))
        .collect();
    let unstaged: Vec<&GitStatusEntry> = status
        .entries
        .iter()
        .filter(|item| is_unstaged(item))
        .collect();
    let untracked: Vec<&GitStatusEntry> = status
        .entries
        .iter()
        .filter(|item| is_untracked(item))
        .collect();
    if staged.is_empty() && unstaged.is_empty() && untracked.is_empty() {
        return Err(GitError::Parse("没有未提交的更改".to_string()));
    }

    let mut parts = Vec::new();
    if let Some(head) = status.branch.head.as_deref() {
        parts.push(format!("分支: {head}"));
    }

    if !staged.is_empty() {
        parts.push("范围: 暂存区".to_string());
        parts.push(format!(
            "文件:\n{}",
            staged
                .iter()
                .map(|item| format!("- {}", item.path))
                .collect::<Vec<_>>()
                .join("\n")
        ));
        let diff = collect_scope_diff(target, true).await?;
        if !diff.trim().is_empty() {
            parts.push(diff);
        }
    } else {
        parts.push("范围: 工作区".to_string());
        let names = unstaged
            .iter()
            .chain(untracked.iter())
            .map(|item| format!("- {}", item.path))
            .collect::<Vec<_>>();
        if !names.is_empty() {
            parts.push(format!("文件:\n{}", names.join("\n")));
        }
        let diff = collect_scope_diff(target, false).await?;
        if !diff.trim().is_empty() {
            parts.push(diff);
        }
        if !untracked.is_empty() {
            parts.push(format!(
                "未跟踪文件:\n{}",
                untracked
                    .iter()
                    .map(|item| format!("- {}", item.path))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }

    Ok(truncate_context(parts.join("\n\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_appends_marker() {
        let oversized = "a".repeat(COMMIT_MESSAGE_CONTEXT_LIMIT + 8);
        let truncated = truncate_context(oversized);
        assert!(truncated.ends_with("…（已截断）"));
        assert!(truncated.len() < COMMIT_MESSAGE_CONTEXT_LIMIT + 32);
    }
}
