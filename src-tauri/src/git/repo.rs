use serde::{Deserialize, Serialize};

use super::preflight::{evaluate_git_version_output, GitVersion, MIN_GIT_VERSION};
use super::runner::{git, split_nul_strings, GitError, GitTarget, IndexMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRepoInfo {
    pub workspace_id: String,
    pub toplevel: String,
    pub prefix: String,
    pub git_dir: String,
    pub common_dir: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub git_version: String,
}

pub(crate) async fn verify_target_git_version(target: &GitTarget) -> Result<GitVersion, GitError> {
    let output = git(target, &["--version"], &IndexMode::ReadOnly).await?;
    output.require_success(&["--version"])?;
    evaluate_git_version_output(&output.stdout_lossy()).map_err(|error| match error {
        super::preflight::GitPreflightError::TooOld { found } => GitError::VersionTooOld {
            found: found.to_string(),
            required: MIN_GIT_VERSION.to_string(),
        },
        other => GitError::Parse(other.to_string()),
    })
}

pub(crate) async fn load_repo_info(
    target: &GitTarget,
    workspace_id: &str,
) -> Result<GitRepoInfo, GitError> {
    let version = verify_target_git_version(target).await?;
    let parsed = git(
        target,
        &[
            "rev-parse",
            "--show-toplevel",
            "--show-prefix",
            "--absolute-git-dir",
            "--git-common-dir",
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    if !parsed.success() {
        return Err(GitError::NotARepo(parsed.stderr_lossy()));
    }
    let stdout = parsed.stdout_lossy();
    let mut lines = stdout.lines();
    let toplevel = lines.next().unwrap_or_default().trim().to_string();
    let prefix = lines.next().unwrap_or_default().trim().to_string();
    let git_dir = lines.next().unwrap_or_default().trim().to_string();
    let common_dir = lines.next().unwrap_or_default().trim().to_string();
    if toplevel.is_empty() || git_dir.is_empty() {
        return Err(GitError::NotARepo("rev-parse 输出不完整".to_string()));
    }

    Ok(GitRepoInfo {
        workspace_id: workspace_id.to_string(),
        toplevel,
        prefix,
        git_dir,
        common_dir,
        head: head_oid(target).await?,
        branch: abbrev_ref(target, "HEAD").await?,
        upstream: abbrev_ref(target, "@{upstream}").await?,
        git_version: version.to_string(),
    })
}

pub(crate) async fn head_oid(target: &GitTarget) -> Result<Option<String>, GitError> {
    let output = git(
        target,
        &["rev-parse", "-q", "--verify", "HEAD"],
        &IndexMode::ReadOnly,
    )
    .await?;
    if output.success() {
        let oid = output.stdout_lossy().trim().to_string();
        if oid.is_empty() {
            Ok(None)
        } else {
            Ok(Some(oid))
        }
    } else {
        Ok(None)
    }
}

async fn abbrev_ref(target: &GitTarget, spec: &str) -> Result<Option<String>, GitError> {
    let output = git(
        target,
        &["rev-parse", "--abbrev-ref", spec],
        &IndexMode::ReadOnly,
    )
    .await?;
    if !output.success() {
        return Ok(None);
    }
    let value = output.stdout_lossy().trim().to_string();
    if value.is_empty() || value == "HEAD" {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

pub(crate) async fn in_progress_operation(target: &GitTarget) -> Result<Option<String>, GitError> {
    for (name, spec) in [
        ("merge", "MERGE_HEAD"),
        ("rebase", "REBASE_HEAD"),
        ("cherry-pick", "CHERRY_PICK_HEAD"),
        ("revert", "REVERT_HEAD"),
    ] {
        let output = git(
            target,
            &["rev-parse", "-q", "--verify", spec],
            &IndexMode::ReadOnly,
        )
        .await?;
        if output.success() {
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

pub(crate) async fn is_detached_head(target: &GitTarget) -> Result<bool, GitError> {
    let output = git(
        target,
        &["symbolic-ref", "-q", "HEAD"],
        &IndexMode::ReadOnly,
    )
    .await?;
    Ok(!output.success())
}

pub(crate) async fn rev_parse_verify(
    target: &GitTarget,
    spec: &str,
) -> Result<Option<String>, GitError> {
    let output = git(
        target,
        &["rev-parse", "--verify", spec],
        &IndexMode::ReadOnly,
    )
    .await?;
    if output.success() {
        Ok(Some(output.stdout_lossy().trim().to_string()))
    } else {
        Ok(None)
    }
}

const DEFAULT_FILE_LIST_LIMIT: usize = 200;
const MAX_FILE_LIST_LIMIT: usize = 1000;

pub(crate) async fn list_repo_files(
    target: &GitTarget,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<String>, GitError> {
    let output = git(
        target,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["ls-files"])?;
    let mut paths = split_nul_strings(&output.stdout);
    if let Some(needle) = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
    {
        paths.retain(|path| path.to_lowercase().contains(&needle));
    }
    paths.sort();
    let limit = limit
        .unwrap_or(DEFAULT_FILE_LIST_LIMIT)
        .clamp(1, MAX_FILE_LIST_LIMIT);
    paths.truncate(limit);
    Ok(paths)
}
