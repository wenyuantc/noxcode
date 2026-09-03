use serde_json::{Map, Value};

use crate::app::shared::normalize_optional_text;
use crate::db::models::{AiChannel, AiChannelRecord, ChannelModelConfig};
use crate::native::model_catalog::fill_from_catalog;

pub const PROTOCOL_OPENAI: &str = "openai";
pub const PROTOCOL_ANTHROPIC: &str = "anthropic";
pub const PROTOCOL_CODEX: &str = "codex";

pub fn normalize_protocol(value: &str) -> Result<&'static str, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai-compatible" | "openai_compatible" => Ok(PROTOCOL_OPENAI),
        "anthropic" | "claude" => Ok(PROTOCOL_ANTHROPIC),
        "codex" | "responses" | "openai-responses" => Ok(PROTOCOL_CODEX),
        _ => Err("渠道协议必须是 openai、anthropic 或 codex".to_string()),
    }
}

pub fn normalize_base_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err("渠道 Base URL 不能为空".to_string());
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("渠道 Base URL 必须以 http:// 或 https:// 开头".to_string());
    }
    Ok(trimmed)
}

pub fn channel_chat_url(base_url: &str, protocol: &str) -> Result<String, String> {
    let base = normalize_base_url(base_url)?;
    let path = match protocol {
        PROTOCOL_OPENAI => "/v1/chat/completions",
        PROTOCOL_ANTHROPIC => "/v1/messages",
        PROTOCOL_CODEX => "/v1/responses",
        _ => return Err("渠道协议必须是 openai、anthropic 或 codex".to_string()),
    };
    Ok(format!("{base}{path}"))
}

pub fn channel_models_url(base_url: &str) -> Result<String, String> {
    let base = normalize_base_url(base_url)?;
    Ok(format!("{base}/v1/models"))
}

fn push_model_id(models: &mut Vec<String>, value: &str) {
    let id = value.trim();
    if !id.is_empty() && !models.iter().any(|existing| existing == id) {
        models.push(id.to_string());
    }
}

fn push_model_item(models: &mut Vec<String>, item: &Value) {
    match item {
        Value::String(id) => push_model_id(models, id),
        Value::Object(object) => {
            for key in ["id", "model", "name", "model_id"] {
                if let Some(Value::String(id)) = object.get(key) {
                    push_model_id(models, id);
                    return;
                }
            }
        }
        _ => {}
    }
}

/// Parse GET /v1/models JSON from OpenAI, Anthropic, or OpenAI-compatible gateways.
pub fn parse_model_list_json(raw: &str) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| "模型列表响应不是合法 JSON".to_string())?;
    let mut models = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            push_model_item(&mut models, item);
        }
    } else if let Some(object) = value.as_object() {
        for key in ["data", "models", "list"] {
            if let Some(items) = object.get(key).and_then(Value::as_array) {
                for item in items {
                    push_model_item(&mut models, item);
                }
                break;
            }
        }
    }
    Ok(models)
}

pub fn model_list_next_page(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    if object.get("has_more").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    if let Some(id) = object.get("last_id").and_then(Value::as_str) {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    object
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.last())
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
}

pub fn parse_channel_models_json(raw: &str) -> Result<Vec<ChannelModelConfig>, String> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Value =
        serde_json::from_str(raw).map_err(|_| "模型列表不是合法 JSON".to_string())?;
    let list = parsed
        .as_array()
        .ok_or_else(|| "模型列表必须是数组".to_string())?;
    let mut models = Vec::new();
    for item in list {
        let mut config = match item {
            Value::String(id) => ChannelModelConfig {
                id: id.trim().to_string(),
                context_tokens: None,
                max_output_tokens: None,
                thinking_enabled: None,
                thinking_level: None,
                thinking_levels: None,
            },
            Value::Object(_) => serde_json::from_value(item.clone())
                .map_err(|_| "模型配置必须包含 id".to_string())?,
            _ => return Err("模型列表必须是字符串或对象数组".to_string()),
        };
        config.id = config.id.trim().to_string();
        if config.id.is_empty() {
            continue;
        }
        if models
            .iter()
            .any(|existing: &ChannelModelConfig| existing.id == config.id)
        {
            continue;
        }
        fill_from_catalog(&mut config);
        models.push(config);
    }
    Ok(models)
}

pub fn serialize_channel_models(models: &[ChannelModelConfig]) -> String {
    let cleaned: Vec<ChannelModelConfig> = models
        .iter()
        .map(|item| {
            let mut config = item.clone();
            config.id = config.id.trim().to_string();
            config
        })
        .filter(|item| !item.id.is_empty())
        .collect();
    serde_json::to_string(&cleaned).unwrap_or_else(|_| "[]".to_string())
}

pub fn normalize_extra_headers_json(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = raw.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok(None);
    };
    let parsed: Value =
        serde_json::from_str(text).map_err(|_| "额外请求头必须是 JSON 对象".to_string())?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "额外请求头必须是 JSON 对象".to_string())?;
    let mut headers = Map::new();
    for (key, value) in object {
        let name = key.trim();
        if name.is_empty() {
            continue;
        }
        let header_value = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => return Err("额外请求头的值必须是字符串".to_string()),
        };
        headers.insert(name.to_string(), Value::String(header_value));
    }
    if headers.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&headers)
        .map(Some)
        .map_err(|_| "额外请求头必须是 JSON 对象".to_string())
}

fn optional_secret(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

pub fn record_to_channel(record: AiChannelRecord) -> Result<AiChannel, String> {
    let api_key = optional_secret(record.api_key.as_deref());
    let api_key_configured = api_key.is_some();
    Ok(AiChannel {
        id: record.id,
        name: record.name,
        protocol: record.protocol,
        base_url: record.base_url,
        extra_headers_json: record.extra_headers_json,
        models: parse_channel_models_json(&record.models_json)?,
        lite_model: normalize_optional_text(record.lite_model.as_deref()),
        enabled: record.enabled != 0,
        api_key,
        api_key_configured,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_aliases_normalize() {
        assert_eq!(normalize_protocol("OpenAI").unwrap(), PROTOCOL_OPENAI);
        assert_eq!(normalize_protocol("claude").unwrap(), PROTOCOL_ANTHROPIC);
        assert_eq!(normalize_protocol("responses").unwrap(), PROTOCOL_CODEX);
        assert!(normalize_protocol("gemini").is_err());
    }

    #[test]
    fn models_url_is_shared_across_protocols() {
        assert_eq!(
            channel_models_url("https://api.example.com/").unwrap(),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn parse_model_list_accepts_openai_and_anthropic_shapes() {
        let openai =
            parse_model_list_json(r#"{"data":[{"id":"gpt-4o"},{"id":"gpt-4o"}]}"#).unwrap();
        assert_eq!(openai, vec!["gpt-4o".to_string()]);
        let anthropic =
            parse_model_list_json(r#"{"data":[{"id":"claude-sonnet-4","display_name":"Sonnet"}]}"#)
                .unwrap();
        assert_eq!(anthropic, vec!["claude-sonnet-4".to_string()]);
        let proxy = parse_model_list_json(r#"{"models":[{"name":"deepseek-chat"}]}"#).unwrap();
        assert_eq!(proxy, vec!["deepseek-chat".to_string()]);
        assert!(parse_model_list_json(r#"{"data":[]}"#).unwrap().is_empty());
        assert!(parse_model_list_json("not-json").is_err());
    }

    #[test]
    fn model_list_pagination_reads_last_id() {
        assert_eq!(
            model_list_next_page(r#"{"data":[{"id":"m1"}],"has_more":true,"last_id":"m1"}"#)
                .as_deref(),
            Some("m1")
        );
        assert!(model_list_next_page(r#"{"data":[{"id":"m1"}],"has_more":false}"#).is_none());
    }

    #[test]
    fn chat_urls_match_protocol_defaults() {
        assert_eq!(
            channel_chat_url("https://api.example.com/", PROTOCOL_OPENAI).unwrap(),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            channel_chat_url("https://api.example.com", PROTOCOL_ANTHROPIC).unwrap(),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            channel_chat_url("https://api.example.com", PROTOCOL_CODEX).unwrap(),
            "https://api.example.com/v1/responses"
        );
    }

    #[test]
    fn models_json_roundtrip_dedupes() {
        let parsed = parse_channel_models_json(r#"[" gpt-4o ", "gpt-4o", ""]"#).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "gpt-4o");
        let encoded = serialize_channel_models(&parsed);
        assert!(encoded.contains("gpt-4o"));
    }

    #[test]
    fn channel_models_json_accepts_ids_and_objects() {
        let from_ids = parse_channel_models_json(r#"["gpt-4o","gpt-4o"]"#).unwrap();
        assert_eq!(from_ids.len(), 1);
        assert_eq!(from_ids[0].id, "gpt-4o");
        assert_eq!(from_ids[0].context_tokens, Some(128000));
        let from_objects = parse_channel_models_json(
            r#"[{"id":"deepseek-reasoner","context_tokens":64000,"thinking_enabled":true}]"#,
        )
        .unwrap();
        assert_eq!(from_objects[0].context_tokens, Some(64000));
        assert_eq!(from_objects[0].thinking_enabled, Some(true));
        assert!(from_objects[0].max_output_tokens.is_some());
        let subset = parse_channel_models_json(
            r#"[{"id":"gpt-5.6-luna","thinking_enabled":true,"thinking_level":"high","thinking_levels":["low","high"]}]"#,
        )
        .unwrap();
        assert_eq!(
            subset[0].thinking_levels.as_deref(),
            Some(["low".to_string(), "high".to_string()].as_slice())
        );
        assert_eq!(subset[0].thinking_level.as_deref(), Some("high"));
        let unknown = parse_channel_models_json(
            r#"[{"id":"custom-local-model","thinking_enabled":true,"thinking_level":"high"}]"#,
        )
        .unwrap();
        assert_eq!(
            unknown[0].thinking_levels.as_deref(),
            Some(["low".to_string(), "medium".to_string(), "high".to_string()].as_slice())
        );
        assert_eq!(unknown[0].thinking_level.as_deref(), Some("high"));
    }

    #[test]
    fn extra_headers_require_object() {
        assert!(normalize_extra_headers_json(Some("[1]")).is_err());
        let stored = normalize_extra_headers_json(Some(r#"{"X-Test":"a"}"#))
            .unwrap()
            .unwrap();
        assert!(stored.contains("X-Test"));
    }

    fn sample_record() -> AiChannelRecord {
        AiChannelRecord {
            id: "c1".to_string(),
            name: "demo".to_string(),
            protocol: PROTOCOL_OPENAI.to_string(),
            base_url: "https://api.example.com".to_string(),
            api_key: None,
            extra_headers_json: None,
            models_json: "[]".to_string(),
            enabled: 1,
            created_at: "2026-08-20 00:00:00".to_string(),
            updated_at: "2026-08-20 00:00:00".to_string(),
            lite_model: None,
        }
    }

    #[test]
    fn channel_dto_includes_api_key_from_column() {
        let mut record = sample_record();
        record.api_key = Some(" sk-live ".to_string());
        let json = serde_json::to_value(record_to_channel(record).unwrap()).unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(object.get("api_key"), Some(&serde_json::json!("sk-live")));
        assert!(!object.contains_key("api_key_ref"));
        assert_eq!(
            object.get("api_key_configured"),
            Some(&serde_json::json!(true))
        );
    }

    #[test]
    fn channel_dto_unconfigured_without_api_key() {
        let record = sample_record();
        let json = serde_json::to_value(record_to_channel(record).unwrap()).unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(object.get("api_key"), Some(&serde_json::json!(null)));
        assert_eq!(
            object.get("api_key_configured"),
            Some(&serde_json::json!(false))
        );
    }
}
