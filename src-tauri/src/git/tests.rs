use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::app::ssh::client::{AuthMaterial, ConnectParams};
use crate::app::ssh::known_hosts::{HostTrustBroker, KnownHostsPolicy};
use crate::app::ssh::test_server::{TestServerOpts, TestSshServer};
use crate::app::ssh::SshPool;
use crate::db::test_support::setup_migrated_pool;

use super::checkpoint::{
    create_checkpoint, delete_checkpoints_for_session, list_checkpoints, preview_restore,
    prune_expired_checkpoints, restore_checkpoint,
};
use super::commit::{checkout_branch, commit_changes, create_branch, list_branches, push_branch};
use super::diff::{get_file_diff, get_numstat, GitFileDiffScope, GitNumstatScope};
use super::repo::{list_repo_files, load_repo_info};
use super::runner::{fixture_git, git, GitTarget, IndexMode, ScratchIndex};
use super::stage::{restore_paths, stage_paths, unstage_paths};
use super::status::get_status;

struct RepoEnv {
    dir: tempfile::TempDir,
    target: GitTarget,
    _server: Option<TestSshServer>,
}

static SSH_TARGET_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn temp_known_hosts() -> PathBuf {
    let dir = tempfile::tempdir().expect("known_hosts dir");
    let path = dir.path().join("known_hosts");
    std::mem::forget(dir);
    path
}

async fn init_files(dir: &Path) {
    let target = GitTarget::Local(dir.to_path_buf());
    fixture_git(&target, &["init"]).await.expect("git init");
    fixture_git(&target, &["symbolic-ref", "HEAD", "refs/heads/main"])
        .await
        .expect("main");
    fixture_git(&target, &["config", "user.name", "tester"])
        .await
        .expect("name");
    fixture_git(&target, &["config", "user.email", "tester@local"])
        .await
        .expect("email");
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    fixture_git(&target, &["add", "README.md"])
        .await
        .expect("add");
    fixture_git(&target, &["commit", "-m", "init"])
        .await
        .expect("commit");
}

async fn local_env() -> RepoEnv {
    let dir = tempfile::tempdir().expect("repo");
    init_files(dir.path()).await;
    let target = GitTarget::Local(dir.path().to_path_buf());
    RepoEnv {
        dir,
        target,
        _server: None,
    }
}

async fn ssh_env() -> RepoEnv {
    let dir = tempfile::tempdir().expect("repo");
    init_files(dir.path()).await;
    let server = TestSshServer::start(TestServerOpts {
        real_shell: true,
        ..TestServerOpts::default()
    })
    .await;
    let pool = SshPool::new(
        Arc::new(HostTrustBroker::new(Duration::from_secs(5))),
        Duration::from_secs(600),
    );
    let target = GitTarget::Ssh {
        pool,
        params: ConnectParams {
            ssh_config_id: "git-test".to_string(),
            name: "git-test".to_string(),
            host: "127.0.0.1".to_string(),
            port: server.port,
            username: "tester".to_string(),
            auth: AuthMaterial::Password("secret".to_string()),
            policy: KnownHostsPolicy::Off,
            known_hosts_path: temp_known_hosts(),
        },
        repo_path: dir.path().to_string_lossy().into_owned(),
    };
    RepoEnv {
        dir,
        target,
        _server: Some(server),
    }
}

async fn seed_session() -> (sqlx::SqlitePool, String, String) {
    let pool = setup_migrated_pool().await;
    sqlx::query(
        "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-git', 'git', 'local')",
    )
    .execute(&pool)
    .await
    .expect("workspace");
    sqlx::query(
        "INSERT INTO agent_sessions (id, workspace_id, status) VALUES ('sess-git', 'ws-git', 'pending')",
    )
    .execute(&pool)
    .await
    .expect("session");
    (pool, "ws-git".to_string(), "sess-git".to_string())
}

fn index_bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(".git/index")).expect("read index")
}

async fn visible_log(target: &GitTarget) -> String {
    git(target, &["log", "--format=%H"], &IndexMode::ReadOnly)
        .await
        .expect("log")
        .stdout_lossy()
}

async fn branch_list(target: &GitTarget) -> String {
    git(target, &["branch", "--list"], &IndexMode::ReadOnly)
        .await
        .expect("branch")
        .stdout_lossy()
}

async fn run_on_targets<F, Fut>(mut test: F)
where
    F: FnMut(GitTarget, PathBuf) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let local = local_env().await;
    test(local.target.clone(), local.dir.path().to_path_buf()).await;
    let _ssh_guard = SSH_TARGET_TEST_LOCK.lock().await;
    let ssh = ssh_env().await;
    test(ssh.target.clone(), ssh.dir.path().to_path_buf()).await;
}

#[tokio::test]
async fn list_repo_files_filters_and_includes_untracked() {
    let env = local_env().await;
    std::fs::write(env.dir.path().join("notes.md"), "n\n").unwrap();
    std::fs::create_dir_all(env.dir.path().join("src")).unwrap();
    std::fs::write(env.dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let all = list_repo_files(&env.target, None, None)
        .await
        .expect("list");
    assert!(all.contains(&"README.md".to_string()));
    assert!(all.contains(&"notes.md".to_string()));
    assert!(all.contains(&"src/main.rs".to_string()));
    let filtered = list_repo_files(&env.target, Some("main"), Some(10))
        .await
        .expect("filter");
    assert_eq!(filtered, vec!["src/main.rs".to_string()]);
}

#[tokio::test]
async fn repo_status_stage_commit_push_and_branch() {
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        let info = load_repo_info(&target, "ws-git").await.expect("info");
        assert!(
            info.toplevel
                .ends_with(dir.file_name().unwrap().to_str().unwrap())
                || info.toplevel.contains(dir.to_string_lossy().as_ref())
        );
        assert_eq!(info.branch.as_deref(), Some("main"));

        std::fs::write(dir.join("a.txt"), "A\n").unwrap();
        stage_paths(&target, &["a.txt".to_string()])
            .await
            .expect("stage");
        let status = get_status(&target, Some("all")).await.expect("status");
        assert!(status
            .entries
            .iter()
            .any(|entry| entry.path == "a.txt" && entry.xy.starts_with('A')));

        unstage_paths(&target, &["a.txt".to_string()])
            .await
            .expect("unstage");
        let status = get_status(&target, Some("all")).await.expect("status");
        assert!(status
            .entries
            .iter()
            .any(|entry| entry.path == "a.txt" && entry.kind == "untracked"));

        stage_paths(&target, &["a.txt".to_string()])
            .await
            .expect("stage again");
        let commit = commit_changes(&target, "add a", None)
            .await
            .expect("commit");
        assert_eq!(commit.oid.len(), 40);

        std::fs::write(dir.join("a.txt"), "A2\n").unwrap();
        let diff = get_file_diff(&target, "a.txt", &GitFileDiffScope::Worktree, None)
            .await
            .expect("diff");
        assert!(diff.patch.contains("A2") || !diff.patch.is_empty());
        restore_paths(&target, &["a.txt".to_string()])
            .await
            .expect("restore");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "A\n");

        let created = create_branch(&target, "feature", true)
            .await
            .expect("branch");
        assert_eq!(created.name, "feature");
        assert!(created.is_current);
        let switched = checkout_branch(&target, "main").await.expect("switch");
        assert_eq!(switched.name, "main");
        assert!(switched.is_current);
        let branches = list_branches(&target).await.expect("list");
        assert!(branches.iter().any(|branch| branch.name == "main"));
        assert!(branches
            .iter()
            .any(|branch| branch.name == "feature" && !branch.is_current));

        let remote_dir = tempfile::tempdir().expect("remote");
        let remote = GitTarget::Local(remote_dir.path().to_path_buf());
        fixture_git(&remote, &["init", "--bare"])
            .await
            .expect("bare");
        fixture_git(
            &target,
            &[
                "remote",
                "add",
                "origin",
                remote_dir.path().to_str().unwrap(),
            ],
        )
        .await
        .expect("remote add");
        push_branch(&target, Some("origin"), Some("feature"), true)
            .await
            .expect("push");
    })
    .await;
}

#[tokio::test]
async fn checkpoint_does_not_touch_user_index() {
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        let (pool, workspace_id, session_id) = seed_session().await;
        std::fs::write(dir.join("b.txt"), "B_user\n").unwrap();
        fixture_git(&target, &["add", "b.txt"])
            .await
            .expect("user add");
        std::fs::write(dir.join("a.txt"), "A_ai\n").unwrap();
        std::fs::write(dir.join("c.txt"), "C_ai\n").unwrap();
        let before = index_bytes(dir);

        create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("会话开始"),
            Some("session_start"),
        )
        .await
        .expect("checkpoint");

        assert_eq!(before, index_bytes(dir));
        let status = get_status(&target, Some("all")).await.expect("status");
        assert!(status.entries.iter().any(|entry| entry.path == "b.txt"));
        assert!(status.entries.iter().any(|entry| entry.path == "a.txt"));
        assert!(status
            .entries
            .iter()
            .any(|entry| entry.path == "c.txt" && entry.kind == "untracked"));
        if matches!(target, GitTarget::Ssh { .. }) {
            let home = std::env::var("HOME").unwrap_or_default();
            let tmp = PathBuf::from(home).join(".noxcode/tmp-index");
            if tmp.is_dir() {
                let leftovers: Vec<_> = std::fs::read_dir(&tmp)
                    .unwrap()
                    .flatten()
                    .filter(|entry| {
                        entry.metadata().ok().is_some_and(|meta| {
                            meta.modified().ok().is_some_and(|time| {
                                time.elapsed()
                                    .map(|spent| spent.as_secs() < 5)
                                    .unwrap_or(false)
                            })
                        })
                    })
                    .collect();
                assert!(
                    leftovers.is_empty(),
                    "SSH 临时索引应被 cleanup: {leftovers:?}"
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn special_filenames_and_renames() {
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        std::fs::write(dir.join("hello world.txt"), "space\n").unwrap();
        std::fs::write(dir.join("中文.txt"), "zh\n").unwrap();
        fixture_git(&target, &["add", "hello world.txt", "中文.txt"])
            .await
            .expect("add special");
        fixture_git(&target, &["commit", "-m", "special"])
            .await
            .expect("commit special");
        fixture_git(&target, &["mv", "hello world.txt", "hello  world.txt"])
            .await
            .expect("mv");
        let numstat = get_numstat(&target, &GitNumstatScope::Staged)
            .await
            .expect("numstat");
        assert!(
            numstat.iter().any(|entry| {
                entry.path == "hello  world.txt"
                    && entry.orig_path.as_deref() == Some("hello world.txt")
            }),
            "expected rename, got {numstat:?}"
        );
        let status = get_status(&target, Some("all")).await.expect("status");
        assert!(status.entries.iter().any(|entry| entry.path == "中文.txt"
            || entry.path == "hello  world.txt"
            || entry.orig_path.as_deref() == Some("hello world.txt")));

        #[cfg(unix)]
        {
            std::fs::write(dir.join("line\nbreak.txt"), "nl\n").unwrap();
            let status = get_status(&target, Some("all")).await.expect("nl status");
            assert!(
                status
                    .entries
                    .iter()
                    .any(|entry| entry.path == "line\nbreak.txt"),
                "newline filename should parse, got {status:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn checkpoint_restore_three_classes_and_visibility() {
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        let (pool, workspace_id, session_id) = seed_session().await;
        std::fs::write(dir.join("keep.txt"), "old\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "gone\n").unwrap();
        fixture_git(&target, &["add", "keep.txt", "gone.txt"])
            .await
            .expect("add");
        fixture_git(&target, &["commit", "-m", "base"])
            .await
            .expect("commit");

        let checkpoint = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("基线"),
            Some("manual"),
        )
        .await
        .expect("cp");

        std::fs::write(dir.join("keep.txt"), "new\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::write(dir.join("fresh.txt"), "fresh\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored.log\n").unwrap();
        std::fs::write(dir.join("ignored.log"), "secret\n").unwrap();

        let preview = preview_restore(&pool, &target, &checkpoint.id)
            .await
            .expect("preview");
        assert!(preview.blocked_reason.is_none(), "{preview:?}");
        assert!(preview.will_overwrite.iter().any(|path| path == "keep.txt"));
        assert!(preview.will_recreate.iter().any(|path| path == "gone.txt"));
        assert!(preview
            .wont_be_touched
            .iter()
            .any(|path| path == "fresh.txt"));

        let restored = restore_checkpoint(&pool, &target, &workspace_id, &checkpoint.id, &[])
            .await
            .expect("restore");
        assert_eq!(
            std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
            "old\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("gone.txt")).unwrap(),
            "gone\n"
        );
        assert!(dir.join("fresh.txt").exists());
        assert!(restored.pre_restore_checkpoint.kind == "auto_pre_restore");

        let listed = list_checkpoints(&pool, &target, &workspace_id, &session_id)
            .await
            .expect("list");
        assert!(listed.iter().all(|item| item.ref_valid));

        let log = visible_log(&target).await;
        assert!(!log.contains(&checkpoint.commit_oid));
        let branches = branch_list(&target).await;
        assert!(!branches.contains("noxcode"));

        restore_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &checkpoint.id,
            &["fresh.txt".to_string()],
        )
        .await
        .expect("delete fresh");
        assert!(!dir.join("fresh.txt").exists());

        std::fs::write(dir.join("ignored.log"), "secret2\n").unwrap();
        restore_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &checkpoint.id,
            &["ignored.log".to_string()],
        )
        .await
        .expect("try delete ignored");
        assert!(
            dir.join("ignored.log").exists(),
            "gitignore 文件即使勾选也不能删"
        );

        let again = restore_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &restored.pre_restore_checkpoint.id,
            &[],
        )
        .await
        .expect("restore pre");
        assert!(!again.restored.is_empty() || dir.join("keep.txt").exists());
    })
    .await;
}

#[tokio::test]
async fn prune_removes_only_expired_after_tool_call_checkpoints() {
    run_on_targets(|target, _dir| async move {
        let (pool, workspace_id, session_id) = seed_session().await;
        let expired = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("工具后"),
            Some("after_tool_call"),
        )
        .await
        .expect("after tool checkpoint");
        let manual = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("手动"),
            Some("manual"),
        )
        .await
        .expect("manual checkpoint");

        sqlx::query(
            "UPDATE agent_sessions SET status = 'exited', ended_at = datetime('now', '-10 days') WHERE id = $1",
        )
        .bind(&session_id)
        .execute(&pool)
        .await
        .expect("end session");

        assert!(
            prune_expired_checkpoints(&pool, &target, &workspace_id, 0)
                .await
                .expect("retention disabled")
                .is_empty()
        );

        let deleted = prune_expired_checkpoints(&pool, &target, &workspace_id, 7)
            .await
            .expect("prune");
        assert_eq!(deleted, vec![expired.ref_name.clone()]);

        let listed = list_checkpoints(&pool, &target, &workspace_id, &session_id)
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, manual.id);
        assert!(listed[0].ref_valid);

        let expired_ref = git(
            &target,
            &["rev-parse", "--verify", &expired.ref_name],
            &IndexMode::ReadOnly,
        )
        .await
        .expect("verify expired ref");
        assert!(!expired_ref.success());
    })
    .await;
}

#[tokio::test]
async fn scratch_index_from_head_isolated_and_supports_unborn_head() {
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        std::fs::write(dir.join("staged.txt"), "staged\n").expect("write staged");
        fixture_git(&target, &["add", "staged.txt"])
            .await
            .expect("stage user file");
        std::fs::write(dir.join("untracked.txt"), "untracked\n").expect("write untracked");
        let before = index_bytes(dir);

        let scratch = ScratchIndex::from_head(&target).await.expect("from HEAD");
        let mode = IndexMode::Scratch(scratch.clone());
        let files = git(&target, &["ls-files", "--stage"], &mode)
            .await
            .expect("list scratch index")
            .require_success(&["ls-files", "--stage"])
            .expect("ls-files success")
            .stdout_lossy();
        assert!(files.contains("README.md"));
        assert!(!files.contains("staged.txt"));
        assert!(!files.contains("untracked.txt"));
        scratch.cleanup().await.expect("cleanup scratch");
        assert_eq!(before, index_bytes(dir));

        fixture_git(&target, &["symbolic-ref", "HEAD", "refs/heads/unborn-test"])
            .await
            .expect("set unborn HEAD");
        let unborn = ScratchIndex::from_head(&target)
            .await
            .expect("unborn index");
        let unborn_mode = IndexMode::Scratch(unborn.clone());
        let unborn_files = git(&target, &["ls-files", "--stage"], &unborn_mode)
            .await
            .expect("list unborn index")
            .require_success(&["ls-files", "--stage"])
            .expect("unborn ls-files success")
            .stdout_lossy();
        assert!(unborn_files.is_empty(), "{unborn_files}");
        unborn.cleanup().await.expect("cleanup unborn scratch");
        assert_eq!(before, index_bytes(dir));
    })
    .await;
}

#[tokio::test]
async fn restore_rejects_merge_and_missing_ref() {
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        let (pool, workspace_id, session_id) = seed_session().await;
        std::fs::write(dir.join("file.txt"), "base\n").unwrap();
        fixture_git(&target, &["add", "file.txt"])
            .await
            .expect("add");
        fixture_git(&target, &["commit", "-m", "base"])
            .await
            .expect("c1");
        let checkpoint = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("before merge"),
            Some("manual"),
        )
        .await
        .expect("cp");

        fixture_git(&target, &["switch", "-c", "other"])
            .await
            .expect("other");
        std::fs::write(dir.join("file.txt"), "other\n").unwrap();
        fixture_git(&target, &["add", "file.txt"])
            .await
            .expect("add o");
        fixture_git(&target, &["commit", "-m", "other"])
            .await
            .expect("c2");
        fixture_git(&target, &["switch", "main"])
            .await
            .expect("main");
        std::fs::write(dir.join("file.txt"), "main\n").unwrap();
        fixture_git(&target, &["add", "file.txt"])
            .await
            .expect("add m");
        fixture_git(&target, &["commit", "-m", "main"])
            .await
            .expect("c3");
        let merge = git(
            &target,
            &["merge", "--no-commit", "other"],
            &IndexMode::user(),
        )
        .await
        .expect("merge invoke");
        assert!(
            !merge.success()
                || dir.join(".git/MERGE_HEAD").exists()
                || git(
                    &target,
                    &["rev-parse", "-q", "--verify", "MERGE_HEAD"],
                    &IndexMode::ReadOnly
                )
                .await
                .expect("merge head")
                .success()
        );

        if git(
            &target,
            &["rev-parse", "-q", "--verify", "MERGE_HEAD"],
            &IndexMode::ReadOnly,
        )
        .await
        .expect("check merge")
        .success()
        {
            let preview = preview_restore(&pool, &target, &checkpoint.id)
                .await
                .expect("preview merge");
            assert!(preview.blocked_reason.is_some());
            let err = restore_checkpoint(&pool, &target, &workspace_id, &checkpoint.id, &[])
                .await
                .expect_err("restore during merge");
            assert!(err.to_string().contains("merge") || err.to_string().contains("中止"));
            let _ = git(&target, &["merge", "--abort"], &IndexMode::user()).await;
        }

        git(
            &target,
            &["update-ref", "-d", &checkpoint.ref_name],
            &IndexMode::ReadOnly,
        )
        .await
        .expect("delete ref");
        let listed = list_checkpoints(&pool, &target, &workspace_id, &session_id)
            .await
            .expect("list");
        assert!(listed.iter().any(|item| !item.ref_valid));
        let preview = preview_restore(&pool, &target, &checkpoint.id)
            .await
            .expect("invalid");
        assert!(preview.blocked_reason.is_some());
    })
    .await;
}

#[tokio::test]
async fn delete_session_cleans_refs_and_objects() {
    run_on_targets(|target, _dir| async move {
        let (pool, workspace_id, session_id) = seed_session().await;
        let checkpoint = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("to delete"),
            Some("manual"),
        )
        .await
        .expect("cp");
        delete_checkpoints_for_session(&pool, &target, &session_id)
            .await
            .expect("delete");
        let leftover = git(
            &target,
            &["for-each-ref", "--format=%(refname)", "refs/noxcode"],
            &IndexMode::ReadOnly,
        )
        .await
        .expect("refs")
        .stdout_lossy();
        assert!(leftover.trim().is_empty(), "{leftover}");
        git(
            &target,
            &["-c", "gc.reflogExpire=now", "gc", "--prune=now"],
            &IndexMode::ReadOnly,
        )
        .await
        .expect("gc");
        let exists = git(
            &target,
            &["cat-file", "-e", &checkpoint.commit_oid],
            &IndexMode::ReadOnly,
        )
        .await
        .expect("cat-file");
        assert!(!exists.success(), "checkpoint 对象应被 gc 回收");
    })
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn readonly_file_reports_failed_and_keeps_prerestore() {
    if nix_is_root() {
        return;
    }
    run_on_targets(|target, dir| async move {
        let dir = dir.as_path();
        let (pool, workspace_id, session_id) = seed_session().await;
        std::fs::write(dir.join("locked.txt"), "old\n").unwrap();
        fixture_git(&target, &["add", "locked.txt"])
            .await
            .expect("add");
        fixture_git(&target, &["commit", "-m", "lock"])
            .await
            .expect("commit");
        let checkpoint = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_id,
            Some("before lock"),
            Some("manual"),
        )
        .await
        .expect("cp");
        std::fs::write(dir.join("locked.txt"), "new\n").unwrap();
        let mut perms = std::fs::metadata(dir).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o555);
        std::fs::set_permissions(dir, perms).unwrap();
        let result = restore_checkpoint(&pool, &target, &workspace_id, &checkpoint.id, &[]).await;
        let mut writable = std::fs::metadata(dir).unwrap().permissions();
        writable.set_mode(0o755);
        std::fs::set_permissions(dir, writable).unwrap();
        let result = result.expect("restore should return even if some files fail");
        assert!(
            !result.failed.is_empty()
                || std::fs::read_to_string(dir.join("locked.txt")).unwrap() == "old\n",
            "要么部分失败，要么成功还原: {result:?}"
        );
        assert_eq!(result.pre_restore_checkpoint.kind, "auto_pre_restore");
    })
    .await;
}

fn nix_is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}
