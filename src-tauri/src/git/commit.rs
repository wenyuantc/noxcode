use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::repo::{head_oid, in_progress_operation};
use super::runner::{git, git_with, with_repo_lock, GitError, GitRunOptions, GitTarget, IndexMode};
use super::status::get_status;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPullResult {
    pub updated: bool,
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

pub(crate) async fn pull_branch(target: &GitTarget) -> Result<GitPullResult, GitError> {
    with_repo_lock(target, || async {
        let status = get_status(target, Some("no")).await?;
        if let Some(operation) = in_progress_operation(target).await? {
            return Err(GitError::Blocked(format!(
                "仓库有未完成的 {operation} 操作，请先完成或中止后再拉取"
            )));
        }
        if status.entries.iter().any(|entry| entry.kind == "unmerged") {
            return Err(GitError::Blocked(
                "存在未解决的冲突，请先处理后再拉取".to_string(),
            ));
        }
        if status.branch.head.is_none() {
            return Err(GitError::Blocked(
                "当前处于游离 HEAD，请先切换到分支后再拉取".to_string(),
            ));
        }
        let before = status
            .branch
            .oid
            .ok_or_else(|| GitError::Blocked("当前分支尚无提交，无法安全快进拉取".to_string()))?;
        if status.branch.upstream.is_none() {
            return Err(GitError::Blocked(
                "当前分支未配置上游，请先设置跟踪分支后再拉取".to_string(),
            ));
        }

        let args = ["pull", "--ff-only", "--no-rebase", "--no-autostash"];
        let output = git_with(
            target,
            &args,
            &IndexMode::user(),
            GitRunOptions {
                timeout: Some(Duration::from_secs(300)),
                ..GitRunOptions::default()
            },
        )
        .await?;
        output.require_success(&args)?;
        let after = head_oid(target)
            .await?
            .ok_or_else(|| GitError::Parse("拉取完成，但无法读取当前提交".to_string()))?;
        Ok(GitPullResult {
            updated: before != after,
            message: [output.stdout_lossy(), output.stderr_lossy()]
                .into_iter()
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string(),
        })
    })
    .await
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

pub(crate) async fn checkout_branch(target: &GitTarget, name: &str) -> Result<GitBranch, GitError> {
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

    with_repo_lock(target, || async {
        git(target, &["switch", name], &IndexMode::user())
            .await?
            .require_success(&["switch", name])?;
        Ok(())
    })
    .await?;

    list_branches(target)
        .await?
        .into_iter()
        .find(|branch| branch.name == name && branch.is_current)
        .ok_or_else(|| GitError::Parse(format!("已切换分支但无法读取: {name}")))
}
