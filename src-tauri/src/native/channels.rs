use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};

use crate::app::network_settings::{load_network_settings, NetworkSettings};
use crate::app::shared::{new_id, normalize_optional_text, now_sqlite, sqlite_pool};
use crate::db::models::{
    AiChannel, AiChannelRecord, CreateAiChannel, ListAiChannelModelsResult, TestAiChannelPayload,
    TestAiChannelResult, UpdateAiChannel,
};
use crate::native::model::{ModelClient, ModelClientConfig, RetryConfig};
use crate::native::model_catalog::normalize_channel_model_config;
use crate::native::protocol::{
    normalize_base_url, normalize_extra_headers_json, normalize_protocol,
    parse_channel_models_json, record_to_channel, serialize_channel_models,
};

fn normalize_channel_name(value: &str) -> Result<String, String> {
    let name = value.trim().to_string();
    if name.is_empty() {
        return Err("渠道名称不能为空".to_string());
    }
    Ok(name)
}

pub(crate) async fn fetch_channel_record(
    pool: &SqlitePool,
    id: &str,
) -> Result<AiChannelRecord, String> {
    sqlx::query_as::<_, AiChannelRecord>("SELECT * FROM ai_channels WHERE id = $1 LIMIT 1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|_| format!("渠道 {id} 不存在"))
}

fn non_empty_secret(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn channel_stored_api_key(record: &AiChannelRecord) -> Option<String> {
    non_empty_secret(record.api_key.as_deref())
}

#[allow(dead_code)]
pub(crate) fn require_channel_api_key(record: &AiChannelRecord) -> Result<String, String> {
    channel_stored_api_key(record).ok_or_else(|| "渠道未配置 API 密钥".to_string())
}

async fn fetch_channel(pool: &SqlitePool, id: &str) -> Result<AiChannel, String> {
    record_to_channel(fetch_channel_record(pool, id).await?)
}

fn parse_header_map(raw: Option<&str>) -> Result<HashMap<String, String>, String> {
    let Some(json) = normalize_extra_headers_json(raw)? else {
        return Ok(HashMap::new());
    };
    let value: Value =
        serde_json::from_str(&json).map_err(|_| "额外请求头必须是 JSON 对象".to_string())?;
    let mut headers = HashMap::new();
    if let Some(object) = value.as_object() {
        for (key, item) in object {
            if let Some(text) = item.as_str() {
                headers.insert(key.clone(), text.to_string());
            }
        }
    }
    Ok(headers)
}

struct ResolvedChannelHttp {
    protocol: String,
    base_url: String,
    extra_headers_json: Option<String>,
    api_key: String,
    first_model: Option<String>,
    network: NetworkSettings,
}

async fn resolve_channel_http<R: Runtime>(
    app: &AppHandle<R>,
    payload: &TestAiChannelPayload,
) -> Result<ResolvedChannelHttp, String> {
    let pool = sqlite_pool(app).await?;
    let stored = if let Some(id) = payload
        .id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        Some(fetch_channel_record(&pool, id).await?)
    } else {
        None
    };

    let protocol = normalize_protocol(
        payload
            .protocol
            .as_deref()
            .or(stored.as_ref().map(|item| item.protocol.as_str()))
            .unwrap_or(""),
    )?
    .to_string();
    let base_url = normalize_base_url(
        payload
            .base_url
            .as_deref()
            .or(stored.as_ref().map(|item| item.base_url.as_str()))
            .unwrap_or(""),
    )?;
    let extra_headers_json = payload.extra_headers_json.clone().or_else(|| {
        stored
            .as_ref()
            .and_then(|item| item.extra_headers_json.clone())
    });
    let api_key = match normalize_optional_text(payload.api_key.as_deref()) {
        Some(value) => value,
        None => stored
            .as_ref()
            .and_then(channel_stored_api_key)
            .ok_or_else(|| "请先填写渠道 API 密钥".to_string())?,
    };
    Ok(ResolvedChannelHttp {
        protocol,
        base_url,
        extra_headers_json,
        api_key,
        first_model: stored.as_ref().and_then(|item| {
            parse_channel_models_json(&item.models_json)
                .ok()
                .and_then(|models| models.into_iter().next().map(|model| model.id))
        }),
        network: load_network_settings(app)?,
    })
}

fn channel_client(
    protocol: &str,
    base_url: &str,
    api_key: &str,
    extra_headers_json: Option<&str>,
    network: &NetworkSettings,
) -> Result<ModelClient, String> {
    ModelClient::new(ModelClientConfig {
        protocol: protocol.to_string(),
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        extra_headers: parse_header_map(extra_headers_json)?,
        retry: RetryConfig::none(),
        timeout: Duration::from_secs(20),
        network: network.clone(),
    })
}

async fn send_probe_request(
    protocol: &str,
    base_url: &str,
    api_key: &str,
    extra_headers_json: Option<&str>,
    network: &NetworkSettings,
    model: &str,
) -> Result<TestAiChannelResult, String> {
    let client = channel_client(protocol, base_url, api_key, extra_headers_json, network)?;
    match client.probe(model).await {
        Ok(()) => Ok(TestAiChannelResult {
            ok: true,
            status: Some(200),
            message: "渠道测通成功".to_string(),
        }),
        Err(message) => Ok(TestAiChannelResult {
            ok: false,
            status: None,
            message,
        }),
    }
}

pub(crate) async fn list_ai_channels_with(pool: &SqlitePool) -> Result<Vec<AiChannel>, String> {
    let records = sqlx::query_as::<_, AiChannelRecord>(
        "SELECT * FROM ai_channels ORDER BY enabled DESC, name COLLATE NOCASE, created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取渠道列表失败: {error}"))?;
    let mut channels = Vec::with_capacity(records.len());
    for record in records {
        channels.push(record_to_channel(record)?);
    }
    Ok(channels)
}

pub(crate) async fn create_ai_channel_with(
    pool: &SqlitePool,
    payload: CreateAiChannel,
) -> Result<AiChannel, String> {
    let name = normalize_channel_name(&payload.name)?;
    let protocol = normalize_protocol(&payload.protocol)?.to_string();
    let base_url = normalize_base_url(&payload.base_url)?;
    let extra_headers_json = normalize_extra_headers_json(payload.extra_headers_json.as_deref())?;
    let mut models = payload.models.unwrap_or_default();
    for model in &mut models {
        normalize_channel_model_config(model)?;
    }
    let models_json = serialize_channel_models(&models);
    let enabled = i64::from(payload.enabled.unwrap_or(true));
    let id = new_id();
    let now = now_sqlite();
    let api_key = normalize_optional_text(payload.api_key.as_deref());

    sqlx::query(
        "INSERT INTO ai_channels (id, name, protocol, base_url, api_key, extra_headers_json, models_json, enabled, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&protocol)
    .bind(&base_url)
    .bind(&api_key)
    .bind(&extra_headers_json)
    .bind(&models_json)
    .bind(enabled)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| format!("创建渠道失败: {error}"))?;

    fetch_channel(pool, &id).await
}

pub(crate) async fn update_ai_channel_with(
    pool: &SqlitePool,
    id: &str,
    updates: UpdateAiChannel,
) -> Result<AiChannel, String> {
    let current = fetch_channel_record(pool, id).await?;
    let name = match updates.name.as_deref() {
        Some(value) => normalize_channel_name(value)?,
        None => current.name.clone(),
    };
    let protocol = match updates.protocol.as_deref() {
        Some(value) => normalize_protocol(value)?.to_string(),
        None => normalize_protocol(&current.protocol)?.to_string(),
    };
    let base_url = match updates.base_url.as_deref() {
        Some(value) => normalize_base_url(value)?,
        None => current.base_url.clone(),
    };
    let extra_headers_json = match updates.extra_headers_json {
        Some(Some(value)) => normalize_extra_headers_json(Some(&value))?,
        Some(None) => None,
        None => current.extra_headers_json.clone(),
    };
    let models_json = match updates.models {
        Some(mut models) => {
            for model in &mut models {
                normalize_channel_model_config(model)?;
            }
            serialize_channel_models(&models)
        }
        None => current.models_json.clone(),
    };
    let enabled = updates.enabled.map(i64::from).unwrap_or(current.enabled);
    let now = now_sqlite();
    let incoming_key = normalize_optional_text(updates.api_key.as_deref());
    let api_key = if let Some(secret) = incoming_key {
        Some(secret)
    } else {
        current.api_key.clone()
    };

    sqlx::query(
        "UPDATE ai_channels SET name = $1, protocol = $2, base_url = $3, api_key = $4, extra_headers_json = $5, models_json = $6, enabled = $7, updated_at = $8 WHERE id = $9",
    )
    .bind(&name)
    .bind(&protocol)
    .bind(&base_url)
    .bind(&api_key)
    .bind(&extra_headers_json)
    .bind(&models_json)
    .bind(enabled)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新渠道失败: {error}"))?;

    fetch_channel(pool, id).await
}

pub(crate) async fn delete_ai_channel_with(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let current = fetch_channel_record(pool, id).await?;
    let referenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_profiles WHERE ai_channel_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(|error| format!("检查渠道引用失败: {error}"))?;
    if referenced > 0 {
        return Err(format!(
            "渠道「{}」仍被 {} 个 Agent 档案使用，无法删除",
            current.name, referenced
        ));
    }

    sqlx::query("DELETE FROM ai_channels WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| format!("删除渠道失败: {error}"))?;
    Ok(())
}

#[tauri::command]
pub async fn list_ai_channels<R: Runtime>(app: AppHandle<R>) -> Result<Vec<AiChannel>, String> {
    let pool = sqlite_pool(&app).await?;
    list_ai_channels_with(&pool).await
}

#[tauri::command]
pub async fn create_ai_channel<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateAiChannel,
) -> Result<AiChannel, String> {
    let pool = sqlite_pool(&app).await?;
    create_ai_channel_with(&pool, payload).await
}

#[tauri::command]
pub async fn update_ai_channel<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    updates: UpdateAiChannel,
) -> Result<AiChannel, String> {
    let pool = sqlite_pool(&app).await?;
    update_ai_channel_with(&pool, &id, updates).await
}

#[tauri::command]
pub async fn delete_ai_channel<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let pool = sqlite_pool(&app).await?;
    delete_ai_channel_with(&pool, &id).await
}

#[tauri::command]
pub async fn test_ai_channel<R: Runtime>(
    app: AppHandle<R>,
    payload: TestAiChannelPayload,
) -> Result<TestAiChannelResult, String> {
    let target = resolve_channel_http(&app, &payload).await?;
    let model = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .or(target.first_model.clone())
        .unwrap_or_else(|| "dummy".to_string());

    send_probe_request(
        &target.protocol,
        &target.base_url,
        &target.api_key,
        target.extra_headers_json.as_deref(),
        &target.network,
        &model,
    )
    .await
}

#[tauri::command]
pub async fn list_ai_channel_models<R: Runtime>(
    app: AppHandle<R>,
    payload: TestAiChannelPayload,
) -> Result<ListAiChannelModelsResult, String> {
    let target = resolve_channel_http(&app, &payload).await?;
    let client = channel_client(
        &target.protocol,
        &target.base_url,
        &target.api_key,
        target.extra_headers_json.as_deref(),
        &target.network,
    )?;
    let listed = client.list_models().await?;
    let models = listed.models;
    let message = if models.is_empty() {
        "网关未返回可用模型，请检查协议、Base URL 和密钥".to_string()
    } else if listed.truncated {
        format!("已获取 {} 个模型（列表已截断，可能不完整）", models.len())
    } else {
        format!("已获取 {} 个模型", models.len())
    };

    Ok(ListAiChannelModelsResult {
        models,
        message,
        truncated: listed.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{ChannelModelConfig, UpdateAiChannel};
    use crate::db::test_support::setup_migrated_pool;

    fn sample_create() -> CreateAiChannel {
        CreateAiChannel {
            name: "demo".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.example.com/".to_string(),
            api_key: Some("sk-live".to_string()),
            extra_headers_json: None,
            models: Some(vec![ChannelModelConfig {
                id: "gpt-4o".to_string(),
                context_tokens: None,
                max_output_tokens: None,
                thinking_enabled: None,
                thinking_level: None,
                thinking_levels: None,
            }]),
            enabled: Some(true),
        }
    }

    #[test]
    fn create_lists_and_fills_catalog_defaults() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let created = create_ai_channel_with(&pool, sample_create())
                .await
                .expect("create");
            assert_eq!(created.name, "demo");
            assert_eq!(created.protocol, "openai");
            assert_eq!(created.base_url, "https://api.example.com");
            assert_eq!(created.api_key.as_deref(), Some("sk-live"));
            assert!(created.api_key_configured);
            assert_eq!(created.models.len(), 1);
            assert_eq!(created.models[0].id, "gpt-4o");
            assert_eq!(created.models[0].context_tokens, Some(128000));

            let listed = list_ai_channels_with(&pool).await.expect("list");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, created.id);
        });
    }

    #[test]
    fn update_without_api_key_keeps_existing() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let created = create_ai_channel_with(&pool, sample_create())
                .await
                .expect("create");
            let updated = update_ai_channel_with(
                &pool,
                &created.id,
                UpdateAiChannel {
                    name: Some("renamed".to_string()),
                    protocol: None,
                    base_url: None,
                    api_key: None,
                    extra_headers_json: None,
                    models: None,
                    enabled: None,
                },
            )
            .await
            .expect("update");
            assert_eq!(updated.name, "renamed");
            assert_eq!(updated.api_key.as_deref(), Some("sk-live"));
            assert!(updated.api_key_configured);
        });
    }

    #[test]
    fn delete_is_rejected_when_profile_references_channel() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let created = create_ai_channel_with(&pool, sample_create())
                .await
                .expect("create");
            let now = now_sqlite();
            sqlx::query(
                "INSERT INTO agent_profiles (id, name, ai_channel_id, model, reasoning_effort, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind("profile-1")
            .bind("档案")
            .bind(&created.id)
            .bind("gpt-4o")
            .bind("high")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("insert profile");

            let err = delete_ai_channel_with(&pool, &created.id)
                .await
                .expect_err("should reject");
            assert!(err.contains("Agent 档案"), "unexpected error: {err}");

            delete_ai_channel_with(&pool, &created.id)
                .await
                .expect_err("still referenced");

            sqlx::query("DELETE FROM agent_profiles WHERE id = $1")
                .bind("profile-1")
                .execute(&pool)
                .await
                .expect("delete profile");
            delete_ai_channel_with(&pool, &created.id)
                .await
                .expect("delete after unlink");
            assert!(list_ai_channels_with(&pool).await.expect("list").is_empty());
        });
    }

    #[test]
    fn require_channel_api_key_reads_column() {
        let mut record = AiChannelRecord {
            id: "c1".to_string(),
            name: "demo".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: None,
            extra_headers_json: None,
            models_json: "[]".to_string(),
            enabled: 1,
            created_at: "2026-08-20 00:00:00".to_string(),
            updated_at: "2026-08-20 00:00:00".to_string(),
        };
        assert!(require_channel_api_key(&record).is_err());
        record.api_key = Some(" sk-live ".to_string());
        assert_eq!(require_channel_api_key(&record).unwrap(), "sk-live");
    }

    #[tokio::test]
    #[ignore]
    async fn live_channel_probe_and_list_models() {
        let protocol =
            std::env::var("NOXCODE_LIVE_CHANNEL_PROTOCOL").expect("NOXCODE_LIVE_CHANNEL_PROTOCOL");
        let base_url =
            std::env::var("NOXCODE_LIVE_CHANNEL_BASE_URL").expect("NOXCODE_LIVE_CHANNEL_BASE_URL");
        let api_key =
            std::env::var("NOXCODE_LIVE_CHANNEL_API_KEY").expect("NOXCODE_LIVE_CHANNEL_API_KEY");
        let model = std::env::var("NOXCODE_LIVE_CHANNEL_MODEL").unwrap_or_else(|_| "dummy".into());
        let client = channel_client(
            normalize_protocol(&protocol).expect("protocol"),
            &normalize_base_url(&base_url).expect("base url"),
            &api_key,
            None,
            &NetworkSettings::default(),
        )
        .expect("client");
        client.probe(&model).await.expect("live probe");
        let listed = client.list_models().await.expect("live list models");
        assert!(
            !listed.models.is_empty(),
            "live list_models returned no models"
        );
    }
}
