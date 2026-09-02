use super::repo::head_oid;
use super::runner::{git, with_repo_lock, GitError, GitTarget, IndexMode};

pub(crate) async fn stage_paths(target: &GitTarget, paths: &[String]) -> Result<(), GitError> {
    require_paths(paths)?;
    with_repo_lock(target, || async {
        let mut args = vec!["add", "-A", "--"];
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        args.extend(refs.iter().copied());
        git(target, &args, &IndexMode::user())
            .await?
            .require_success(&args)?;
        Ok(())
    })
    .await
}

pub(crate) async fn unstage_paths(target: &GitTarget, paths: &[String]) -> Result<(), GitError> {
    require_paths(paths)?;
    with_repo_lock(target, || async {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        if head_oid(target).await?.is_none() {
            let mut args = vec!["rm", "--cached", "-q", "--"];
            args.extend(refs.iter().copied());
            git(target, &args, &IndexMode::user())
                .await?
                .require_success(&args)?;
        } else {
            let mut args = vec!["restore", "--staged", "--"];
            args.extend(refs.iter().copied());
            git(target, &args, &IndexMode::user())
                .await?
                .require_success(&args)?;
        }
        Ok(())
    })
    .await
}

pub(crate) async fn restore_paths(target: &GitTarget, paths: &[String]) -> Result<(), GitError> {
    require_paths(paths)?;
    let mut args = vec!["restore", "--worktree", "--"];
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    args.extend(refs.iter().copied());
    git(target, &args, &IndexMode::ReadOnly)
        .await?
        .require_success(&args)?;
    Ok(())
}

fn require_paths(paths: &[String]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Err(GitError::Parse("paths 不能为空".to_string()));
    }
    for path in paths {
        super::runner::assert_safe_rel_path(path)?;
    }
    Ok(())
}
