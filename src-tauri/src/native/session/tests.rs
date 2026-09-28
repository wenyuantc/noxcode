use super::{
    format_native_diagnostics, is_cancelled_run_error, is_mcp_error_status, native_startup_banner,
    next_loop_step, should_announce_session_startup, NativeLoopAction, NativeLoopEvent,
};
use crate::native::agent::compact::{BudgetSnapshot, ContextWindow};
use crate::native::agent::r#loop::AgentDiagnosticsSnapshot;
use crate::native::input_queue::NativeInputQueue;
use crate::native::manager::NativeConfigurationRequest;

#[tokio::test]
async fn ssh_session_preflight_requires_verified_password_and_returns_config() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query(
            "INSERT INTO ssh_configs (id, name, host, username, auth_type) VALUES ('ssh-preflight', 'test', 'example.test', 'tester', 'password')",
        )
        .execute(&pool)
        .await
        .unwrap();
    let context = crate::engine::context::ExecutionContext {
        execution_target: "ssh".to_string(),
        working_dir: Some("/repo".to_string()),
        ssh_config_id: Some("ssh-preflight".to_string()),
        target_host_label: Some("tester@example.test:22".to_string()),
    };
    for status in [None, Some("failed"), Some("unknown")] {
        sqlx::query("UPDATE ssh_configs SET password_probe_status = $1, last_check_status = 'passed' WHERE id = 'ssh-preflight'")
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
        let error = super::load_session_ssh_config(&pool, &context)
            .await
            .unwrap_err();
        assert!(error.contains("密码"), "{error}");
        assert!(error.contains("设置"), "{error}");
    }
    for status in ["passed", "available"] {
        sqlx::query("UPDATE ssh_configs SET password_probe_status = $1 WHERE id = 'ssh-preflight'")
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        let config = super::load_session_ssh_config(&pool, &context)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config.id, "ssh-preflight");
        assert_eq!(config.password_probe_status.as_deref(), Some(status));
    }
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(session_count, 0);
}

#[tokio::test]
async fn ssh_session_preflight_keeps_local_and_key_sessions_available() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    let mut context = crate::engine::context::ExecutionContext::local_default();
    assert!(super::load_session_ssh_config(&pool, &context)
        .await
        .unwrap()
        .is_none());
    context.execution_target = "ssh".to_string();
    assert!(super::load_session_ssh_config(&pool, &context)
        .await
        .unwrap_err()
        .contains("ssh_config_id"));
    context.ssh_config_id = Some("ssh-key-preflight".to_string());
    assert!(super::load_session_ssh_config(&pool, &context)
        .await
        .unwrap_err()
        .contains("不存在"));
    sqlx::query(
            "INSERT INTO ssh_configs (id, name, host, username, auth_type, private_key_path) VALUES ('ssh-key-preflight', 'test', 'example.test', 'tester', 'key', '/test/id_ed25519')",
        )
        .execute(&pool)
        .await
        .unwrap();
    let config = super::load_session_ssh_config(&pool, &context)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.auth_type, "key");
    assert!(config.password_probe_status.is_none());
}

#[test]
fn plan_implementation_keeps_current_effort_unless_requested() {
    assert_eq!(
        super::resolve_plan_implementation_effort(Some("high"), None).as_deref(),
        Some("high")
    );
    assert_eq!(
        super::resolve_plan_implementation_effort(Some("high"), Some("  ")).as_deref(),
        Some("high")
    );
    assert_eq!(
        super::resolve_plan_implementation_effort(Some("high"), Some("max")).as_deref(),
        Some("max")
    );
    assert!(super::plan_implementation_unchanged(
        "ch",
        "model",
        Some("high"),
        "ch",
        "model",
        Some("high")
    ));
    assert!(!super::plan_implementation_unchanged(
        "ch",
        "model",
        Some("high"),
        "ch",
        "model",
        Some("max")
    ));
    assert!(!super::plan_implementation_unchanged(
        "ch",
        "old",
        Some("high"),
        "ch",
        "new",
        Some("high")
    ));
}

#[test]
fn live_followup_rejects_silently_ignored_configuration_changes() {
    let runtime = crate::db::models::NativeSessionRuntime {
        ai_channel_id: "channel".to_string(),
        model: "model".to_string(),
        reasoning_effort: Some("high".to_string()),
        permission_mode: "default".to_string(),
        plan_mode: false,
        worktree_path: None,
        sandbox_active: false,
    };
    let base = serde_json::json!({"ai_channel_id":"channel", "workspace_id":"ws", "prompt":"next"});
    let input = serde_json::from_value(base.clone()).unwrap();
    assert!(super::validate_live_configuration(&runtime, &input).is_ok());
    for (key, value) in [
        ("ai_channel_id", serde_json::json!("other-channel")),
        ("model", serde_json::json!("other-model")),
        ("reasoning_effort", serde_json::json!("low")),
        ("permission_mode", serde_json::json!("yolo")),
        ("plan_mode", serde_json::json!(true)),
    ] {
        let mut changed = base.clone();
        changed[key] = value;
        let input = serde_json::from_value(changed).unwrap();
        assert!(
            super::validate_live_configuration(&runtime, &input).is_err(),
            "{key}"
        );
    }
}

#[tokio::test]
async fn recv_idle_wait_prefers_configuration_over_queued_input() {
    let (_followup_tx, mut followup_rx) = tokio::sync::mpsc::channel(8);
    let (config_tx, mut config_rx) = tokio::sync::mpsc::channel(8);
    let queue = NativeInputQueue::new("sess-config");
    queue
        .enqueue("already queued", Vec::new())
        .expect("enqueue");
    let (reply, _done) = tokio::sync::oneshot::channel();
    config_tx
        .send(NativeConfigurationRequest {
            request_id: "req-1".into(),
            ai_channel_id: "ch-new".into(),
            model: "model-new".into(),
            reasoning_effort: None,
            reply,
        })
        .await
        .unwrap();
    let wait = super::recv_idle_wait(
        &mut followup_rx,
        &mut config_rx,
        &queue,
        &crate::native::tools::CancelFlag::new(),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await;
    match wait {
        super::NativeIdleWait::Configuration(Some(request)) => {
            assert_eq!(request.request_id, "req-1");
            assert_eq!(request.model, "model-new");
        }
        _ => panic!("expected configuration first"),
    }
    assert_eq!(queue.snapshot().items.len(), 1);
    assert_eq!(queue.snapshot().items[0].text, "already queued");
}

#[tokio::test]
async fn update_agent_session_channel_keeps_the_same_session_id() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type, created_at, updated_at) VALUES ('ws-ch', 'ws', 'local', 't', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-old', 'old', 'openai', 'http://x'), ('ch-new', 'new', 'openai', 'http://y')",
        )
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
            "INSERT INTO agent_sessions (id, ai_channel_id, workspace_id, session_kind, status, started_at, created_at) VALUES ('sess-ch', 'ch-old', 'ws-ch', 'execution', 'running', 't', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
    super::update_agent_session_channel(&pool, "sess-ch", "ch-new")
        .await
        .unwrap();
    let channel: String =
        sqlx::query_scalar("SELECT ai_channel_id FROM agent_sessions WHERE id = 'sess-ch'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(channel, "ch-new");
}

#[tokio::test]
async fn pending_plan_survives_until_explicitly_cleared() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query(
            "INSERT INTO agent_sessions (id, session_kind, status, started_at, created_at) VALUES ('sess-plan', 'plan', 'running', 't', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();

    super::save_pending_plan_with(&pool, "sess-plan", "req-1", "## 目标\n落库计划")
        .await
        .unwrap();
    let raw: String =
        sqlx::query_scalar("SELECT pending_plan_json FROM agent_sessions WHERE id = 'sess-plan'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let saved: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(saved["request_id"], "req-1");
    assert_eq!(saved["plan"], "## 目标\n落库计划");
    assert!(saved["created_at"]
        .as_str()
        .is_some_and(|it| !it.is_empty()));

    super::clear_pending_plan_with(&pool, "sess-plan")
        .await
        .unwrap();
    let cleared: Option<String> =
        sqlx::query_scalar("SELECT pending_plan_json FROM agent_sessions WHERE id = 'sess-plan'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(cleared.is_none());
}

#[tokio::test]
async fn workspace_hooks_auto_trust_in_yolo() {
    use crate::native::tools::dispatch::ToolCtx;
    use crate::native::tools::local::LocalWorkspace;
    use crate::native::tools::permission::NativePermissionDecision;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let root = tempfile::tempdir().unwrap();
    let hooks = vec![crate::db::models::NativeHook::shell(
        "workspace",
        "session_start",
        "*",
        "printf trusted",
        5,
        true,
    )];
    let mut ctx = ToolCtx::new(LocalWorkspace::new(root.path().to_path_buf()));
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    ctx.auto_approve_opaque_bash = true;
    ctx.hooks = vec![crate::db::models::NativeHook::shell(
        "auto-allow",
        "permission_request",
        "*",
        "printf '{\"decision\":\"allow\"}'",
        5,
        true,
    )];
    assert!(super::approve_workspace_hooks(&ctx, &hooks).await);
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    ctx.request_permission = Some(Arc::new({
        let seen = seen.clone();
        move |prompt, tx| {
            assert_eq!(prompt.tool_name, "WorkspaceHooks");
            seen.fetch_add(1, Ordering::SeqCst);
            tx.send(NativePermissionDecision::Deny).unwrap();
        }
    }));
    assert!(super::approve_workspace_hooks(&ctx, &hooks).await);
    assert_eq!(requests.load(Ordering::SeqCst), 0);

    ctx.allow_all_high_risk.store(false, Ordering::SeqCst);
    ctx.request_permission = None;
    assert!(!super::approve_workspace_hooks(&ctx, &hooks).await);
    ctx.request_permission = Some(Arc::new(move |prompt, tx| {
        assert_eq!(prompt.tool_name, "WorkspaceHooks");
        assert!(prompt.suggested_rule.is_none());
        seen.fetch_add(1, Ordering::SeqCst);
        tx.send(NativePermissionDecision::Deny).unwrap();
    }));
    assert!(!super::approve_workspace_hooks(&ctx, &hooks).await);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    ctx.request_permission = Some(Arc::new(|_, tx| {
        tx.send(NativePermissionDecision::AllowOnce).unwrap();
    }));
    assert!(super::approve_workspace_hooks(&ctx, &hooks).await);
    ctx.cancel.cancel();
    assert!(!super::approve_workspace_hooks(&ctx, &hooks).await);
}

#[tokio::test]
async fn workspace_hook_trust_timeout_expires_request() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::native::tools::dispatch::ToolCtx::new(
        crate::native::tools::local::LocalWorkspace::new(root.path().to_path_buf()),
    );
    ctx.permission_timeout = std::time::Duration::from_millis(10);
    let pending = Arc::new(std::sync::Mutex::new(None));
    let pending_request = pending.clone();
    ctx.request_permission = Some(Arc::new(move |_, tx| {
        *pending_request.lock().unwrap() = Some(tx);
    }));
    let expired = Arc::new(AtomicBool::new(false));
    let expired_handler = expired.clone();
    ctx.expire_permission = Some(Arc::new(move |_| {
        let expired = expired_handler.clone();
        tauri::async_runtime::spawn(async move {
            expired.store(true, Ordering::SeqCst);
        })
    }));
    assert!(!super::approve_workspace_hooks(&ctx, &[]).await);
    assert!(expired.load(Ordering::SeqCst));
}

#[tokio::test]
async fn graceful_finish_waits_for_completion_and_blocks_racing_input() {
    use crate::native::manager::{
        NativeAgentManager, NativeFollowup, NativeLiveSession, NativeSessionInfo,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let manager = Arc::new(tokio::sync::Mutex::new(NativeAgentManager::new()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let cancel = crate::native::tools::CancelFlag::new();
    let cancelled = cancel.clone();
    let manager_run = manager.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let finished_run = finished.clone();
    let (closing_tx, closing_rx) = tokio::sync::oneshot::channel();
    let join = tokio::spawn(async move {
        assert!(matches!(rx.recv().await, Some(NativeFollowup::Finish)));
        assert!(!cancelled.is_cancelled());
        let _ = closing_tx.send(());
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        finished_run.store(true, Ordering::SeqCst);
        manager_run.lock().await.remove_session("finish-test");
    });
    manager.lock().await.add_session(NativeLiveSession {
        info: NativeSessionInfo {
            profile_id: String::new(),
            channel_id: "ch".to_string(),
            workspace_id: Some("ws".to_string()),
            session_kind: "execution".to_string(),
            session_record_id: "finish-test".to_string(),
        },
        runtime: None,
        plan_mode: std::sync::Arc::default(),
        allow_session_commands: std::sync::Arc::default(),
        background: None,
        processes: None,
        closing: false,
        cancel,
        followup_tx: tx,
        config_tx: crate::native::manager::unused_config_tx(),
        input_queue: Arc::new(NativeInputQueue::new("sess-1")),
        join,
        allow_all_high_risk: Arc::new(AtomicBool::new(false)),
        working: Arc::new(AtomicBool::new(false)),
        pending_compactions: Arc::default(),
        permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
        workspace_root: None,
        pending_permission: Default::default(),
        pending_question: Default::default(),
        pending_plan_approval: Default::default(),
        live_model: None,
        transcript_model: None,
    });
    let manager_finish = manager.clone();
    let finish =
        tokio::spawn(async move { super::finish_live_input(&manager_finish, "finish-test").await });
    closing_rx.await.unwrap();
    assert!(!finished.load(Ordering::SeqCst));
    assert!(
        super::enqueue_live_input(&manager, "finish-test", "race", None, None, None)
            .await
            .is_err()
    );
    finish.await.unwrap().unwrap();
    assert!(finished.load(Ordering::SeqCst));
    assert!(manager.lock().await.get_session("finish-test").is_none());
    super::finish_live_input(&manager, "finish-test")
        .await
        .unwrap();
}

#[test]
fn cancelled_run_error_is_not_a_failure() {
    assert!(is_cancelled_run_error("已取消"));
    assert!(!is_cancelled_run_error("模型超时"));
}

#[test]
fn permission_event_includes_suggested_rule() {
    use crate::native::manager::PermissionRequest;
    use crate::native::tools::contract::{PatternSource, PermissionCapability};
    use crate::native::tools::permission::{NativeToolRiskKind, PermissionRuleSuggestion};

    let request = PermissionRequest {
        origin: None,
        request_id: "r1".to_string(),
        profile_id: "p1".to_string(),
        workspace_id: Some("w1".to_string()),
        session_kind: "execution".to_string(),
        tool_name: "Bash".to_string(),
        kind: NativeToolRiskKind::Opaque,
        summary: "rm -rf /tmp/x".to_string(),
        file_access: None,
        allow_once_only: false,
        remote: false,
        mcp_server_id: None,
        suggested_rule: Some(PermissionRuleSuggestion {
            capability: PermissionCapability::Bash,
            pattern: "rm".to_string(),
            source: PatternSource::Command,
            plan_bash: None,
        }),
    };
    let event = super::permission_event("sess-1", &request);
    assert_eq!(event.suggested_rule.as_ref().unwrap().pattern, "rm");
    let json = serde_json::to_value(&event).expect("json");
    assert_eq!(json["suggested_rule"]["pattern"], "rm");
    assert_eq!(json["suggested_rule"]["capability"], "bash");
}

#[test]
fn persist_stdout_message_wraps_tool_events() {
    use crate::db::models::{NativeToolEvent, NativeToolPhase};

    let tool = NativeToolEvent {
        phase: NativeToolPhase::Start,
        call_id: "c1".to_string(),
        name: "Read".to_string(),
        title: "读取 a.ts".to_string(),
        args_summary: "a.ts".to_string(),
        ok: None,
        duration_ms: None,
        result_preview: None,
        subagent_tag: None,
        mcp_server: None,
        mcp_tool: None,
        image_names: Vec::new(),
    };
    let raw = super::persist_stdout_message("[读取] a.ts", Some(&tool), None, None);
    let value: serde_json::Value = serde_json::from_str(&raw).expect("envelope");
    assert_eq!(value["nox"], 1);
    assert_eq!(value["line"], "[读取] a.ts");
    assert_eq!(value["tool"]["call_id"], "c1");
    assert_eq!(
        super::persist_stdout_message("[读取] a.ts", None, None, None),
        "[读取] a.ts"
    );
    let images = [crate::db::models::NativeToolImage {
        name: "a.png".to_string(),
        mime_type: "image/png".to_string(),
        data_url: "data:image/png;base64,QQ==".to_string(),
        attachment_id: None,
    }];
    let with_images =
        super::persist_stdout_message("[USER_INPUT] 看图", None, Some(images.as_slice()), None);
    let image_value: serde_json::Value = serde_json::from_str(&with_images).expect("envelope");
    assert_eq!(image_value["line"], "[USER_INPUT] 看图");
    assert_eq!(image_value["images"][0]["name"], "a.png");
}

#[test]
fn mcp_screenshot_history_stores_reference_not_base64() {
    use crate::db::models::{NativeToolEvent, NativeToolImage, NativeToolPhase};
    let event = NativeToolEvent {
        phase: NativeToolPhase::Result,
        call_id: "shot".into(),
        name: "mcp__playwright__browser_take_screenshot".into(),
        title: String::new(),
        args_summary: String::new(),
        ok: Some(true),
        duration_ms: None,
        result_preview: None,
        subagent_tag: None,
        mcp_server: Some("playwright".into()),
        mcp_tool: Some("browser_take_screenshot".into()),
        image_names: vec!["shot.png".into()],
    };
    let images = [NativeToolImage {
        name: "shot.png".into(),
        mime_type: "image/png".into(),
        data_url: "data:image/png;base64,AAAA".into(),
        attachment_id: Some("attachment-1".into()),
    }];
    let persisted = super::persist_stdout_message("screenshot", Some(&event), Some(&images), None);
    assert!(!persisted.contains("base64"));
    let value: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(value["images"][0]["attachment_id"], "attachment-1");
    assert_eq!(value["images"][0]["data_url"], "");
}

#[test]
fn persist_stdout_message_retains_exact_assistant_fragment_identity_and_bytes() {
    let fragment = crate::db::models::NativeAssistantFragment {
        chain_id: "answer".into(),
        part: 1,
        subagent_tag: None,
    };
    let text = "  1;\n```\n";
    let persisted = super::persist_stdout_message(text, None, None, Some(&fragment));
    let value: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(value["line"], text);
    assert_eq!(value["assistant"]["chain_id"], "answer");
    assert_eq!(value["assistant"]["part"], 1);
}

#[test]
fn native_one_shot_text_requires_non_empty_assistant() {
    let mut message = crate::native::model::types::Message::assistant_text("  ok  ");
    assert_eq!(super::native_one_shot_text(&message).as_deref(), Ok("ok"));
    message.content = "   ".to_string();
    assert_eq!(
        super::native_one_shot_text(&message).unwrap_err(),
        "内置 Agent 未返回可用内容"
    );
}

#[test]
fn native_one_shot_text_uses_plan_shaped_reasoning() {
    let mut message = crate::native::model::types::Message::assistant_text("");
    message.reasoning_content =
        "{\"markdown\":\"# 计划\",\"steps\":[{\"title\":\"a\"}]}".to_string();
    assert!(super::native_one_shot_text(&message)
        .expect("usable reasoning")
        .contains("计划"));
}

#[test]
fn native_one_shot_text_rejects_plain_reasoning() {
    let mut message = crate::native::model::types::Message::assistant_text("");
    message.reasoning_content = "先分析任务边界再给出步骤".to_string();
    let error = super::native_one_shot_text(&message).unwrap_err();
    assert!(error.contains("思考内容"));
    assert!(error.contains("没有正文"));
}

#[test]
fn runtime_effort_clamps_to_channel_allowed_levels() {
    let mut config = crate::native::model_catalog::apply_catalog_defaults("gpt-5.6-luna");
    config.thinking_enabled = Some(true);
    config.thinking_levels = Some(vec!["low".to_string(), "high".to_string()]);
    config.thinking_level = Some("high".to_string());
    crate::native::model_catalog::fill_from_catalog(&mut config);
    let resolved = super::resolve_run_model_config(std::slice::from_ref(&config), "gpt-5.6-luna");
    assert_eq!(
        crate::native::model_catalog::resolve_runtime_reasoning_effort(&resolved, Some("max"))
            .as_deref(),
        Some("high")
    );
}

#[test]
fn native_startup_banner_includes_model_and_channel() {
    assert_eq!(
        native_startup_banner("CRS", "codex", "gpt-5.6-luna", Some("high"), true),
        "[内置 Agent] 启动会话 渠道=CRS 协议=codex model=gpt-5.6-luna effort=high thinking=on"
    );
    assert_eq!(
            native_startup_banner("DeepSeek", "openai", "deepseek-v4-flash", None, false),
            "[内置 Agent] 启动会话 渠道=DeepSeek 协议=openai model=deepseek-v4-flash effort=默认 thinking=off"
        );
}

#[test]
fn resume_does_not_announce_startup_or_restore_banners() {
    assert!(should_announce_session_startup(None));
    assert!(should_announce_session_startup(Some("")));
    assert!(should_announce_session_startup(Some("   ")));
    assert!(!should_announce_session_startup(Some("sess-1")));
    assert!(is_mcp_error_status(
        "[MCP] 无法连接 files：timeout（已跳过，不回退到其他位置）"
    ));
    assert!(is_mcp_error_status("[MCP] 握手失败 git：boom（已跳过）"));
    assert!(is_mcp_error_status("[MCP] 读取配置失败：bad json"));
    assert!(is_mcp_error_status("[MCP] 没有成功连接的服务器"));
    assert!(!is_mcp_error_status("[MCP] 未启用服务器"));
    assert!(!is_mcp_error_status("[MCP] 已连接：a"));
    assert!(!is_mcp_error_status("[续聊] 已恢复上一会话 2 条上下文"));
}

#[test]
fn native_diagnostics_describe_budget_and_context_window() {
    let budget = BudgetSnapshot {
        limit: 200_000,
        spent: 12_345,
        remaining: 187_655,
        active_reservations: 256,
    };
    let context = ContextWindow {
        generation: 2,
        token_limit: 16_000,
        compactions: 1,
        resets: 1,
        threshold_percent: 85,
    };
    let details =
        format_native_diagnostics(&budget, &context, &AgentDiagnosticsSnapshot::default());
    assert!(details.contains("已用 12345"));
    assert!(details.contains("上下文窗口代数 2"));
    assert!(details.contains("压缩 1 次"));
    assert!(details.contains("重置 1 次"));
}

#[test]
fn next_loop_step_covers_plan_followup_and_exit() {
    assert_eq!(
        next_loop_step(true, NativeLoopEvent::TurnFinished),
        NativeLoopAction::WaitFollowup
    );
    assert_eq!(
        next_loop_step(false, NativeLoopEvent::TurnFinished),
        NativeLoopAction::Exit
    );
    assert_eq!(
        next_loop_step(true, NativeLoopEvent::FollowupInput),
        NativeLoopAction::RunFollowup
    );
    assert_eq!(
        next_loop_step(true, NativeLoopEvent::FollowupFinish),
        NativeLoopAction::Exit
    );
    assert_eq!(
        next_loop_step(true, NativeLoopEvent::Cancelled),
        NativeLoopAction::Exit
    );
    assert_eq!(
        next_loop_step(true, NativeLoopEvent::Error),
        NativeLoopAction::Exit
    );
}

#[test]
fn user_turn_count_counts_user_messages() {
    let messages = vec![
        crate::native::model::types::Message::system("s"),
        crate::native::model::types::Message::user("a"),
        crate::native::model::types::Message::assistant_text("b"),
        crate::native::model::types::Message::user("c"),
    ];
    assert_eq!(super::user_turn_count(&messages), 2);
}

#[test]
fn session_title_truncates_unicode_scalars() {
    assert_eq!(super::session_title("  hello  ").as_deref(), Some("hello"));
    assert_eq!(super::session_title("   "), None);
    let chinese = "一二三四五六七八九十";
    let thirty = format!("{chinese}{chinese}{chinese}");
    let over = format!("{thirty}超出");
    assert_eq!(thirty.chars().count(), 30);
    assert!(over.len() > 30);
    assert_eq!(
        super::session_title(&over).as_deref(),
        Some(thirty.as_str())
    );
}

#[tokio::test]
async fn insert_resume_inherits_source_title() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query("INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-t', 'ws', 'local')")
        .execute(&pool)
        .await
        .expect("ws");
    sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-t', 'ch', 'openai', 'http://x')",
        )
        .execute(&pool)
        .await
        .expect("ch");
    let prompt = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十超出";
    let source = super::insert_agent_session(
        &pool,
        &crate::app::shared::new_id(),
        "ch-t",
        "ws-t",
        "/tmp",
        "local",
        None,
        None,
        "execution",
        None,
        prompt,
    )
    .await
    .expect("source");
    let source_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM agent_sessions WHERE id = $1")
            .bind(&source)
            .fetch_one(&pool)
            .await
            .expect("source title");
    let resumed = super::insert_agent_session(
        &pool,
        &crate::app::shared::new_id(),
        "ch-t",
        "ws-t",
        "/tmp",
        "local",
        None,
        None,
        "execution",
        Some(&source),
        "继续",
    )
    .await
    .expect("resume");
    let resume_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM agent_sessions WHERE id = $1")
            .bind(&resumed)
            .fetch_one(&pool)
            .await
            .expect("resume title");
    assert_eq!(
        source_title.as_deref(),
        Some("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十")
    );
    assert_eq!(resume_title, source_title);
}

#[tokio::test]
async fn enqueue_live_input_queues_without_steering_the_active_turn() {
    use crate::native::manager::{NativeAgentManager, NativeLiveSession};
    use crate::native::tools::CancelFlag;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let mut manager = NativeAgentManager::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    manager.add_session(NativeLiveSession {
        info: crate::native::manager::NativeSessionInfo {
            profile_id: String::new(),
            channel_id: "ch-1".to_string(),
            workspace_id: Some("ws-1".to_string()),
            session_kind: "execution".to_string(),
            session_record_id: "sess-1".to_string(),
        },
        runtime: None,
        plan_mode: std::sync::Arc::default(),
        allow_session_commands: std::sync::Arc::default(),
        background: None,
        processes: None,
        closing: false,
        cancel: CancelFlag::new(),
        followup_tx: tx,
        config_tx: crate::native::manager::unused_config_tx(),
        input_queue: Arc::new(NativeInputQueue::new("sess-1")),
        join: tokio::spawn(async {}),
        allow_all_high_risk: Arc::new(AtomicBool::new(false)),
        working: Arc::new(AtomicBool::new(false)),
        pending_compactions: Arc::default(),
        pending_permission: VecDeque::new(),
        pending_question: VecDeque::new(),
        permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
        workspace_root: None,
        pending_plan_approval: VecDeque::new(),
        live_model: None,
        transcript_model: None,
    });
    let manager = tokio::sync::Mutex::new(manager);

    let (info, snapshot) =
        super::enqueue_live_input(&manager, "sess-1", "  下一条  ", None, None, None)
            .await
            .expect("enqueue")
            .expect("live");
    assert_eq!(info.session_record_id, "sess-1");
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert_eq!(snapshot.items[0].text, "下一条");
    let queue = manager
        .lock()
        .await
        .get_session("sess-1")
        .unwrap()
        .input_queue
        .clone();
    let item = queue
        .recv(&CancelFlag::new(), &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(item.text, "下一条");
    assert!(item.images.is_empty());
    assert!(
        super::enqueue_live_input(&manager, "missing", "x", None, None, None)
            .await
            .expect("missing")
            .is_none()
    );
}

#[tokio::test]
async fn enqueue_live_input_allows_images_without_text() {
    use crate::native::manager::{NativeAgentManager, NativeLiveSession};
    use crate::native::tools::CancelFlag;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("noxcode-followup-img-{stamp}.png"));
    std::fs::write(&path, b"\x89PNG\r\n").expect("png");

    let mut manager = NativeAgentManager::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    manager.add_session(NativeLiveSession {
        info: crate::native::manager::NativeSessionInfo {
            profile_id: String::new(),
            channel_id: "ch-1".to_string(),
            workspace_id: Some("ws-1".to_string()),
            session_kind: "execution".to_string(),
            session_record_id: "sess-1".to_string(),
        },
        runtime: None,
        plan_mode: std::sync::Arc::default(),
        allow_session_commands: std::sync::Arc::default(),
        background: None,
        processes: None,
        closing: false,
        cancel: CancelFlag::new(),
        followup_tx: tx,
        config_tx: crate::native::manager::unused_config_tx(),
        input_queue: Arc::new(NativeInputQueue::new("sess-1")),
        join: tokio::spawn(async {}),
        allow_all_high_risk: Arc::new(AtomicBool::new(false)),
        working: Arc::new(AtomicBool::new(false)),
        pending_compactions: Arc::default(),
        pending_permission: VecDeque::new(),
        pending_question: VecDeque::new(),
        permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
        workspace_root: None,
        pending_plan_approval: VecDeque::new(),
        live_model: None,
        transcript_model: None,
    });
    let manager = tokio::sync::Mutex::new(manager);
    let paths = [path.to_string_lossy().into_owned()];
    super::enqueue_live_input(
        &manager,
        "sess-1",
        "   ",
        Some(paths.as_slice()),
        None,
        None,
    )
    .await
    .expect("enqueue")
    .expect("live");
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    let queue = manager
        .lock()
        .await
        .get_session("sess-1")
        .unwrap()
        .input_queue
        .clone();
    let item = queue
        .recv(&CancelFlag::new(), &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(item.text.is_empty());
    assert_eq!(item.images.len(), 1);
    assert_eq!(
        item.images[0].name,
        path.file_name().unwrap().to_string_lossy()
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn reactivate_session_reuses_row_and_preserves_identity() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-t', 'ws', 'local'), ('ws-other', 'other', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
    sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-t', 'ch', 'openai', 'http://x'), ('ch-new', 'ch2', 'openai', 'http://y')",
        )
        .execute(&pool)
        .await
        .expect("ch");
    sqlx::query(
        r#"
            INSERT INTO agent_sessions (
                id, ai_channel_id, workspace_id, working_dir, execution_target,
                session_kind, status, started_at, ended_at, exit_code, created_at,
                title, pinned, input_tokens, output_tokens, total_tokens
            ) VALUES (
                'sess-keep', 'ch-t', 'ws-t', '/old', 'local',
                'execution', 'exited', '2026-01-01 00:00:00', '2026-01-02 00:00:00', 0,
                '2026-01-01 00:00:00', '原标题', 1, 11, 22, 33
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("seed");

    let mismatched = super::reactivate_agent_session(
        &pool,
        "sess-keep",
        "ws-other",
        "ch-new",
        "/new",
        "local",
        None,
        None,
        "execution",
    )
    .await;
    assert!(mismatched
        .expect_err("workspace mismatch")
        .contains("会话不属于当前工作区"));

    let missing = super::reactivate_agent_session(
        &pool,
        "missing",
        "ws-t",
        "ch-new",
        "/new",
        "local",
        None,
        None,
        "execution",
    )
    .await;
    assert!(missing
        .expect_err("missing")
        .contains("会话不存在: missing"));

    let reactivated = super::reactivate_agent_session(
        &pool,
        "sess-keep",
        "ws-t",
        "ch-new",
        "/new",
        "local",
        None,
        None,
        "plan",
    )
    .await
    .expect("reactivate");
    assert_eq!(reactivated, "sess-keep");

    let row = sqlx::query_as::<_, crate::db::models::AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = 'sess-keep'",
    )
    .fetch_one(&pool)
    .await
    .expect("row");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM agent_sessions")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
    assert_eq!(row.id, "sess-keep");
    assert_eq!(row.title.as_deref(), Some("原标题"));
    assert_eq!(row.pinned, 1);
    assert_eq!(row.created_at, "2026-01-01 00:00:00");
    assert_eq!(row.input_tokens, Some(11));
    assert_eq!(row.output_tokens, Some(22));
    assert_eq!(row.total_tokens, Some(33));
    assert_eq!(row.status, "running");
    assert_eq!(row.ai_channel_id.as_deref(), Some("ch-new"));
    assert_eq!(row.working_dir.as_deref(), Some("/new"));
    assert_eq!(row.session_kind, "plan");
    assert!(row.ended_at.is_none());
    assert!(row.exit_code.is_none());
    assert_ne!(row.started_at, "2026-01-01 00:00:00");
}

#[tokio::test]
async fn reactivate_archived_session_is_rejected_without_changing_metadata() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query(
            "INSERT INTO agent_sessions (id, title, archived, pinned, status, working_dir) VALUES ('archived', '保留名称', 1, 1, 'exited', '/old')",
        )
        .execute(&pool)
        .await
        .unwrap();
    let error = super::reactivate_agent_session(
        &pool,
        "archived",
        "workspace",
        "channel",
        "/new",
        "local",
        None,
        None,
        "plan",
    )
    .await
    .unwrap_err();
    assert!(error.contains("已归档"));
    let row = sqlx::query_as::<_, crate::db::models::AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = 'archived'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.archived, 1);
    assert_eq!(row.pinned, 1);
    assert_eq!(row.title.as_deref(), Some("保留名称"));
    assert_eq!(row.working_dir.as_deref(), Some("/old"));
    assert_eq!(row.status, "exited");
}

#[tokio::test]
async fn reactivate_stale_running_session_keeps_same_id() {
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query("INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-t', 'ws', 'local')")
        .execute(&pool)
        .await
        .expect("ws");
    sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-t', 'ch', 'openai', 'http://x')",
        )
        .execute(&pool)
        .await
        .expect("ch");
    sqlx::query(
        r#"
            INSERT INTO agent_sessions (
                id, ai_channel_id, workspace_id, working_dir, execution_target,
                session_kind, status, started_at, created_at, title
            ) VALUES (
                'sess-stale', 'ch-t', 'ws-t', '/old', 'local',
                'execution', 'running', '2026-01-01 00:00:00', '2026-01-01 00:00:00', '卡住'
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("seed");

    let reactivated = super::reactivate_agent_session(
        &pool,
        "sess-stale",
        "ws-t",
        "ch-t",
        "/old",
        "local",
        None,
        None,
        "execution",
    )
    .await
    .expect("reactivate stale");
    assert_eq!(reactivated, "sess-stale");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM agent_sessions")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
    let status: String =
        sqlx::query_scalar("SELECT status FROM agent_sessions WHERE id = 'sess-stale'")
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "running");
}

#[test]
fn extract_session_subagents_correctly_groups_and_resolves_status() {
    let events = vec![
        (
            "evt-1".to_string(),
            Some("[子 Agent 1(general) - 构建代码] 启动（general）".to_string()),
            "2026-09-17 10:00:00".to_string(),
        ),
        (
            "evt-2".to_string(),
            Some(r#"{"nox":1,"line":"[子 Agent 1(general) - 构建代码] 正在编译"}"#.to_string()),
            "2026-09-17 10:00:05".to_string(),
        ),
        (
            "evt-3".to_string(),
            Some("[子 Agent 1(general) - 构建代码] 结束 成功".to_string()),
            "2026-09-17 10:00:10".to_string(),
        ),
        (
            "evt-4".to_string(),
            Some("[子 Agent 2(explore) - 探索架构] 启动（explore）".to_string()),
            "2026-09-17 10:00:15".to_string(),
        ),
        (
            "evt-5".to_string(),
            Some("[子 Agent 2(explore) - 探索架构] 结束 失败: 文件不存在".to_string()),
            "2026-09-17 10:00:20".to_string(),
        ),
        (
            "evt-6".to_string(),
            Some("[子 Agent 3(explore) - 运行中任务] 启动（explore）".to_string()),
            "2026-09-17 10:00:25".to_string(),
        ),
    ];

    let subagents = super::extract_session_subagents(&events, true);
    assert_eq!(subagents.len(), 3);

    assert_eq!(subagents[0].index, 1);
    assert_eq!(subagents[0].kind, "general");
    assert_eq!(subagents[0].description, "构建代码");
    assert_eq!(subagents[0].status, "completed");
    assert_eq!(subagents[0].duration_ms, Some(10_000));

    assert_eq!(subagents[1].index, 2);
    assert_eq!(subagents[1].kind, "explore");
    assert_eq!(subagents[1].status, "failed");
    assert_eq!(subagents[1].duration_ms, Some(5_000));
    assert!(subagents[1].error_message.is_some());

    assert_eq!(subagents[2].index, 3);
    assert_eq!(subagents[2].status, "running");
    assert_eq!(subagents[2].duration_ms, None);

    // When session is inactive, non-finished subagents become stopped
    let inactive = super::extract_session_subagents(&events, false);
    assert_eq!(inactive[2].status, "stopped");
}
#[tokio::test]
async fn steer_replay_uses_insertion_order_and_never_replays_interrupted_inputs() {
    use crate::native::steer::{SteerMailbox, SteerReceipt, SteerStatus};
    let pool = crate::db::test_support::setup_migrated_pool().await;
    sqlx::query("INSERT INTO agent_sessions (id, title) VALUES ('steer-history', 'steer')")
        .execute(&pool)
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let paths = vec!["/already/cleaned/image.png".into()];
    let mut receipt = SteerReceipt {
        session_record_id: "steer-history".into(),
        instance_id: "old-runtime".into(),
        turn_id: "old-turn".into(),
        input_id: id.clone(),
        text: "retain draft".into(),
        image_count: 1,
        payload_hash: SteerMailbox::payload_hash("retain draft", &paths),
        generation: 1,
        status: SteerStatus::Accepted,
        error: None,
    };
    for (event_id, status) in [
        ("z-accepted", SteerStatus::Accepted),
        ("a-applied", SteerStatus::Applied),
    ] {
        receipt.status = status;
        sqlx::query("INSERT INTO agent_session_events (id, session_id, event_type, message, created_at) VALUES ($1, 'steer-history', 'native_steer', $2, '2026-09-21 00:00:00')").bind(event_id).bind(serde_json::to_string(&receipt).unwrap()).execute(&pool).await.unwrap();
    }
    let loaded = super::load_steer_receipts(&pool, "steer-history", None)
        .await
        .unwrap();
    assert_eq!(loaded[0].status, SteerStatus::Applied);
    let retry = super::find_steer_receipt(
        &pool,
        "steer-history",
        &id,
        "old-turn",
        "retain draft",
        &paths,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(retry.status, SteerStatus::Applied);
    assert!(super::find_steer_receipt(
        &pool,
        "steer-history",
        &id,
        "old-turn",
        "changed",
        &paths,
        None
    )
    .await
    .is_err());
    receipt.input_id = uuid::Uuid::new_v4().to_string();
    receipt.status = SteerStatus::Accepted;
    super::insert_session_event(
        &pool,
        "steer-history",
        "native_steer",
        Some(&serde_json::to_string(&receipt).unwrap()),
    )
    .await
    .unwrap();
    let recovered = super::load_steer_receipts(&pool, "steer-history", None)
        .await
        .unwrap();
    assert_eq!(recovered[1].status, SteerStatus::Cancelled);
    assert_eq!(recovered[1].text, "retain draft");
    assert_eq!(recovered[1].image_count, 1);
    assert!(recovered[1]
        .error
        .as_deref()
        .unwrap()
        .contains("图片需要重新选择"));
    assert!(crate::app::sessions::get_agent_session_log_lines_with(
        &pool,
        "steer-history",
        None,
        None,
        None
    )
    .await
    .unwrap()
    .is_empty());
    assert_eq!(
        super::load_steer_receipts(&pool, "steer-history", None)
            .await
            .unwrap(),
        recovered
    );
}
struct SteerIpcFixture {
    pool: sqlx::SqlitePool,
    manager: std::sync::Arc<tokio::sync::Mutex<crate::native::manager::NativeAgentManager>>,
    mailbox: std::sync::Arc<crate::native::steer::SteerMailbox>,
    config: tempfile::TempDir,
    input: super::SteerSubmission,
}

impl SteerIpcFixture {
    async fn new() -> Self {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id, title) VALUES ('steer-ipc', 'steer')")
            .execute(&pool)
            .await
            .unwrap();
        let session = crate::native::manager::tests::live_session("steer-ipc");
        let mailbox = session.input_queue.steer.clone();
        let persisted_pool = pool.clone();
        mailbox.configure(
            std::sync::Arc::new(move |receipt| {
                let pool = persisted_pool.clone();
                Box::pin(async move { super::persist_steer_receipt(&pool, &receipt).await })
            }),
            std::sync::Arc::new(|_| {}),
        );
        mailbox.begin_turn().await;
        let turn = mailbox.snapshot().await.turn_id.unwrap();
        let mut manager = crate::native::manager::NativeAgentManager::new();
        manager.add_session(session);
        Self {
            pool,
            manager: std::sync::Arc::new(tokio::sync::Mutex::new(manager)),
            mailbox,
            config: tempfile::tempdir().unwrap(),
            input: super::SteerSubmission {
                session_record_id: "steer-ipc".into(),
                expected_turn_id: turn,
                input_id: uuid::Uuid::new_v4().to_string(),
                text: "change direction".into(),
                image_paths: vec![],
            },
        }
    }

    async fn submit(
        &self,
        input: &super::SteerSubmission,
    ) -> Result<crate::native::steer::SteerReceipt, crate::native::steer::SteerSubmissionError>
    {
        let root = crate::native::images::attachments_dir(self.config.path());
        super::submit_native_steer_with(
            &self.pool,
            &self.manager,
            input,
            |paths| async move { super::load_steer_submission_images(&root, &paths) },
            |_, _| {},
        )
        .await
    }

    async fn install_pending_plan(
        &self,
    ) -> tokio::sync::oneshot::Receiver<crate::native::tools::dispatch::PlanApprovalAnswer> {
        let pending = serde_json::json!({"request_id":"plan-request", "plan":"keep this plan", "created_at":"same timestamp"}).to_string();
        let approved = serde_json::json!({"request_id":"plan-request", "status":"saved", "body":"authorized body", "saved_hash":"retain-me"}).to_string();
        sqlx::query("UPDATE agent_sessions SET pending_plan_json = $1, approved_plan_json = $2 WHERE id = 'steer-ipc'")
                .bind(pending).bind(approved).execute(&self.pool).await.unwrap();
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.manager
            .lock()
            .await
            .enqueue_plan_approval(
                "steer-ipc",
                crate::native::manager::PendingPlanApproval {
                    request: crate::native::manager::PlanApprovalRequest {
                        origin: Some(self.mailbox.origin()),
                        request_id: "plan-request".into(),
                        profile_id: "".into(),
                        workspace_id: None,
                        session_kind: "plan".into(),
                        plan: "keep this plan".into(),
                    },
                    reply,
                },
            )
            .unwrap();
        receiver
    }

    async fn plan_json(&self) -> (Option<String>, Option<String>) {
        sqlx::query_as("SELECT pending_plan_json, approved_plan_json FROM agent_sessions WHERE id = 'steer-ipc'").fetch_one(&self.pool).await.unwrap()
    }
}

#[tokio::test]
async fn steer_ipc_concurrent_uuid_retries_do_not_reload_cleaned_images_or_reject_a_completed_turn()
{
    use crate::native::steer::SteerStatus;
    let mut fixture = SteerIpcFixture::new().await;
    let image = crate::native::images::stage_image_bytes(
        fixture.config.path(),
        "image.png",
        b"\x89PNG\r\nfixture",
    )
    .unwrap();
    fixture.input.image_paths = vec![image.to_string_lossy().into_owned()];
    let loaded = std::sync::Arc::new(tokio::sync::Notify::new());
    let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let first_pool = fixture.pool.clone();
    let first_manager = fixture.manager.clone();
    let first_input = fixture.input.clone();
    let first_root = crate::native::images::attachments_dir(fixture.config.path());
    let first_loaded = loaded.clone();
    let first_permit = permit.clone();
    let first = tokio::spawn(async move {
        super::submit_native_steer_with(
            &first_pool,
            &first_manager,
            &first_input,
            |paths| async move {
                let images = super::load_steer_submission_images(&first_root, &paths)?;
                first_loaded.notify_one();
                let _permit = first_permit.acquire().await.unwrap();
                Ok(images)
            },
            |_, _| {},
        )
        .await
    });
    loaded.notified().await;
    let second_pool = fixture.pool.clone();
    let second_manager = fixture.manager.clone();
    let second_input = fixture.input.clone();
    let second_root = crate::native::images::attachments_dir(fixture.config.path());
    let second_read = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let second_read_wait = second_read.clone();
    let second_loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted_loads = second_loads.clone();
    let second = tokio::spawn(async move {
        super::submit_native_steer_with(
            &second_pool,
            &second_manager,
            &second_input,
            |paths| async move {
                counted_loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _permit = second_read_wait.acquire().await.unwrap();
                super::load_steer_submission_images(&second_root, &paths)
            },
            |_, _| {},
        )
        .await
    });
    tokio::task::yield_now().await;
    permit.add_permits(1);
    let first_receipt = first.await.unwrap().unwrap();
    assert!(!image.exists(), "first durable acceptance cleans staging");
    let claimed = fixture.mailbox.take().await.unwrap();
    fixture
        .mailbox
        .finish_input(&claimed.receipt.input_id, SteerStatus::Applied, None)
        .await
        .unwrap();
    assert!(fixture.mailbox.seal().await);
    second_read.add_permits(1);
    let second_receipt = second.await.unwrap().unwrap();
    assert_eq!(first_receipt.input_id, second_receipt.input_id);
    assert_eq!(
        second_loads.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a concurrent UUID retry must not enter image loading"
    );
    assert!(fixture.mailbox.take().await.is_none());
    assert_eq!(
        fixture.submit(&fixture.input).await.unwrap().status,
        SteerStatus::Applied
    );
    let accepts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_session_events WHERE event_type = 'native_steer' AND json_extract(message, '$.status') = 'accepted'").fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(accepts, 1);
}

#[tokio::test]
async fn steer_ipc_receipt_insert_failure_rolls_back_pending_and_authorized_plan_changes() {
    use crate::native::steer::SteerStatus;
    let mut fixture = SteerIpcFixture::new().await;
    let mut plan_reply = fixture.install_pending_plan().await;
    let initial = fixture.plan_json().await;
    let origin = fixture.mailbox.origin();
    let image = crate::native::images::stage_image_bytes(
        fixture.config.path(),
        "image.png",
        b"\x89PNG\r\nfixture",
    )
    .unwrap();
    fixture.input.image_paths = vec![image.to_string_lossy().into_owned()];
    sqlx::query("CREATE TRIGGER fail_steer_accept BEFORE INSERT ON agent_session_events WHEN NEW.event_type = 'native_steer' AND json_extract(NEW.message, '$.status') = 'accepted' BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END").execute(&fixture.pool).await.unwrap();
    let error = fixture.submit(&fixture.input).await.unwrap_err();
    assert!(error.message.contains("injected receipt failure"));
    assert_eq!(fixture.plan_json().await, initial);
    assert!(fixture
        .manager
        .lock()
        .await
        .require_plan_approval("steer-ipc", "plan-request")
        .is_ok());
    assert!(matches!(
        plan_reply.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(image.exists());
    assert!(fixture.mailbox.take().await.is_none());
    assert!(fixture.mailbox.is_current(&origin));
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_session_events WHERE event_type = 'native_steer'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(events, 0);
    sqlx::query("DROP TRIGGER fail_steer_accept")
        .execute(&fixture.pool)
        .await
        .unwrap();
    let accepted = fixture.submit(&fixture.input).await.unwrap();
    assert_eq!(accepted.status, SteerStatus::Accepted);
    let (pending, approved) = fixture.plan_json().await;
    assert!(pending.is_none());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&approved.unwrap()).unwrap()["status"],
        "cancelled"
    );
    assert!(plan_reply.await.is_err());
}

#[tokio::test]
async fn steer_ipc_late_seal_or_full_mailbox_preserves_plan_and_admission_limits() {
    for late_seal in [false, true] {
        let fixture = SteerIpcFixture::new().await;
        if !late_seal {
            for _ in 0..8 {
                let mut input = fixture.input.clone();
                input.input_id = uuid::Uuid::new_v4().to_string();
                fixture.submit(&input).await.unwrap();
            }
        }
        let plan_reply = fixture.install_pending_plan().await;
        let initial = fixture.plan_json().await;
        let authorization_revision = fixture
            .manager
            .lock()
            .await
            .plan_authorization_revision("steer-ipc");
        let operation_lock = fixture
            .manager
            .lock()
            .await
            .session_operation_lock("steer-ipc");
        let operation = if late_seal {
            Some(operation_lock.lock_owned().await)
        } else {
            None
        };
        let pool = fixture.pool.clone();
        let manager = fixture.manager.clone();
        let input = fixture.input.clone();
        let admission = tokio::spawn(async move {
            super::submit_native_steer_with(
                &pool,
                &manager,
                &input,
                |_| async { Ok(Default::default()) },
                |_, _| {},
            )
            .await
        });
        if late_seal {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while fixture
                    .manager
                    .lock()
                    .await
                    .plan_authorization_revision("steer-ipc")
                    == authorization_revision
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(fixture.mailbox.seal().await);
            drop(operation);
        }
        assert!(admission.await.unwrap().is_err());
        assert_eq!(fixture.plan_json().await, initial);
        assert!(fixture
            .manager
            .lock()
            .await
            .require_plan_approval("steer-ipc", "plan-request")
            .is_ok());
        if !late_seal {
            assert_eq!(
                fixture
                    .manager
                    .lock()
                    .await
                    .plan_authorization_revision("steer-ipc"),
                authorization_revision
            );
        }
        fixture
            .manager
            .lock()
            .await
            .resolve_plan_approval("steer-ipc", "plan-request", Default::default())
            .unwrap();
        assert!(
            !plan_reply.await.unwrap().approved,
            "failed admission leaves the plan answerable"
        );
    }
}

#[tokio::test]
async fn steer_ipc_more_than_256_inputs_evicts_only_terminal_cache_and_keeps_durable_dedup() {
    use crate::native::steer::SteerStatus;
    let fixture = SteerIpcFixture::new().await;
    let pending = fixture.submit(&fixture.input).await.unwrap();
    let held = fixture.mailbox.take().await.unwrap();
    assert_eq!(held.receipt.input_id, pending.input_id);
    let mut oldest_terminal = fixture.input.clone();
    oldest_terminal.input_id = uuid::Uuid::new_v4().to_string();
    for index in 0..257 {
        let mut input = oldest_terminal.clone();
        if index > 0 {
            input.input_id = uuid::Uuid::new_v4().to_string();
        }
        input.text = format!("completed {index}");
        fixture.submit(&input).await.unwrap();
        let claimed = fixture.mailbox.take().await.unwrap();
        fixture
            .mailbox
            .finish_input(&claimed.receipt.input_id, SteerStatus::Applied, None)
            .await
            .unwrap();
    }
    let snapshot = fixture.mailbox.snapshot().await;
    assert_eq!(snapshot.receipts.len(), 129);
    assert_eq!(
        snapshot
            .receipts
            .iter()
            .filter(|receipt| receipt.status == SteerStatus::Accepted)
            .count(),
        1
    );
    assert!(snapshot
        .receipts
        .iter()
        .any(|receipt| receipt.input_id == pending.input_id));
    assert!(!snapshot
        .receipts
        .iter()
        .any(|receipt| receipt.input_id == oldest_terminal.input_id));
    oldest_terminal.text = "completed 0".into();
    assert_eq!(
        fixture.submit(&oldest_terminal).await.unwrap().status,
        SteerStatus::Applied
    );
    assert!(
        fixture.mailbox.take().await.is_none(),
        "evicted same-turn UUID never redelivers"
    );
    assert_eq!(
        fixture.submit(&fixture.input).await.unwrap().status,
        SteerStatus::Accepted
    );
    fixture
        .mailbox
        .finish_input(&pending.input_id, SteerStatus::Applied, None)
        .await
        .unwrap();
    assert!(fixture.mailbox.seal().await);
    assert_eq!(
        fixture.submit(&oldest_terminal).await.unwrap().status,
        SteerStatus::Applied
    );
}
#[test]
fn lifecycle_exit_and_interaction_events_identify_the_originating_runtime() {
    let exit = crate::db::models::AgentSessionExit {
        instance_id: "old-runtime".into(),
        session_record_id: "session".into(),
        profile_id: "".into(),
        workspace_id: None,
        session_kind: "execution".into(),
        code: 0,
        worktree_path: None,
    };
    assert_eq!(
        serde_json::to_value(exit).unwrap()["instance_id"],
        "old-runtime"
    );
    let origin = Some(crate::native::steer::MainOrigin {
        instance_id: "child-runtime".into(),
        generation: 42,
        child: true,
    });
    let question = crate::native::manager::PlanQuestionRequest {
        origin: origin.clone(),
        request_id: "question".into(),
        profile_id: "".into(),
        workspace_id: None,
        session_kind: "plan".into(),
        questions: vec![],
    };
    let plan = crate::native::manager::PlanApprovalRequest {
        origin,
        request_id: "approval".into(),
        profile_id: "".into(),
        workspace_id: None,
        session_kind: "plan".into(),
        plan: "body".into(),
    };
    assert_eq!(
        serde_json::to_value(super::question_event("session", &question)).unwrap()["instance_id"],
        "child-runtime"
    );
    assert_eq!(
        serde_json::to_value(super::plan_approval_event("session", &plan)).unwrap()["instance_id"],
        "child-runtime"
    );
    let delta = crate::db::models::NativeTextDelta {
        session_record_id: "session".into(),
        instance_id: "runtime".into(),
        turn_id: Some("completed-turn".into()),
        kind: "text".into(),
        text: "".into(),
        clear: true,
        assistant: None,
    };
    let wire = serde_json::to_value(delta).unwrap();
    assert_eq!(wire["instance_id"], "runtime");
    assert_eq!(wire["turn_id"], "completed-turn");
}
