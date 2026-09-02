use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::runner::{git, git_with, with_repo_lock, GitError, GitRunOptions, GitTarget, IndexMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitResult {
    pub oid: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPushResult {
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub set_upstream: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitBranch {
    pub name: String,
    pub oid: String,
    pub upstream: Option<String>,
    pub is_current: bool,
}

pub(crate) async fn commit_changes(
    target: &GitTarget,
    message: &str,
    paths: Option<&[String]>,
) -> Result<GitCommitResult, GitError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(GitError::Parse("提交信息不能为空".to_string()));
    }
    if let Some(paths) = paths {
        for path in paths {
            super::runner::assert_safe_rel_path(path)?;
        }
    }

    with_repo_lock(target, || async {
        let mut args = vec!["commit", "-m", message];
        let path_refs: Vec<&str> = paths.unwrap_or(&[]).iter().map(String::as_str).collect();
        if !path_refs.is_empty() {
            args.push("--");
            args.extend(path_refs.iter().copied());
        }
        git(target, &args, &IndexMode::user())
            .await?
            .require_success(&args)?;
        let oid = git(target, &["rev-parse", "HEAD"], &IndexMode::ReadOnly)
            .await?
            .require_success(&["rev-parse", "HEAD"])?
            .stdout_lossy()
            .trim()
            .to_string();
        Ok(GitCommitResult {
            oid,
            message: message.to_string(),
        })
    })
    .await
}

pub(crate) async fn push_branch(
    target: &GitTarget,
    remote: Option<&str>,
    branch: Option<&str>,
    set_upstream: bool,
) -> Result<GitPushResult, GitError> {
    let mut args = vec!["push".to_string()];
    if set_upstream {
        let remote =
            remote.ok_or_else(|| GitError::Parse("set_upstream 需要 remote".to_string()))?;
        let branch =
            branch.ok_or_else(|| GitError::Parse("set_upstream 需要 branch".to_string()))?;
        args.push("--set-upstream".to_string());
        args.push(remote.to_string());
        args.push(branch.to_string());
    } else {
        if let Some(remote) = remote {
            args.push(remote.to_string());
        }
        if let Some(branch) = branch {
            args.push(branch.to_string());
        }
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git_with(
        target,
        &refs,
        &IndexMode::ReadOnly,
        GitRunOptions {
            timeout: Some(Duration::from_secs(300)),
            stdin: None,
            extra_env: Vec::new(),
        },
    )
    .await?;
    output.require_success(&refs)?;
    Ok(GitPushResult {
        remote: remote.map(ToOwned::to_owned),
        branch: branch.map(ToOwned::to_owned),
        set_upstream,
        message: output.stderr_lossy(),
    })
}

pub(crate) async fn list_branches(target: &GitTarget) -> Result<Vec<GitBranch>, GitError> {
    let output = git(
        target,
        &[
            "for-each-ref",
            "--format=%(refname:short)%00%(objectname)%00%(upstream:short)%00%(HEAD)",
            "refs/heads",
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["for-each-ref"])?;
    let mut branches = Vec::new();
    for line in output.stdout_lossy().lines() {
        let mut parts = line.split('\0');
        let Some(name) = parts.next().filter(|value| !value.is_empty()) else {
            continue;
        };
        let oid = parts.next().unwrap_or_default().to_string();
        let upstream = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let is_current = parts.next() == Some("*");
        branches.push(GitBranch {
            name: name.to_string(),
            oid,
            upstream,
            is_current,
        });
    }
    Ok(branches)
}

pub(crate) async fn create_branch(
    target: &GitTarget,
    name: &str,
    checkout: bool,
) -> Result<GitBranch, GitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GitError::Parse("分支名不能为空".to_string()));
    }
    git(
        target,
        &["check-ref-format", "--branch", name],
        &IndexMode::ReadOnly,
    )
    .await?
    .require_success(&["check-ref-format", "--branch", name])?;

    if checkout {
        with_repo_lock(target, || async {
            git(target, &["switch", "-c", name], &IndexMode::user())
                .await?
                .require_success(&["switch", "-c", name])?;
            Ok(())
        })
        .await?;
    } else {
        git(target, &["branch", name], &IndexMode::ReadOnly)
            .await?
            .require_success(&["branch", name])?;
    }

    list_branches(target)
        .await?
        .into_iter()
        .find(|branch| branch.name == name)
        .ok_or_else(|| GitError::Parse(format!("已创建分支但无法读取: {name}")))
}
