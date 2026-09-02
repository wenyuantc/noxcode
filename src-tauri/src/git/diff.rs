use serde::{Deserialize, Serialize};

use super::runner::{git, split_nul_strings, GitError, GitTarget, IndexMode};
use super::status::get_status;

const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitFileDiffScope {
    Worktree,
    Staged,
    Range { from_oid: String, to_oid: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitNumstatScope {
    Worktree,
    Staged,
    Upstream,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitFileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub patch: String,
    pub is_binary: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitNumstatEntry {
    pub path: String,
    pub orig_path: Option<String>,
    pub added: Option<i64>,
    pub deleted: Option<i64>,
    pub is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameStatusEntry {
    pub status: String,
    pub path: String,
    pub orig_path: Option<String>,
}

pub(crate) async fn get_file_diff(
    target: &GitTarget,
    path: &str,
    scope: &GitFileDiffScope,
    old_path: Option<&str>,
) -> Result<GitFileDiff, GitError> {
    super::runner::assert_safe_rel_path(path)?;
    if let Some(old_path) = old_path {
        super::runner::assert_safe_rel_path(old_path)?;
    }

    let compare_path = old_path.unwrap_or(path);
    let mut args = vec![
        "diff".to_string(),
        "--no-ext-diff".to_string(),
        "--no-color".to_string(),
        "--binary".to_string(),
    ];
    match scope {
        GitFileDiffScope::Worktree => {}
        GitFileDiffScope::Staged => args.push("--cached".to_string()),
        GitFileDiffScope::Range { from_oid, to_oid } => {
            args.push(from_oid.clone());
            args.push(to_oid.clone());
        }
    }
    args.push("--".to_string());
    args.push(compare_path.to_string());
    if old_path.is_some() && compare_path != path {
        args.push(path.to_string());
    }

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git(target, &arg_refs, &IndexMode::ReadOnly).await?;
    if !output.success() && !output.stdout.is_empty() {
        // diff 在有差异时仍可能 exit 0；非 0 且无 stdout 才算失败
    }
    if !output.success() && output.stdout.is_empty() && !is_untracked_exit(&output.stderr_lossy()) {
        if matches!(scope, GitFileDiffScope::Worktree) {
            return untracked_file_diff(target, path).await;
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        return Err(output.command_error(&refs));
    }

    let mut patch = if output.stdout.is_empty() && matches!(scope, GitFileDiffScope::Worktree) {
        return untracked_file_diff(target, path).await;
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };

    classify_patch(path, old_path, &mut patch)
}

async fn untracked_file_diff(target: &GitTarget, path: &str) -> Result<GitFileDiff, GitError> {
    let status = get_status(target, Some("all")).await?;
    let is_untracked = status
        .entries
        .iter()
        .any(|entry| entry.kind == "untracked" && entry.path == path);
    if !is_untracked {
        return Ok(GitFileDiff {
            path: path.to_string(),
            old_path: None,
            patch: String::new(),
            is_binary: false,
            truncated: false,
        });
    }

    let dev_null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let output = git(
        target,
        &[
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-color",
            "--binary",
            "--",
            dev_null,
            path,
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    let mut patch = String::from_utf8_lossy(&output.stdout).into_owned();
    classify_patch(path, None, &mut patch)
}

fn is_untracked_exit(stderr: &str) -> bool {
    stderr.contains("does not exist") || stderr.contains("no such path")
}

fn classify_patch(
    path: &str,
    old_path: Option<&str>,
    patch: &mut String,
) -> Result<GitFileDiff, GitError> {
    let is_binary = patch.contains("GIT binary patch") || patch.contains("Binary files");
    if is_binary {
        if let Some(index) = patch
            .find("GIT binary patch")
            .or_else(|| patch.find("Binary files"))
        {
            let keep_end = patch[index..]
                .find('\n')
                .map(|offset| index + offset + 1)
                .unwrap_or(patch.len());
            patch.truncate(keep_end);
        }
    }
    let truncated = patch.len() > MAX_DIFF_BYTES;
    if truncated {
        patch.truncate(MAX_DIFF_BYTES);
    }
    Ok(GitFileDiff {
        path: path.to_string(),
        old_path: old_path.map(ToOwned::to_owned),
        patch: std::mem::take(patch),
        is_binary,
        truncated,
    })
}

pub(crate) async fn get_numstat(
    target: &GitTarget,
    scope: &GitNumstatScope,
) -> Result<Vec<GitNumstatEntry>, GitError> {
    let mut args = vec![
        "diff".to_string(),
        "--numstat".to_string(),
        "-z".to_string(),
        "--find-renames".to_string(),
    ];
    match scope {
        GitNumstatScope::Worktree => {}
        GitNumstatScope::Staged => args.push("--cached".to_string()),
        GitNumstatScope::Upstream => {
            let status = get_status(target, Some("no")).await?;
            let Some(upstream) = status.branch.upstream else {
                return Err(GitError::Parse("当前分支没有设置 upstream".to_string()));
            };
            args.push(format!("{upstream}...HEAD"));
        }
    }
    args.push("--".to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git(target, &refs, &IndexMode::ReadOnly).await?;
    output.require_success(&refs)?;
    parse_numstat(&output.stdout)
}

pub(crate) fn parse_numstat(bytes: &[u8]) -> Result<Vec<GitNumstatEntry>, GitError> {
    let records = split_nul_strings(bytes);
    let mut entries = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index].clone();
        if let Some((added, deleted, rest)) = split_numstat_prefix(&record) {
            if rest.is_empty() {
                let orig = records
                    .get(index + 1)
                    .cloned()
                    .ok_or_else(|| GitError::Parse("numstat rename 缺少 orig_path".to_string()))?;
                let path = records
                    .get(index + 2)
                    .cloned()
                    .ok_or_else(|| GitError::Parse("numstat rename 缺少 path".to_string()))?;
                entries.push(GitNumstatEntry {
                    path,
                    orig_path: Some(orig),
                    added,
                    deleted,
                    is_binary: added.is_none() && deleted.is_none(),
                });
                index += 3;
                continue;
            }
            entries.push(GitNumstatEntry {
                path: rest,
                orig_path: None,
                added,
                deleted,
                is_binary: added.is_none() && deleted.is_none(),
            });
            index += 1;
        } else {
            index += 1;
        }
    }
    Ok(entries)
}

fn split_numstat_prefix(record: &str) -> Option<(Option<i64>, Option<i64>, String)> {
    let mut parts = record.splitn(3, '\t');
    let added = parts.next()?;
    let deleted = parts.next()?;
    let path = parts.next().unwrap_or("").to_string();
    Some((parse_stat_count(added), parse_stat_count(deleted), path))
}

fn parse_stat_count(value: &str) -> Option<i64> {
    if value == "-" {
        None
    } else {
        value.parse().ok()
    }
}

pub(crate) async fn name_status_against(
    target: &GitTarget,
    oid: &str,
) -> Result<Vec<NameStatusEntry>, GitError> {
    let output = git(
        target,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            oid,
            "--",
            ".",
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["diff", "--name-status"])?;
    parse_name_status(&output.stdout)
}

pub(crate) fn parse_name_status(bytes: &[u8]) -> Result<Vec<NameStatusEntry>, GitError> {
    let records = split_nul_strings(bytes);
    let mut entries = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let code = records[index].clone();
        if code.starts_with('R') || code.starts_with('C') {
            let orig = records
                .get(index + 1)
                .cloned()
                .ok_or_else(|| GitError::Parse("name-status rename 缺少路径".to_string()))?;
            let path = records
                .get(index + 2)
                .cloned()
                .ok_or_else(|| GitError::Parse("name-status rename 缺少新路径".to_string()))?;
            entries.push(NameStatusEntry {
                status: code,
                path,
                orig_path: Some(orig),
            });
            index += 3;
        } else {
            let path = records
                .get(index + 1)
                .cloned()
                .ok_or_else(|| GitError::Parse("name-status 缺少路径".to_string()))?;
            entries.push(NameStatusEntry {
                status: code,
                path,
                orig_path: None,
            });
            index += 2;
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numstat_rename_and_binary() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"0\t0\t\0hello world.txt\0hello  world.txt\0");
        raw.extend_from_slice(b"-\t-\timage.png\0");
        raw.extend_from_slice(b"3\t1\tsrc/main.rs\0");
        let entries = parse_numstat(&raw).unwrap();
        assert_eq!(entries[0].path, "hello  world.txt");
        assert_eq!(entries[0].orig_path.as_deref(), Some("hello world.txt"));
        assert_eq!(entries[1].path, "image.png");
        assert!(entries[1].is_binary);
        assert_eq!(entries[2].added, Some(3));
        assert_eq!(entries[2].deleted, Some(1));
    }

    #[test]
    fn parses_name_status_no_renames() {
        let raw = b"M\0src/main.rs\0D\0gone.rs\0A\0new.rs\0";
        let entries = parse_name_status(raw).unwrap();
        assert_eq!(entries[0].status, "M");
        assert_eq!(entries[1].status, "D");
        assert_eq!(entries[2].status, "A");
    }

    #[test]
    fn binary_patch_is_stripped() {
        let mut patch =
            "diff --git a/a.bin b/a.bin\nGIT binary patch\nliteral 12\nabcdef\n".to_string();
        let diff = classify_patch("a.bin", None, &mut patch).unwrap();
        assert!(diff.is_binary);
        assert!(diff.patch.contains("GIT binary patch"));
        assert!(!diff.patch.contains("literal"));
    }
}
