use std::fs;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use serde_json::json;

use super::dispatch::{execute_tool, PermissionRequester, ToolCtx};
use super::file_access::*;
use super::local::LocalWorkspace;
use super::permission::{
    NativePermissionDecision, NativeToolRiskKind, PermissionRule, RuleEffect, RuleScope,
};
use crate::native::permission_rules::{add_rules, delete_rule, load_effective_rules, shared_rules};

fn context(root: &std::path::Path) -> ToolCtx {
    ToolCtx::new(LocalWorkspace::new(root.to_path_buf()))
}

fn reply_with(decision: NativePermissionDecision, count: Arc<AtomicUsize>) -> PermissionRequester {
    Arc::new(move |prompt, reply| {
        assert!(prompt.file_access.is_some());
        count.fetch_add(1, Ordering::SeqCst);
        reply.send(decision).unwrap();
    })
}

#[tokio::test]
async fn external_reads_and_searches_ask_per_call_without_leaking_to_children() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("notes.txt"), "external content\n").unwrap();
    let mut ctx = context(root.path());
    let count = Arc::new(AtomicUsize::new(0));
    ctx.request_permission = Some(reply_with(
        NativePermissionDecision::AllowOnce,
        count.clone(),
    ));
    let read = json!({"file_path":outside.path().join("notes.txt")}).to_string();
    let (first, second) = tokio::join!(
        execute_tool(&ctx, "Read", &read),
        execute_tool(&ctx, "Read", &read)
    );
    assert!(first.unwrap().contains("external content"));
    assert!(second.unwrap().contains("external content"));
    execute_tool(&ctx.fork_for_child(), "Read", &read)
        .await
        .unwrap();
    execute_tool(
        &ctx,
        "Glob",
        &json!({"path":outside.path(),"pattern":"*.txt"}).to_string(),
    )
    .await
    .unwrap();
    let grep = execute_tool(
        &ctx,
        "Grep",
        &json!({"path":outside.path(),"pattern":"external"}).to_string(),
    )
    .await
    .unwrap();
    assert!(grep.contains("external content"));
    assert_eq!(count.load(Ordering::SeqCst), 5);
    assert!(ctx.workspace.authorized_paths.is_empty());
    assert!(ctx.permission_rules_snapshot().is_empty());
    assert!(!ctx.allow_all_high_risk.load(Ordering::SeqCst));
}

#[tokio::test]
async fn yolo_allows_all_six_file_tools_outside_workspace() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("file.txt");
    let moved = outside.path().join("nested/moved.txt");
    fs::write(&file, "original\n").unwrap();
    let mut ctx = context(root.path());
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    ctx.request_permission = Some(Arc::new(|_, _| panic!("yolo must not ask")));
    execute_tool(&ctx, "Read", &json!({"file_path":file}).to_string())
        .await
        .unwrap();
    execute_tool(
        &ctx,
        "Glob",
        &json!({"path":outside.path(),"pattern":"*.txt"}).to_string(),
    )
    .await
    .unwrap();
    execute_tool(
        &ctx,
        "Grep",
        &json!({"path":outside.path(),"pattern":"original"}).to_string(),
    )
    .await
    .unwrap();
    execute_tool(
        &ctx,
        "Write",
        &json!({"file_path":file,"content":"changed\n"}).to_string(),
    )
    .await
    .unwrap();
    execute_tool(
        &ctx,
        "Edit",
        &json!({"file_path":file,"old_string":"changed","new_string":"edited"}).to_string(),
    )
    .await
    .unwrap();
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n*** Move to: {}\n@@\n-edited\n+moved\n*** End Patch",
        file.display(),
        moved.display()
    );
    execute_tool(&ctx, "ApplyPatch", &json!({"patch":patch}).to_string())
        .await
        .unwrap();
    assert!(!file.exists());
    assert_eq!(fs::read_to_string(&moved).unwrap(), "moved\n");
    let patch = format!(
        "*** Begin Patch\n*** Delete File: {}\n*** Add File: {}\n+created\n*** End Patch",
        moved.display(),
        file.display()
    );
    execute_tool(&ctx, "ApplyPatch", &json!({"patch":patch}).to_string())
        .await
        .unwrap();
    assert!(!moved.exists());
    assert_eq!(fs::read_to_string(file).unwrap(), "created\n");
}

#[tokio::test]
async fn whitelist_survives_reload_and_keeps_file_directory_and_write_boundaries() {
    let config = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("file.txt");
    let neighbor = outside.path().join("neighbor.txt");
    fs::write(&file, "data").unwrap();
    fs::write(&neighbor, "data").unwrap();
    let mut ctx = context(root.path());
    let args = json!({"file_path":file}).to_string();
    let access = collect_file_access(&ctx, "Read", &args).await.unwrap();
    let selection = vec![FileAccessSelection {
        path: access.paths[0].path.clone(),
        directory: false,
    }];
    let rules = access
        .rules_for_selection(&selection, RuleScope::Workspace)
        .unwrap();
    let saved = add_rules(config.path(), Some(root.path()), RuleEffect::Allow, rules).unwrap();
    ctx.permission_rules = shared_rules(load_effective_rules(config.path(), Some(root.path())));
    let count = Arc::new(AtomicUsize::new(0));
    ctx.request_permission = Some(reply_with(NativePermissionDecision::Deny, count.clone()));
    execute_tool(&ctx, "Read", &args).await.unwrap();
    assert!(
        execute_tool(&ctx, "Read", &json!({"file_path":neighbor}).to_string())
            .await
            .is_err()
    );
    assert!(execute_tool(
        &ctx,
        "Write",
        &json!({"file_path":file,"content":"blocked"}).to_string()
    )
    .await
    .is_err());
    assert_eq!(fs::read_to_string(&file).unwrap(), "data");
    let directory = vec![FileAccessSelection {
        path: selection[0].path.clone(),
        directory: true,
    }];
    add_rules(
        config.path(),
        Some(root.path()),
        RuleEffect::Allow,
        access
            .rules_for_selection(&directory, RuleScope::Workspace)
            .unwrap(),
    )
    .unwrap();
    *ctx.permission_rules.write().unwrap() = load_effective_rules(config.path(), Some(root.path()));
    execute_tool(&ctx, "Read", &json!({"file_path":neighbor}).to_string())
        .await
        .unwrap();
    execute_tool(
        &ctx,
        "Grep",
        &json!({"path":outside.path(),"pattern":"data"}).to_string(),
    )
    .await
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    for rule in ctx.permission_rules_snapshot().allow {
        delete_rule(config.path(), Some(root.path()), &rule.id).unwrap();
    }
    *ctx.permission_rules.write().unwrap() = load_effective_rules(config.path(), Some(root.path()));
    assert!(execute_tool(&ctx, "Read", &args).await.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert!(!saved[0].id.is_empty());
}

#[tokio::test]
async fn multi_target_patch_is_approved_together_and_denial_never_partially_writes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let source = root.path().join("source.txt");
    let dest = outside.path().join("destination.txt");
    fs::write(&source, "original\n").unwrap();
    let mut ctx = context(root.path());
    ctx.request_permission = Some(Arc::new(move |prompt, reply| {
        assert_eq!(prompt.kind, NativeToolRiskKind::ExternalPath);
        let access = prompt.file_access.unwrap();
        assert_eq!(access.paths.len(), 3);
        assert!(access
            .paths
            .iter()
            .any(|path| path.operation == "move_destination"));
        reply.send(NativePermissionDecision::Deny).unwrap();
    }));
    let patch = format!("*** Begin Patch\n*** Add File: first.txt\n+first\n*** Update File: source.txt\n*** Move to: {}\n@@\n-original\n+updated\n*** End Patch",dest.display());
    let args = json!({"patch":patch}).to_string();
    assert!(execute_tool(&ctx, "ApplyPatch", &args).await.is_err());
    assert!(!root.path().join("first.txt").exists());
    assert!(!dest.exists());
    assert_eq!(fs::read_to_string(source).unwrap(), "original\n");
    let deny: PermissionRule =
        serde_json::from_value(json!({"capability":"edit","pattern":dest,"source":"path"}))
            .unwrap();
    ctx.permission_rules
        .write()
        .unwrap()
        .push(RuleEffect::Deny, deny);
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    assert!(execute_tool(&ctx, "ApplyPatch", &args)
        .await
        .unwrap_err()
        .contains("权限规则拒绝"));
    assert!(!root.path().join("first.txt").exists());
}

#[tokio::test]
async fn legacy_allow_does_not_grant_external_access_and_timeouts_do_not_write() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("new.txt");
    let mut ctx = context(root.path());
    let rule =
        serde_json::from_value(json!({"capability":"edit","pattern":"Write","source":"tool_name"}))
            .unwrap();
    ctx.permission_rules
        .write()
        .unwrap()
        .push(RuleEffect::Allow, rule);
    let args = json!({"file_path":file,"content":"blocked"}).to_string();
    assert!(execute_tool(&ctx, "Write", &args)
        .await
        .unwrap_err()
        .contains("确认通道"));
    ctx.permission_timeout = std::time::Duration::from_millis(10);
    let held = Arc::new(Mutex::new(Vec::new()));
    let sink = held.clone();
    ctx.request_permission = Some(Arc::new(move |_, reply| sink.lock().unwrap().push(reply)));
    assert!(execute_tool(&ctx, "Write", &args)
        .await
        .unwrap_err()
        .contains("超时"));
    assert!(!file.exists());
    let cancel = ctx.cancel.clone();
    ctx.request_permission = Some(Arc::new(move |_, reply| {
        cancel.cancel();
        let _ = reply.send(NativePermissionDecision::AllowOnce);
    }));
    assert!(execute_tool(&ctx, "Write", &args).await.is_err());
    assert!(!file.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn changed_symlink_after_confirmation_requires_a_new_approval() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let first = outside.path().join("first.txt");
    let second = outside.path().join("second.txt");
    let link = root.path().join("link");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    symlink(&first, &link).unwrap();
    let mut ctx = context(root.path());
    let count = Arc::new(AtomicUsize::new(0));
    let sink = count.clone();
    let link_clone = link.clone();
    ctx.request_permission = Some(Arc::new(move |_, reply| {
        let decision = if sink.fetch_add(1, Ordering::SeqCst) == 0 {
            fs::remove_file(&link_clone).unwrap();
            symlink(&second, &link_clone).unwrap();
            NativePermissionDecision::AllowOnce
        } else {
            NativePermissionDecision::Deny
        };
        reply.send(decision).unwrap();
    }));
    assert!(
        execute_tool(&ctx, "Read", &json!({"file_path":link}).to_string())
            .await
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn absolute_paths_and_symlink_aliases_still_match_workspace_deny_rules() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("private")).unwrap();
    fs::write(root.path().join("private/data.txt"), "inside").unwrap();
    fs::write(outside.path().join("data.txt"), "outside").unwrap();
    symlink(
        outside.path().join("data.txt"),
        root.path().join("private/link.txt"),
    )
    .unwrap();
    let ctx = context(root.path());
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    ctx.permission_rules.write().unwrap().push(
        RuleEffect::Deny,
        serde_json::from_value(json!({"capability":"read","source":"path","pattern":"private/**"}))
            .unwrap(),
    );
    for file in ["data.txt", "link.txt"] {
        let error = execute_tool(
            &ctx,
            "Read",
            &json!({"file_path":root.path().join("private").join(file)}).to_string(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("权限规则拒绝"), "{error}");
    }
}

#[test]
fn ssh_rules_are_bound_to_connection_identity_and_component_boundaries() {
    let target = PermissionTarget::Ssh {
        config_id: "ssh-1".into(),
        host: "host-one".into(),
        port: 22,
        username: "user".into(),
    };
    let prompt = FileAccessPrompt {
        target: target.clone(),
        paths: vec![FileAccessPath {
            path: "/data/file.txt".into(),
            requested_path: "/data/file.txt".into(),
            capability: super::contract::PermissionCapability::Read,
            scope: PathAccessScope::Exact,
            operation: "read".into(),
            outside_workspace: true,
        }],
    };
    let mut selections = vec![FileAccessSelection {
        path: "/data/file.txt".into(),
        directory: true,
    }];
    let rules = prompt
        .rules_for_selection(&selections, RuleScope::Workspace)
        .unwrap();
    assert!(external_rule_matches(&rules[0], &target, &prompt.paths[0]));
    assert!(!external_rule_matches(
        &rules[0],
        &PermissionTarget::Local,
        &prompt.paths[0]
    ));
    let mut other = target.clone();
    if let PermissionTarget::Ssh { host, .. } = &mut other {
        *host = "host-two".into();
    }
    assert!(!external_rule_matches(&rules[0], &other, &prompt.paths[0]));
    let mut neighbor = prompt.paths[0].clone();
    neighbor.path = "/data-extra/file.txt".into();
    assert!(!external_rule_matches(&rules[0], &target, &neighbor));
    selections[0].path = "/".into();
    assert!(prompt
        .rules_for_selection(&selections, RuleScope::Global)
        .is_err());
}
