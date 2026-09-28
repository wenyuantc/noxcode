#[allow(unused_imports)]
use super::*;

pub(crate) fn session_title(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(30).collect())
}

pub(super) async fn resolve_insert_title(
    pool: &sqlx::SqlitePool,
    prompt: &str,
    resume_session_id: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(resume_id) = resume_session_id {
        let inherited = sqlx::query_scalar::<_, Option<String>>(
            "SELECT title FROM agent_sessions WHERE id = $1 LIMIT 1",
        )
        .bind(resume_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取续聊标题失败: {error}"))?;
        if let Some(title) = inherited {
            return Ok(title);
        }
    }
    Ok(session_title(prompt))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_agent_session(
    pool: &sqlx::SqlitePool,
    id: &str,
    ai_channel_id: &str,
    workspace_id: &str,
    working_dir: &str,
    execution_target: &str,
    ssh_config_id: Option<&str>,
    target_host_label: Option<&str>,
    kind: &str,
    resume_session_id: Option<&str>,
    prompt: &str,
) -> Result<String, String> {
    let now = now_sqlite();
    let title = resolve_insert_title(pool, prompt, resume_session_id).await?;
    sqlx::query(
        r#"
        INSERT INTO agent_sessions (
            id, ai_channel_id, workspace_id, working_dir, execution_target,
            ssh_config_id, target_host_label, session_kind, status,
            started_at, resume_session_id, created_at, title
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'running', $9, $10, $9, $11)
        "#,
    )
    .bind(id)
    .bind(ai_channel_id)
    .bind(workspace_id)
    .bind(working_dir)
    .bind(execution_target)
    .bind(ssh_config_id)
    .bind(target_host_label)
    .bind(kind)
    .bind(&now)
    .bind(resume_session_id)
    .bind(title)
    .execute(pool)
    .await
    .map_err(|error| format!("创建会话失败: {error}"))?;
    Ok(id.to_string())
}

pub(super) async fn enqueue_live_input(
    manager: &Mutex<NativeAgentManager>,
    session_record_id: &str,
    input: &str,
    image_paths: Option<&[String]>,
    attachment_pool: Option<&sqlx::SqlitePool>,
    attachment_root: Option<&std::path::Path>,
) -> Result<Option<(NativeSessionInfo, NativeInputQueueSnapshot)>, String> {
    let trimmed = input.trim();
    let mut loaded = crate::native::images::load_native_images(image_paths);
    if !loaded.images.is_empty() {
        if let (Some(pool), Some(root)) = (attachment_pool, attachment_root) {
            crate::native::images::remember_loaded_media(pool, root, &mut loaded).await?;
        }
    }
    crate::native::images::cleanup_staged_loaded_images(&loaded);
    if trimmed.is_empty() && loaded.images.is_empty() {
        return Err("输入内容不能为空".to_string());
    }
    let manager = manager.lock().await;
    manager.require_running()?;
    let Some(session) = manager.get_session(session_record_id) else {
        return Ok(None);
    };
    if session.closing {
        return Err("内置 Agent 正在结束，请稍后重试".to_string());
    }
    let snapshot = session.input_queue.enqueue(trimmed, loaded.images)?;
    session.working.store(true, Ordering::SeqCst);
    Ok(Some((session.info.clone(), snapshot)))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn reactivate_agent_session(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    workspace_id: &str,
    ai_channel_id: &str,
    working_dir: &str,
    execution_target: &str,
    ssh_config_id: Option<&str>,
    target_host_label: Option<&str>,
    kind: &str,
) -> Result<String, String> {
    require_unarchived_session_with(pool, session_id).await?;
    let session = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_id}"))?;
    if session.workspace_id.as_deref() != Some(workspace_id) {
        return Err("会话不属于当前工作区".to_string());
    }
    let now = now_sqlite();
    sqlx::query(
        r#"
        UPDATE agent_sessions SET
            ai_channel_id = $1,
            workspace_id = $2,
            working_dir = $3,
            execution_target = $4,
            ssh_config_id = $5,
            target_host_label = $6,
            session_kind = $7,
            status = 'running',
            started_at = $8,
            ended_at = NULL,
            exit_code = NULL
        WHERE id = $9
        "#,
    )
    .bind(ai_channel_id)
    .bind(workspace_id)
    .bind(working_dir)
    .bind(execution_target)
    .bind(ssh_config_id)
    .bind(target_host_label)
    .bind(kind)
    .bind(&now)
    .bind(session_id)
    .execute(pool)
    .await
    .map_err(|error| format!("恢复会话失败: {error}"))?;
    Ok(session_id.to_string())
}

pub(super) async fn update_agent_session_status(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
    status: &str,
    exit_code: Option<i32>,
    ended_at: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE agent_sessions SET status = $1, exit_code = COALESCE($2, exit_code), ended_at = COALESCE($3, ended_at) WHERE id = $4",
    )
    .bind(status)
    .bind(exit_code)
    .bind(ended_at)
    .bind(session_record_id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新会话状态失败: {error}"))?;
    Ok(())
}

pub(super) async fn apply_session_usage(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
    delta: &UsageDelta,
) -> Result<(), String> {
    sqlx::query(
        r#"
        UPDATE agent_sessions SET
            input_tokens = COALESCE(input_tokens, 0) + $1,
            output_tokens = COALESCE(output_tokens, 0) + $2,
            total_tokens = COALESCE(total_tokens, 0) + $3,
            reasoning_tokens = COALESCE(reasoning_tokens, 0) + $4,
            cached_tokens = COALESCE(cached_tokens, 0) + $5
        WHERE id = $6
        "#,
    )
    .bind(delta.input_tokens.unwrap_or(0) as i64)
    .bind(delta.output_tokens.unwrap_or(0) as i64)
    .bind(delta.total_tokens.unwrap_or(0) as i64)
    .bind(delta.reasoning_tokens.unwrap_or(0) as i64)
    .bind(delta.cached_tokens.unwrap_or(0) as i64)
    .bind(session_record_id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新会话用量失败: {error}"))?;
    Ok(())
}

pub(super) fn attach_mutation_checkpoint(
    app: &AppHandle,
    runner: &mut AgentRunner,
    workspace_id: String,
    session_record_id: String,
) {
    let enabled = crate::native::settings::load_native_settings(app)
        .map(|settings| settings.auto_checkpoint_after_tool_call)
        .unwrap_or(true);
    runner.ctx.record_file_revisions = enabled;
    if !enabled {
        runner.ctx.on_mutation = None;
        return;
    }

    let inflight = Arc::new(AtomicBool::new(false));
    let app = app.clone();
    let active_root = runner.ctx.active_root.clone();
    runner.ctx.on_mutation = Some(Arc::new(move |tool_name: &str| {
        if inflight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let inflight = inflight.clone();
        let app = app.clone();
        let workspace_id = workspace_id.clone();
        let session_record_id = session_record_id.clone();
        let active_root = active_root.clone();
        let label = format!("after_tool_call:{tool_name}");
        tauri::async_runtime::spawn(async move {
            let result = async {
                let pool = sqlite_pool(&app).await?;
                let cwd = active_root
                    .read()
                    .map(|root| root.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let target = crate::git::resolve_git_target_at(&app, &workspace_id, &cwd).await?;
                create_checkpoint(
                    &pool,
                    &target,
                    &workspace_id,
                    &session_record_id,
                    Some(&label),
                    Some("after_tool_call"),
                )
                .await
                .map_err(String::from)?;
                Ok::<(), String>(())
            }
            .await;
            if let Err(error) = result {
                eprintln!("[native] after_tool_call 打点失败: {error}");
            }
            inflight.store(false, Ordering::SeqCst);
        });
    }));
}

/// 按设置装配本地工具运行时：Bash 默认超时、shell 快照、ripgrep、artifact 存储。
/// SSH 工作区不导出本机 shell 快照，但 artifact 目录仍在本机。
pub(super) async fn configure_local_tool_runtime(
    app: &AppHandle,
    runner: &mut AgentRunner,
    session_record_id: &str,
    workspace_id: &str,
) {
    let settings = match crate::native::settings::load_native_settings(app) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("[native] 读取运行时设置失败，使用默认值: {error}");
            return;
        }
    };
    runner.ctx.workspace.bash_default_timeout =
        Duration::from_secs(settings.bash_default_timeout_secs.max(1) as u64);
    let config_dir = app.path().app_config_dir().ok();
    runner.ctx.app_config_dir = config_dir.clone();
    runner.ctx.worktree_root = settings.worktree_root.clone();
    runner.ctx.worktree_fetch_before_create = settings.worktree_fetch_before_create;
    if settings.rg_sidecar_enabled {
        let bundled = app.path().resource_dir().ok().map(|dir| dir.join("tools"));
        runner.ctx.workspace.rg_binary =
            crate::native::tools::local::locate_ripgrep(bundled.as_deref());
    }
    if settings.shell_snapshot_enabled && runner.ctx.ssh.is_none() {
        if let Some(dir) = config_dir.as_ref() {
            match crate::native::tools::shell_snapshot::capture_shell_snapshot(dir).await {
                Ok(path) => runner.ctx.workspace.shell_snapshot = Some(path),
                Err(error) => eprintln!("[native] 导出 shell 快照失败，回退 bash -lc: {error}"),
            }
        }
    }
    runner.ctx.workspace.sandbox = crate::native::tools::sandbox::SandboxPolicy {
        enabled: settings.bash_sandbox_enabled && runner.ctx.ssh.is_none(),
        extra_write_roots: Vec::new(),
    };
    if settings.lsp_enabled && runner.ctx.ssh.is_none() {
        runner.ctx.lsp = Some(Arc::new(crate::native::tools::lsp::LspHub::new(
            runner.ctx.active_workspace_root(),
            true,
        )));
    }
    runner.ctx.computer_control_enabled =
        settings.computer_control_enabled && runner.ctx.ssh.is_none();
    let processes = Arc::new(crate::native::tools::processes::ProcessRegistry::new());
    let emit_app = app.clone();
    let emit_session = session_record_id.to_string();
    processes.set_on_change(Some(Arc::new(move |items| {
        let _ = emit_app.emit(
            "native-background-processes",
            serde_json::json!({
                "session_record_id": emit_session,
                "processes": items,
            }),
        );
    })));
    runner.ctx.processes = Some(processes);
    if let Ok(target) = crate::git::resolve_git_target(app, workspace_id).await {
        runner.ctx.git_target = Some(target);
    }
    if let Some(dir) = config_dir.as_ref() {
        let record_app = app.clone();
        let store = crate::native::artifacts::ArtifactStore::new(dir, session_record_id)
            .with_recorder(Arc::new(move |record| {
                let app = record_app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(pool) = sqlite_pool(&app).await {
                        if let Err(error) =
                            crate::native::artifacts::insert_artifact_row(&pool, &record).await
                        {
                            eprintln!("[native] {error}");
                        }
                    }
                });
            }));
        runner
            .ctx
            .workspace
            .extra_read_roots
            .push(store.dir().to_path_buf());
        runner.set_artifact_store(Arc::new(store));
    }
}

pub(super) async fn maybe_isolate_session_worktree(
    app: &AppHandle,
    workspace_id: &str,
    session_record_id: &str,
    run_cwd: &mut String,
    isolate: bool,
    is_resume: bool,
    existing_working_dir: Option<&str>,
) -> (Option<String>, Vec<String>) {
    let mut notices = Vec::new();
    if is_resume {
        if let Some(existing) = existing_working_dir
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let configured_root = crate::native::settings::load_native_settings(app)
                .ok()
                .map(|settings| settings.worktree_root);
            if crate::git::worktree::is_managed_worktree_path_with_root(
                existing,
                session_record_id,
                configured_root
                    .as_deref()
                    .and_then(crate::git::managed::configured_root_opt),
            ) {
                *run_cwd = existing.to_string();
                return (Some(existing.to_string()), notices);
            }
        }
        return (None, notices);
    }
    if !isolate {
        return (None, notices);
    }
    let settings = crate::native::settings::load_native_settings(app).ok();
    let configured_root = settings
        .as_ref()
        .map(|item| item.worktree_root.as_str())
        .unwrap_or("");
    match crate::git::resolve_git_target(app, workspace_id).await {
        Ok(target) => {
            let path = match &target {
                crate::git::GitTarget::Local(_) => match app.path().app_config_dir() {
                    Ok(dir) => {
                        let root = crate::git::worktree::resolve_local_worktree_root(
                            &dir,
                            configured_root,
                        );
                        crate::git::worktree::local_worktree_path_in_root(&root, session_record_id)
                            .to_string_lossy()
                            .into_owned()
                    }
                    Err(error) => {
                        notices.push(format!("无法解析配置目录，已跳过 worktree 隔离：{error}"));
                        return (None, notices);
                    }
                },
                crate::git::GitTarget::Ssh { .. } => {
                    crate::git::worktree::remote_worktree_path(session_record_id)
                }
            };
            if settings
                .as_ref()
                .is_some_and(|item| item.worktree_fetch_before_create)
            {
                if let Err(error) = crate::git::worktree::fetch_all_prune(&target).await {
                    notices.push(format!("创建工作树前获取上游失败，已继续创建：{error}"));
                }
            }
            match crate::git::worktree::add_session_worktree(&target, &path, session_record_id)
                .await
            {
                Ok(_) => {
                    *run_cwd = path.clone();
                    if settings
                        .as_ref()
                        .is_some_and(|item| item.worktree_auto_prune)
                    {
                        if let (Ok(pool), Some(state)) = (
                            crate::app::shared::sqlite_pool(app).await,
                            app.try_state::<std::sync::Arc<
                                tokio::sync::Mutex<crate::native::manager::NativeAgentManager>,
                            >>(),
                        ) {
                            if let Err(error) = crate::git::prune_old_managed_worktrees(
                                app,
                                &pool,
                                state.inner(),
                                Some(session_record_id),
                            )
                            .await
                            {
                                notices.push(format!("自动清理旧工作树失败：{error}"));
                            }
                        }
                    }
                    (Some(path), notices)
                }
                Err(error) => {
                    notices.push(format!("无法创建隔离 worktree，已使用主工作区：{error}"));
                    (None, notices)
                }
            }
        }
        Err(_) => {
            notices.push("工作区不是 git 仓库，已跳过 worktree 隔离".to_string());
            (None, notices)
        }
    }
}

pub(super) async fn load_session_ssh_config(
    pool: &sqlx::SqlitePool,
    execution_context: &ExecutionContext,
) -> Result<Option<SshConfigRecord>, String> {
    if !execution_context.is_ssh() {
        return Ok(None);
    }
    let ssh_id = execution_context
        .ssh_config_id
        .as_deref()
        .ok_or_else(|| "SSH 工作区缺少 ssh_config_id".to_string())?;
    let config = fetch_ssh_config_record_by_id(pool, ssh_id).await?;
    validate_password_execution(&config)?;
    Ok(Some(config))
}

#[tauri::command]
pub async fn restore_session_worktree(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_id: String,
) -> Result<Option<String>, String> {
    crate::app::lifecycle::require_running(&app)?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("session_id 不能为空".to_string());
    }
    let pool = sqlite_pool(&app).await?;
    let working_dir: Option<String> =
        sqlx::query_scalar("SELECT working_dir FROM agent_sessions WHERE id = $1 LIMIT 1")
            .bind(&session_id)
            .fetch_optional(&pool)
            .await
            .map_err(|error| format!("读取会话失败: {error}"))?
            .flatten();
    let configured_root = crate::native::settings::load_native_settings(&app)
        .ok()
        .map(|settings| settings.worktree_root);
    let configured_opt = configured_root
        .as_deref()
        .and_then(crate::git::managed::configured_root_opt);
    let path = {
        let mut manager = state.lock().await;
        let Some(path) = manager
            .isolation_worktree_path(&session_id)
            .or_else(|| {
                manager
                    .get_session(&session_id)
                    .and_then(|item| item.runtime.as_ref()?.worktree_path.clone())
            })
            .or_else(|| {
                working_dir.filter(|item| {
                    crate::git::worktree::is_managed_worktree_path_with_root(
                        item,
                        &session_id,
                        configured_opt,
                    )
                })
            })
        else {
            return Ok(None);
        };
        manager.restore_isolation_worktree_to(&session_id, &path);
        if let Some(session) = manager.get_session_mut(&session_id) {
            if let Some(runtime) = session.runtime.as_mut() {
                runtime.worktree_path = Some(path.clone());
            }
        }
        path
    };
    sqlx::query("UPDATE agent_sessions SET working_dir = $1 WHERE id = $2")
        .bind(&path)
        .bind(&session_id)
        .execute(&pool)
        .await
        .map_err(|error| format!("更新会话工作目录失败: {error}"))?;
    Ok(Some(path))
}
