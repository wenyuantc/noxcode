use serde::{Deserialize, Serialize};

use super::runner::{git, split_nul_strings, GitError, GitTarget, IndexMode};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitBranchInfo {
    pub oid: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitStatusEntry {
    pub kind: String,
    pub xy: String,
    pub path: String,
    pub orig_path: Option<String>,
    pub score: Option<String>,
    pub mode_head: Option<String>,
    pub mode_index: Option<String>,
    pub mode_worktree: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitStatus {
    pub branch: GitBranchInfo,
    pub entries: Vec<GitStatusEntry>,
}

pub(crate) async fn get_status(
    target: &GitTarget,
    untracked_mode: Option<&str>,
) -> Result<GitStatus, GitError> {
    let mode = match untracked_mode.unwrap_or("all") {
        value @ ("all" | "normal" | "no") => value,
        other => return Err(GitError::Parse(format!("无效的 untracked_mode: {other}"))),
    };
    let untracked = format!("--untracked-files={mode}");
    let output = git(
        target,
        &["status", "--porcelain=v2", "--branch", &untracked, "-z"],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["status", "--porcelain=v2"])?;
    parse_porcelain_v2(&output.stdout)
}

pub(crate) fn parse_porcelain_v2(bytes: &[u8]) -> Result<GitStatus, GitError> {
    let records = split_nul_strings(bytes);
    let mut branch = GitBranchInfo {
        oid: None,
        head: None,
        upstream: None,
        ahead: None,
        behind: None,
    };
    let mut entries = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = &records[index];
        if let Some(rest) = record.strip_prefix("# ") {
            parse_branch_header(rest, &mut branch);
            index += 1;
            continue;
        }
        if let Some(rest) = record.strip_prefix("? ") {
            entries.push(GitStatusEntry {
                kind: "untracked".to_string(),
                xy: "??".to_string(),
                path: rest.to_string(),
                orig_path: None,
                score: None,
                mode_head: None,
                mode_index: None,
                mode_worktree: None,
            });
            index += 1;
            continue;
        }
        if let Some(rest) = record.strip_prefix("! ") {
            entries.push(GitStatusEntry {
                kind: "ignored".to_string(),
                xy: "!!".to_string(),
                path: rest.to_string(),
                orig_path: None,
                score: None,
                mode_head: None,
                mode_index: None,
                mode_worktree: None,
            });
            index += 1;
            continue;
        }
        if record.starts_with("1 ") {
            entries.push(parse_ordinary(record)?);
            index += 1;
            continue;
        }
        if record.starts_with("2 ") {
            let orig = records
                .get(index + 1)
                .cloned()
                .ok_or_else(|| GitError::Parse("rename 条目缺少 orig_path".to_string()))?;
            entries.push(parse_rename(record, orig)?);
            index += 2;
            continue;
        }
        if record.starts_with("u ") {
            entries.push(parse_unmerged(record)?);
            index += 1;
            continue;
        }
        index += 1;
    }
    Ok(GitStatus { branch, entries })
}

fn parse_branch_header(rest: &str, branch: &mut GitBranchInfo) {
    if let Some(value) = rest.strip_prefix("branch.oid ") {
        if value != "(initial)" {
            branch.oid = Some(value.to_string());
        }
    } else if let Some(value) = rest.strip_prefix("branch.head ") {
        if value != "(detached)" {
            branch.head = Some(value.to_string());
        }
    } else if let Some(value) = rest.strip_prefix("branch.upstream ") {
        branch.upstream = Some(value.to_string());
    } else if let Some(value) = rest.strip_prefix("branch.ab ") {
        parse_ahead_behind(value, branch);
    }
}

fn parse_ahead_behind(value: &str, branch: &mut GitBranchInfo) {
    let mut ahead = None;
    let mut behind = None;
    for part in value.split_whitespace() {
        if let Some(number) = part.strip_prefix('+') {
            ahead = number.parse().ok();
        } else if let Some(number) = part.strip_prefix('-') {
            behind = number.parse().ok();
        }
    }
    branch.ahead = ahead;
    branch.behind = behind;
}

fn parse_ordinary(record: &str) -> Result<GitStatusEntry, GitError> {
    let rest = record
        .strip_prefix("1 ")
        .ok_or_else(|| GitError::Parse(record.to_string()))?;
    if rest.len() < 3 {
        return Err(GitError::Parse(record.to_string()));
    }
    let xy = rest[..2].to_string();
    let fields = rest[3..].splitn(7, ' ').collect::<Vec<_>>();
    if fields.len() < 7 {
        return Err(GitError::Parse(format!("ordinary 字段不足: {record}")));
    }
    Ok(GitStatusEntry {
        kind: "ordinary".to_string(),
        xy,
        path: fields[6].to_string(),
        orig_path: None,
        score: None,
        mode_head: Some(fields[1].to_string()),
        mode_index: Some(fields[2].to_string()),
        mode_worktree: Some(fields[3].to_string()),
    })
}

fn parse_rename(record: &str, orig_path: String) -> Result<GitStatusEntry, GitError> {
    let rest = record
        .strip_prefix("2 ")
        .ok_or_else(|| GitError::Parse(record.to_string()))?;
    if rest.len() < 3 {
        return Err(GitError::Parse(record.to_string()));
    }
    let xy = rest[..2].to_string();
    let fields = rest[3..].splitn(8, ' ').collect::<Vec<_>>();
    if fields.len() < 8 {
        return Err(GitError::Parse(format!("rename 字段不足: {record}")));
    }
    Ok(GitStatusEntry {
        kind: "rename".to_string(),
        xy,
        path: fields[7].to_string(),
        orig_path: Some(orig_path),
        score: Some(fields[6].to_string()),
        mode_head: Some(fields[1].to_string()),
        mode_index: Some(fields[2].to_string()),
        mode_worktree: Some(fields[3].to_string()),
    })
}

fn parse_unmerged(record: &str) -> Result<GitStatusEntry, GitError> {
    let rest = record
        .strip_prefix("u ")
        .ok_or_else(|| GitError::Parse(record.to_string()))?;
    if rest.len() < 3 {
        return Err(GitError::Parse(record.to_string()));
    }
    let xy = rest[..2].to_string();
    let fields = rest[3..].splitn(9, ' ').collect::<Vec<_>>();
    if fields.len() < 9 {
        return Err(GitError::Parse(format!("unmerged 字段不足: {record}")));
    }
    Ok(GitStatusEntry {
        kind: "unmerged".to_string(),
        xy,
        path: fields[8].to_string(),
        orig_path: None,
        score: None,
        mode_head: None,
        mode_index: None,
        mode_worktree: Some(fields[4].to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headers_rename_and_special_paths() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"# branch.oid abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +1 -2\0");
        raw.extend_from_slice(
            b"2 R. N... 100644 100644 100644 111 222 R100 hello  world.txt\0hello world.txt\0",
        );
        raw.extend_from_slice("? 我的 文件.txt\0".as_bytes());
        raw.extend_from_slice(b"? line\nbreak.txt\0");
        raw.extend_from_slice(b"1 M. N... 100644 100644 100644 aaa bbb src/main.rs\0");
        raw.extend_from_slice(b"u UU N... 100644 100644 100644 100644 a b c conflict.rs\0");

        let status = parse_porcelain_v2(&raw).unwrap();
        assert_eq!(status.branch.oid.as_deref(), Some("abcdef"));
        assert_eq!(status.branch.head.as_deref(), Some("main"));
        assert_eq!(status.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.branch.ahead, Some(1));
        assert_eq!(status.branch.behind, Some(2));
        assert_eq!(status.entries[0].kind, "rename");
        assert_eq!(status.entries[0].path, "hello  world.txt");
        assert_eq!(
            status.entries[0].orig_path.as_deref(),
            Some("hello world.txt")
        );
        assert_eq!(status.entries[0].score.as_deref(), Some("R100"));
        assert_eq!(status.entries[1].path, "我的 文件.txt");
        assert_eq!(status.entries[2].path, "line\nbreak.txt");
        assert_eq!(status.entries[3].kind, "ordinary");
        assert_eq!(status.entries[3].xy, "M.");
        assert_eq!(status.entries[4].kind, "unmerged");
        assert_eq!(status.entries[4].path, "conflict.rs");
    }
}
