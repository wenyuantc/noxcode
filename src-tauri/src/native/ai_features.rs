use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::app::activity_logs::insert_activity_log;
use crate::app::ai_settings::{load_ai_settings, AiFeatureOverride, CommitMessageStyle};
use crate::app::sessions::rename_agent_session_with;
use crate::app::shared::{normalize_optional_text, sqlite_pool};
use crate::db::models::AgentSessionRecord;
use crate::git::{
    apply_resolved_files, collect_commit_message_context, conflict_resolve_prompt,
    list_unmerged_paths, merge_in_progress, read_worktree_text, resolve_git_target,
    run_abort_merge, sanitize_conflict_resolution, MergeWorktreeResult, ResolveWorktreeAction,
};
use crate::native::channels::fetch_channel_record;
use crate::native::manager::NativeAgentManager;
use crate::native::model::call_log::{
    OPERATION_COMMIT_MESSAGE, OPERATION_MERGE_RESOLVE, OPERATION_SESSION_TITLE,
};
use crate::native::protocol::record_to_channel;
use crate::native::session::{
    run_native_one_shot, session_title, NativeOneShotArgs, NativeOneShotResult,
};

#[derive(Debug, Clone, Serialize)]
pub struct NativeSessionTitle {
    pub session_id: String,
    pub title: String,
}

pub(crate) fn sanitize_generated_session_title(raw: &str) -> Option<String> {
    let first_line = raw.lines().next().unwrap_or("").trim();
    let stripped = first_line
        .trim_matches(|item| {
            matches!(
                item,
                '"' | '\'' | '`' | '「' | '」' | '『' | '』' | '“' | '”' | '‘' | '’'
            )
        })
        .trim();
    session_title(stripped)
}

pub(crate) fn sanitize_generated_commit_message(
    raw: &str,
    style: CommitMessageStyle,
) -> Result<String, String> {
    let mut text = raw.trim().to_string();
    if text.starts_with("```") {
        let mut lines = text.lines();
        let _ = lines.next();
        let mut body: Vec<&str> = lines.collect();
        if body
            .last()
            .is_some_and(|line| line.trim().starts_with("```"))
        {
            body.pop();
        }
        text = body.join("\n").trim().to_string();
    }
    if style == CommitMessageStyle::Concise {
        text = text.lines().next().unwrap_or("").trim().to_string();
    }
    if text.is_empty() {
        return Err("模型未返回提交说明".to_string());
    }
    Ok(text)
}

pub(crate) async fn resolve_ai_feature_target(
    pool: &SqlitePool,
    feature: &AiFeatureOverride,
) -> Result<(String, String, Option<String>), String> {
    if !feature.enabled {
        return Err("该 AI 功能未开启".to_string());
    }
    let channel_id = match normalize_optional_text(feature.channel_id.as_deref()) {
        Some(id) => id,
        None => sqlx::query_scalar::<_, String>(
            "SELECT id FROM ai_channels WHERE enabled = 1 ORDER BY created_at ASC, id ASC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取 AI 渠道失败: {error}"))?
        .ok_or_else(|| "请先配置并启用 AI 渠道".to_string())?,
    };
    let record = fetch_channel_record(pool, &channel_id).await?;
    if record.enabled == 0 {
        return Err(format!("渠道「{}」已停用", record.name));
    }
    let channel = record_to_channel(record)?;
    let model = match normalize_optional_text(feature.model.as_deref()) {
        Some(id) => {
            if !channel.models.iter().any(|item| item.id == id) {
                return Err(format!("渠道「{}」没有模型「{}」", channel.name, id));
            }
            id
        }
        None => channel
            .models
            .first()
            .map(|item| item.id.clone())
            .ok_or_else(|| format!("渠道「{}」未配置模型", channel.name))?,
    };
    Ok((
        channel.id,
        model,
        normalize_optional_text(feature.reasoning_effort.as_deref()),
    ))
}

fn commit_message_prompt(style: CommitMessageStyle, context: &str) -> String {
    match style {
        CommitMessageStyle::Concise => format!(
            "根据以下 Git 变更生成一条简约的 Conventional Commit 提交说明。\n\
只输出一行提交说明，不要正文、不要解释、不要代码围栏。\n\
格式：type(scope): subject\n\
整行不超过 72 个字符。subject 用祈使语气概括最主要的变更。\n\n\
变更：\n{context}"
        ),
        CommitMessageStyle::Detailed => format!(
            "根据以下 Git 变更生成一条明细的 Conventional Commit 提交说明。\n\
只输出提交说明本身，不要解释，不要代码围栏。\n\
第一行格式：type(scope): subject（不超过 72 个字符）。\n\
空一行后写正文：用项目符号列出本次变更做了什么、为什么改、影响哪些模块或文件。\n\
条目必须来自下面的实际 diff，不要编造；变更很小也要写清具体改动，不要只给标题。\n\n\
变更：\n{context}"
        ),
    }
}

fn session_title_prompt(prompt: &str) -> String {
    format!(
        "用不超过 30 个汉字或英文词概括下面用户请求的主题，作为会话标题。\n\
只输出标题，不要引号、标点装饰或解释。\n\n\
用户请求：\n{prompt}"
    )
}

async fn run_feature_one_shot(
    app: &AppHandle,
    pool: &SqlitePool,
    feature: &AiFeatureOverride,
    workspace_id: Option<&str>,
    session_id: Option<&str>,
    prompt: String,
    operation: &str,
) -> Result<NativeOneShotResult, String> {
    let (channel_id, model, reasoning_effort) = resolve_ai_feature_target(pool, feature).await?;
    run_native_one_shot(
        app,
        NativeOneShotArgs {
            channel_id: &channel_id,
            workspace_id,
            session_id,
            prompt,
            image_paths: None,
            model: Some(&model),
            reasoning_effort: reasoning_effort.as_deref(),
            operation: Some(operation),
        },
    )
    .await
}

#[tauri::command]
pub async fn generate_git_commit_message(
    app: AppHandle,
    workspace_id: String,
) -> Result<String, String> {
    let settings = load_ai_settings(&app)?;
    if !settings.commit_message.enabled {
        return Err("未开启 Git 提交信息自动生成".to_string());
    }
    let target = resolve_git_target(&app, &workspace_id).await?;
    let context = collect_commit_message_context(&target).await?;
    let pool = sqlite_pool(&app).await?;
    let result = run_feature_one_shot(
        &app,
        &pool,
        settings.commit_message.as_override(),
        Some(&workspace_id),
        None,
        commit_message_prompt(settings.commit_message.style, &context),
        OPERATION_COMMIT_MESSAGE,
    )
    .await?;
    sanitize_generated_commit_message(&result.text, settings.commit_message.style)
}

async fn resolve_merge_ai_target(
    pool: &SqlitePool,
    session: &AgentSessionRecord,
) -> Result<(String, String, Option<String>), String> {
    if let Some(channel_id) = session
        .ai_channel_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let record = fetch_channel_record(pool, channel_id).await?;
        let channel = record_to_channel(record)?;
        let model = session
            .model
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| channel.models.first().map(|item| item.id.clone()))
            .ok_or_else(|| format!("渠道「{}」未配置模型", channel.name))?;
        return Ok((channel.id, model, None));
    }
    resolve_ai_feature_target(
        pool,
        &AiFeatureOverride {
            enabled: true,
            channel_id: None,
            model: None,
            reasoning_effort: None,
        },
    )
    .await
}

#[tauri::command]
pub async fn resolve_session_worktree_merge(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    workspace_id: String,
    session_id: String,
    action: ResolveWorktreeAction,
) -> Result<MergeWorktreeResult, String> {
    if state
        .lock()
        .await
        .has_working_workspace_processes(&workspace_id)
    {
        return Err("该工作区有正在执行的会话，请等待本轮结束或停止后再处理合并".to_string());
    }
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    let result = match action {
        ResolveWorktreeAction::Abort => run_abort_merge(&target).await.map_err(String::from)?,
        ResolveWorktreeAction::Ai => {
            if !merge_in_progress(&target).await.map_err(String::from)? {
                return Err("当前没有进行中的合并".to_string());
            }
            let conflicts = list_unmerged_paths(&target).await.map_err(String::from)?;
            if conflicts.is_empty() {
                return Err("没有未合并的冲突文件".to_string());
            }
            let session = sqlx::query_as::<_, AgentSessionRecord>(
                "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
            )
            .bind(&session_id)
            .fetch_optional(&pool)
            .await
            .map_err(|error| format!("读取会话失败: {error}"))?
            .ok_or_else(|| format!("会话不存在: {session_id}"))?;
            let (channel_id, model, reasoning_effort) =
                resolve_merge_ai_target(&pool, &session).await?;
            let mut resolutions = Vec::new();
            for path in conflicts {
                let content = match read_worktree_text(&target, &path).await {
                    Ok(text) => text,
                    Err(error) => {
                        resolutions.push((path, Err(error.to_string())));
                        continue;
                    }
                };
                let one_shot = run_native_one_shot(
                    &app,
                    NativeOneShotArgs {
                        channel_id: &channel_id,
                        workspace_id: Some(&workspace_id),
                        session_id: Some(&session_id),
                        prompt: conflict_resolve_prompt(&path, &content),
                        image_paths: None,
                        model: Some(&model),
                        reasoning_effort: reasoning_effort.as_deref(),
                        operation: Some(OPERATION_MERGE_RESOLVE),
                    },
                )
                .await;
                let resolved = match one_shot {
                    Ok(result) => sanitize_conflict_resolution(&result.text),
                    Err(error) => Err(error),
                };
                resolutions.push((path, resolved));
            }
            apply_resolved_files(&target, &resolutions)
                .await
                .map_err(String::from)?
        }
    };
    let _ = insert_activity_log(
        &pool,
        "git_worktree_merge_resolve",
        Some(&workspace_id),
        Some(&session_id),
        &result.message,
        serde_json::json!({
            "action": action,
            "status": result.status,
            "conflicts": result.conflicts,
            "failed": result.failed,
        }),
    )
    .await;
    Ok(result)
}

pub(crate) fn spawn_session_title_generation(
    app: AppHandle,
    session_id: String,
    workspace_id: String,
    prompt: String,
) {
    if session_title(&prompt).is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(error) = generate_session_title(&app, &session_id, &workspace_id, &prompt).await
        {
            eprintln!("[ai] 生成会话标题失败: {error}");
        }
    });
}

async fn generate_session_title(
    app: &AppHandle,
    session_id: &str,
    workspace_id: &str,
    prompt: &str,
) -> Result<(), String> {
    let settings = load_ai_settings(app)?;
    if !settings.session_title.enabled {
        return Ok(());
    }
    let pool = sqlite_pool(app).await?;
    let result = run_feature_one_shot(
        app,
        &pool,
        &settings.session_title,
        Some(workspace_id),
        Some(session_id),
        session_title_prompt(prompt),
        OPERATION_SESSION_TITLE,
    )
    .await?;
    let Some(title) = sanitize_generated_session_title(&result.text) else {
        return Ok(());
    };
    let row = rename_agent_session_with(&pool, session_id, &title).await?;
    let title = row.title.unwrap_or(title);
    let _ = app.emit(
        "native-session-title",
        NativeSessionTitle {
            session_id: session_id.to_string(),
            title,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ai_settings::AiFeatureOverride;
    use crate::db::test_support::setup_migrated_pool;

    #[test]
    fn sanitizes_title_quotes_and_truncates() {
        assert_eq!(
            sanitize_generated_session_title("  「修复登录」  ").as_deref(),
            Some("修复登录")
        );
        assert_eq!(sanitize_generated_session_title("   "), None);
        let chinese = "一二三四五六七八九十";
        let thirty = format!("{chinese}{chinese}{chinese}");
        let over = format!("\"{thirty}超出\"");
        assert_eq!(
            sanitize_generated_session_title(&over).as_deref(),
            Some(thirty.as_str())
        );
    }

    #[test]
    fn sanitizes_commit_fences() {
        let message = sanitize_generated_commit_message(
            "```\nfeat: add button\n```",
            CommitMessageStyle::Detailed,
        )
        .expect("ok");
        assert_eq!(message, "feat: add button");
        let err = sanitize_generated_commit_message("   ", CommitMessageStyle::Detailed)
            .expect_err("empty");
        assert!(err.contains("提交说明"));
    }

    #[test]
    fn concise_keeps_first_line_only() {
        let message = sanitize_generated_commit_message(
            "feat(ui): add button\n\n- keep extra body",
            CommitMessageStyle::Concise,
        )
        .expect("ok");
        assert_eq!(message, "feat(ui): add button");
    }

    #[test]
    fn commit_message_prompt_differs_by_style() {
        let concise = commit_message_prompt(CommitMessageStyle::Concise, "diff a");
        let detailed = commit_message_prompt(CommitMessageStyle::Detailed, "diff a");
        assert!(concise.contains("简约"));
        assert!(concise.contains("不要正文"));
        assert!(detailed.contains("明细"));
        assert!(detailed.contains("项目符号"));
        assert_ne!(concise, detailed);
    }

    #[tokio::test]
    async fn resolve_requires_enabled_feature() {
        let pool = setup_migrated_pool().await;
        let err = resolve_ai_feature_target(
            &pool,
            &AiFeatureOverride {
                enabled: false,
                channel_id: None,
                model: None,
                reasoning_effort: None,
            },
        )
        .await
        .expect_err("disabled");
        assert!(err.contains("未开启"));
    }

    #[tokio::test]
    async fn resolve_requires_enabled_channel() {
        let pool = setup_migrated_pool().await;
        let err = resolve_ai_feature_target(
            &pool,
            &AiFeatureOverride {
                enabled: true,
                channel_id: None,
                model: None,
                reasoning_effort: None,
            },
        )
        .await
        .expect_err("no channel");
        assert!(err.contains("AI 渠道"));
    }
}
