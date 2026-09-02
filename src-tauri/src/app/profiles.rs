use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime, State};
use tokio::sync::Mutex;

use crate::app::shared::{new_id, normalize_optional_text, now_sqlite, sqlite_pool};
use crate::db::models::{AgentProfile, CreateAgentProfile, UpdateAgentProfile};
use crate::native::channels::fetch_channel_record;
use crate::native::manager::NativeAgentManager;

fn normalize_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Err("档案名称不能为空".to_string());
    }
    Ok(name.to_string())
}

fn normalize_model(value: &str) -> Result<String, String> {
    let model = value.trim();
    if model.is_empty() {
        return Err("模型不能为空".to_string());
    }
    Ok(model.to_string())
}

fn normalize_reasoning_effort(value: Option<&str>) -> Result<String, String> {
    match value.map(str::trim).filter(|item| !item.is_empty()) {
        None => Ok("high".to_string()),
        Some(value @ ("low" | "medium" | "high" | "xhigh" | "max")) => Ok(value.to_string()),
        Some(other) => Err(format!("不支持的推理强度: {other}")),
    }
}

async fn ensure_channel_usable(pool: &SqlitePool, channel_id: &str) -> Result<(), String> {
    let record = fetch_channel_record(pool, channel_id).await?;
    if record.enabled == 0 {
        return Err(format!("渠道「{}」已停用", record.name));
    }
    Ok(())
}

pub(crate) async fn fetch_agent_profile_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<AgentProfile, String> {
    sqlx::query_as::<_, AgentProfile>("SELECT * FROM agent_profiles WHERE id = $1 LIMIT 1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取 Agent 档案失败: {error}"))?
        .ok_or_else(|| format!("档案不存在: {id}"))
}

pub(crate) async fn list_agent_profiles_with(
    pool: &SqlitePool,
) -> Result<Vec<AgentProfile>, String> {
    sqlx::query_as::<_, AgentProfile>("SELECT * FROM agent_profiles ORDER BY updated_at DESC")
        .fetch_all(pool)
        .await
        .map_err(|error| format!("读取 Agent 档案列表失败: {error}"))
}

pub(crate) async fn create_agent_profile_with(
    pool: &SqlitePool,
    payload: CreateAgentProfile,
) -> Result<AgentProfile, String> {
    let name = normalize_name(&payload.name)?;
    let channel_id = payload.ai_channel_id.trim();
    if channel_id.is_empty() {
        return Err("必须选择渠道".to_string());
    }
    ensure_channel_usable(pool, channel_id).await?;
    let model = normalize_model(&payload.model)?;
    let reasoning_effort = normalize_reasoning_effort(payload.reasoning_effort.as_deref())?;
    let system_prompt = normalize_optional_text(payload.system_prompt.as_deref());
    let id = new_id();
    let now = now_sqlite();
    sqlx::query(
        "INSERT INTO agent_profiles (id, name, ai_channel_id, model, reasoning_effort, system_prompt, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&id)
    .bind(&name)
    .bind(channel_id)
    .bind(&model)
    .bind(&reasoning_effort)
    .bind(&system_prompt)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| format!("创建档案失败: {error}"))?;
    fetch_agent_profile_by_id(pool, &id).await
}

pub(crate) async fn update_agent_profile_with(
    pool: &SqlitePool,
    id: &str,
    updates: UpdateAgentProfile,
) -> Result<AgentProfile, String> {
    let current = fetch_agent_profile_by_id(pool, id).await?;
    let name = match updates.name.as_deref() {
        Some(value) => normalize_name(value)?,
        None => current.name,
    };
    let channel_id = match updates.ai_channel_id.as_deref() {
        Some(value) => {
            let channel_id = value.trim();
            if channel_id.is_empty() {
                return Err("必须选择渠道".to_string());
            }
            ensure_channel_usable(pool, channel_id).await?;
            channel_id.to_string()
        }
        None => current
            .ai_channel_id
            .ok_or_else(|| "档案未绑定渠道".to_string())?,
    };
    let model = match updates.model.as_deref() {
        Some(value) => normalize_model(value)?,
        None => current.model,
    };
    let reasoning_effort = match updates.reasoning_effort.as_deref() {
        Some(value) => normalize_reasoning_effort(Some(value))?,
        None => current.reasoning_effort,
    };
    let system_prompt = match updates.system_prompt {
        Some(value) => normalize_optional_text(value.as_deref()),
        None => current.system_prompt,
    };
    let now = now_sqlite();
    sqlx::query(
        "UPDATE agent_profiles SET name = $1, ai_channel_id = $2, model = $3, reasoning_effort = $4, system_prompt = $5, updated_at = $6 WHERE id = $7",
    )
    .bind(&name)
    .bind(&channel_id)
    .bind(&model)
    .bind(&reasoning_effort)
    .bind(&system_prompt)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新档案失败: {error}"))?;
    fetch_agent_profile_by_id(pool, id).await
}

pub(crate) async fn delete_agent_profile_with(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM agent_profiles WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| format!("删除档案失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err(format!("档案不存在: {id}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_agent_profiles<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<AgentProfile>, String> {
    let pool = sqlite_pool(&app).await?;
    list_agent_profiles_with(&pool).await
}

#[tauri::command]
pub async fn create_agent_profile<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateAgentProfile,
) -> Result<AgentProfile, String> {
    let pool = sqlite_pool(&app).await?;
    create_agent_profile_with(&pool, payload).await
}

#[tauri::command]
pub async fn update_agent_profile<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    updates: UpdateAgentProfile,
) -> Result<AgentProfile, String> {
    let pool = sqlite_pool(&app).await?;
    update_agent_profile_with(&pool, &id, updates).await
}

#[tauri::command]
pub async fn delete_agent_profile<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, std::sync::Arc<Mutex<NativeAgentManager>>>,
    id: String,
) -> Result<(), String> {
    if state.lock().await.has_profile_processes(&id) {
        return Err("该档案有运行中的会话，无法删除".to_string());
    }
    let pool = sqlite_pool(&app).await?;
    delete_agent_profile_with(&pool, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::CreateAiChannel;
    use crate::db::test_support::setup_migrated_pool;
    use crate::native::channels::create_ai_channel_with;

    async fn seed_channel(pool: &SqlitePool) -> String {
        let channel = create_ai_channel_with(
            pool,
            CreateAiChannel {
                name: "demo".to_string(),
                protocol: "openai".to_string(),
                base_url: "https://api.example.com".to_string(),
                api_key: Some("sk-test".to_string()),
                extra_headers_json: None,
                models: None,
                enabled: Some(true),
            },
        )
        .await
        .expect("channel");
        channel.id
    }

    #[tokio::test]
    async fn create_update_list_and_delete_profile() {
        let pool = setup_migrated_pool().await;
        let channel_id = seed_channel(&pool).await;
        let created = create_agent_profile_with(
            &pool,
            CreateAgentProfile {
                name: "coder".to_string(),
                ai_channel_id: channel_id.clone(),
                model: "gpt-4o".to_string(),
                reasoning_effort: Some("high".to_string()),
                system_prompt: Some("认真".to_string()),
            },
        )
        .await
        .expect("create");
        assert_eq!(created.name, "coder");
        let listed = list_agent_profiles_with(&pool).await.expect("list");
        assert_eq!(listed.len(), 1);
        let updated = update_agent_profile_with(
            &pool,
            &created.id,
            UpdateAgentProfile {
                name: Some("reviewer".to_string()),
                ai_channel_id: None,
                model: None,
                reasoning_effort: Some("low".to_string()),
                system_prompt: Some(None),
            },
        )
        .await
        .expect("update");
        assert_eq!(updated.name, "reviewer");
        assert_eq!(updated.reasoning_effort, "low");
        assert!(updated.system_prompt.is_none());
        delete_agent_profile_with(&pool, &created.id)
            .await
            .expect("delete");
        assert!(list_agent_profiles_with(&pool)
            .await
            .expect("empty")
            .is_empty());
    }

    #[tokio::test]
    async fn rejects_missing_channel_and_empty_model() {
        let pool = setup_migrated_pool().await;
        let err = create_agent_profile_with(
            &pool,
            CreateAgentProfile {
                name: "x".to_string(),
                ai_channel_id: "missing".to_string(),
                model: "gpt-4o".to_string(),
                reasoning_effort: None,
                system_prompt: None,
            },
        )
        .await
        .expect_err("missing channel");
        assert!(err.contains("不存在") || err.contains("渠道"));
        let channel_id = seed_channel(&pool).await;
        let err = create_agent_profile_with(
            &pool,
            CreateAgentProfile {
                name: "x".to_string(),
                ai_channel_id: channel_id,
                model: "   ".to_string(),
                reasoning_effort: None,
                system_prompt: None,
            },
        )
        .await
        .expect_err("empty model");
        assert!(err.contains("模型不能为空"));
    }
}
