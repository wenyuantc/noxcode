#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::db::models::{default_channel_input_types, ChannelModelConfig};

const CATALOG_JSON: &str = include_str!("model_catalog.json");

/// 模型输入类型合法集合，顺序即规范化后的存储顺序。
pub const CHANNEL_INPUT_TYPES: [&str; 3] = ["text", "image", "video"];
pub const INPUT_TYPE_TEXT: &str = "text";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub vendor: String,
    pub label: String,
    pub context_tokens: u32,
    pub max_output_tokens: u32,
    pub thinking: bool,
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    /// 目录声明的输入类型；未声明视为仅文本。
    #[serde(default = "default_channel_input_types")]
    pub input_types: Vec<String>,
}

fn catalog_entries() -> &'static [ModelCatalogEntry] {
    use std::sync::OnceLock;
    static ENTRIES: OnceLock<Vec<ModelCatalogEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        serde_json::from_str(CATALOG_JSON).expect("bundled model catalog must parse")
    })
}

pub fn list_catalog() -> Vec<ModelCatalogEntry> {
    catalog_entries().to_vec()
}

fn normalize_model_key(value: &str) -> String {
    value
        .trim()
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(value)
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase()
}

pub fn lookup_catalog(model_id: &str) -> Option<&'static ModelCatalogEntry> {
    let raw = model_id.trim();
    if raw.is_empty() {
        return None;
    }
    let key = normalize_model_key(raw);
    let entries = catalog_entries();
    if let Some(exact) = entries.iter().find(|entry| {
        entry.id.eq_ignore_ascii_case(raw)
            || entry
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(raw))
    }) {
        return Some(exact);
    }
    if let Some(exact_key) = entries.iter().find(|entry| {
        normalize_model_key(&entry.id) == key
            || entry
                .aliases
                .iter()
                .any(|alias| normalize_model_key(alias) == key)
    }) {
        return Some(exact_key);
    }
    entries
        .iter()
        .filter(|entry| {
            let catalog_key = normalize_model_key(&entry.id);
            catalog_key.len() >= 6
                && (key.starts_with(&catalog_key) || catalog_key.starts_with(&key))
        })
        .max_by_key(|entry| normalize_model_key(&entry.id).len())
}

const FALLBACK_THINKING_LEVELS: [&str; 3] = ["low", "medium", "high"];

pub fn unique_thinking_levels(levels: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut next = Vec::new();
    for level in levels {
        let trimmed = level.as_ref().trim();
        if trimmed.is_empty() || seen.contains(trimmed) {
            continue;
        }
        seen.insert(trimmed.to_string());
        next.push(trimmed.to_string());
    }
    next
}

fn catalog_thinking_levels(entry: Option<&ModelCatalogEntry>) -> Vec<String> {
    if let Some(entry) = entry {
        let levels = unique_thinking_levels(&entry.thinking_levels);
        if !levels.is_empty() {
            return levels;
        }
    }
    FALLBACK_THINKING_LEVELS
        .iter()
        .map(|level| (*level).to_string())
        .collect()
}

pub fn default_thinking_level(levels: &[String], preferred: Option<&str>) -> Option<String> {
    if levels.is_empty() {
        return None;
    }
    if let Some(current) = preferred.map(str::trim).filter(|item| !item.is_empty()) {
        if levels.iter().any(|item| item == current) {
            return Some(current.to_string());
        }
    }
    levels
        .iter()
        .find(|level| *level == "medium")
        .cloned()
        .or_else(|| levels.first().cloned())
}

pub fn selected_thinking_levels(config: &ChannelModelConfig) -> Vec<String> {
    match config.thinking_levels.as_ref() {
        Some(levels) => unique_thinking_levels(levels),
        None => catalog_thinking_levels(lookup_catalog(&config.id)),
    }
}

fn normalize_thinking_config(config: &mut ChannelModelConfig) {
    if let Some(levels) = config.thinking_levels.as_mut() {
        *levels = unique_thinking_levels(levels.iter());
    }
    if config.thinking_enabled == Some(false) {
        config.thinking_level = None;
        return;
    }
    if config.thinking_enabled != Some(true) {
        if let Some(level) = config.thinking_level.as_deref() {
            let trimmed = level.trim();
            if trimmed.is_empty() {
                config.thinking_level = None;
            } else if trimmed != level {
                config.thinking_level = Some(trimmed.to_string());
            }
        }
        return;
    }
    let allowed = selected_thinking_levels(config);
    if config.thinking_levels.is_none() {
        config.thinking_levels = Some(allowed.clone());
    }
    config.thinking_level =
        default_thinking_level(allowed.as_slice(), config.thinking_level.as_deref());
}

/// 规范输入类型：去空白、小写、去重、丢弃未知值，按 text / image / video 排序并保证含 text。
pub fn normalize_input_types(values: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let selected: Vec<String> = values
        .into_iter()
        .map(|value| value.as_ref().trim().to_ascii_lowercase())
        .collect();
    CHANNEL_INPUT_TYPES
        .iter()
        .filter(|kind| **kind == INPUT_TYPE_TEXT || selected.iter().any(|item| item == *kind))
        .map(|kind| (*kind).to_string())
        .collect()
}

/// 写入前拒绝未知输入类型；空白项忽略。
fn validate_input_types(config: &ChannelModelConfig) -> Result<(), String> {
    for value in config.input_types.iter().flatten() {
        let key = value.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        if !CHANNEL_INPUT_TYPES.contains(&key.as_str()) {
            return Err(format!(
                "模型「{}」的输入类型「{}」不受支持",
                config.id,
                value.trim()
            ));
        }
    }
    Ok(())
}

pub fn apply_catalog_defaults(model_id: &str) -> ChannelModelConfig {
    let mut config = ChannelModelConfig {
        id: model_id.trim().to_string(),
        context_tokens: None,
        max_output_tokens: None,
        thinking_enabled: None,
        thinking_level: None,
        thinking_levels: None,
        input_types: None,
    };
    fill_from_catalog(&mut config);
    config
}

pub fn fill_from_catalog(config: &mut ChannelModelConfig) {
    config.id = config.id.trim().to_string();
    if let Some(entry) = lookup_catalog(&config.id) {
        if config.context_tokens.is_none() {
            config.context_tokens = Some(entry.context_tokens);
        }
        if config.max_output_tokens.is_none() {
            config.max_output_tokens = Some(entry.max_output_tokens);
        }
        if config.thinking_enabled.is_none() {
            config.thinking_enabled = Some(entry.thinking);
        }
        if config.thinking_levels.is_none() {
            config.thinking_levels = Some(entry.thinking_levels.clone());
        }
        if config.input_types.is_none() {
            config.input_types = Some(entry.input_types.clone());
        }
    }
    normalize_thinking_config(config);
    // 未设置且目录里没有的模型回退仅文本；已设置的集合只做规范化。
    config.input_types = Some(normalize_input_types(
        config.input_types.as_deref().unwrap_or_default(),
    ));
}

pub fn validate_channel_model_config(config: &ChannelModelConfig) -> Result<(), String> {
    if config.thinking_enabled == Some(true) && selected_thinking_levels(config).is_empty() {
        return Err(format!("模型「{}」开启思考时请至少勾选一个等级", config.id));
    }
    Ok(())
}

pub fn normalize_channel_model_config(config: &mut ChannelModelConfig) -> Result<(), String> {
    validate_input_types(config)?;
    fill_from_catalog(config);
    validate_channel_model_config(config)
}

pub fn resolve_runtime_reasoning_effort(
    config: &ChannelModelConfig,
    reasoning_effort: Option<&str>,
) -> Option<String> {
    if config.thinking_enabled != Some(true) {
        return None;
    }
    let allowed = selected_thinking_levels(config);
    let requested = reasoning_effort
        .map(str::trim)
        .filter(|item| !item.is_empty());
    if let Some(current) = requested {
        if allowed.iter().any(|item| item == current) {
            return Some(current.to_string());
        }
    }
    default_thinking_level(allowed.as_slice(), config.thinking_level.as_deref())
}

#[tauri::command]
pub fn list_model_catalog() -> Vec<ModelCatalogEntry> {
    list_catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_requested_vendors() {
        let catalog = list_catalog();
        let vendors: Vec<&str> = catalog.iter().map(|item| item.vendor.as_str()).collect();
        for vendor in [
            "openai",
            "anthropic",
            "deepseek",
            "minimax",
            "glm",
            "kimi",
            "doubao",
            "hunyuan",
            "gemini",
            "mimo",
        ] {
            assert!(vendors.contains(&vendor), "missing vendor {vendor}");
        }
    }

    #[test]
    fn lookup_matches_aliases_and_prefixed_ids() {
        assert_eq!(lookup_catalog("gpt-4o").unwrap().context_tokens, 128000);
        assert_eq!(
            lookup_catalog("deepseek-ai/DeepSeek-V3").unwrap().id,
            "deepseek-chat"
        );
        assert_eq!(
            lookup_catalog("claude-sonnet-4-6-20260217").unwrap().id,
            "claude-sonnet-4-6"
        );
        assert!(lookup_catalog("totally-unknown-model-xyz").is_none());
    }

    #[test]
    fn newer_point_releases_resolve_to_their_own_entry() {
        assert_eq!(lookup_catalog("claude-fable-5-1").unwrap().id, "claude-fable-5-1");
        assert_eq!(lookup_catalog("fable-5.1").unwrap().id, "claude-fable-5-1");
        assert_eq!(lookup_catalog("claude-fable-5").unwrap().id, "claude-fable-5");
        assert_eq!(lookup_catalog("gemini-3.8-flash").unwrap().id, "gemini-3.8-flash");
        assert_eq!(
            apply_catalog_defaults("gemini-3.8-flash").input_types,
            types(&["text", "image", "video"])
        );
    }

    #[test]
    fn gpt_5_6_luna_includes_xhigh_and_max() {
        let entry = lookup_catalog("gpt-5.6-luna").expect("luna");
        assert!(entry.thinking_levels.contains(&"xhigh".to_string()));
        assert!(entry.thinking_levels.contains(&"max".to_string()));
        let defaults = apply_catalog_defaults("gpt-5.6-luna");
        assert!(defaults
            .thinking_levels
            .as_ref()
            .is_some_and(|levels| levels.contains(&"xhigh".to_string())
                && levels.contains(&"max".to_string())));
    }

    #[test]
    fn fill_from_catalog_keeps_explicit_thinking_level_subset() {
        let mut config = apply_catalog_defaults("gpt-5.6-luna");
        config.thinking_levels = Some(vec!["low".to_string(), "high".to_string()]);
        config.thinking_level = Some("high".to_string());
        fill_from_catalog(&mut config);
        assert_eq!(
            config.thinking_levels.as_deref(),
            Some(["low".to_string(), "high".to_string()].as_slice())
        );
        assert_eq!(config.thinking_level.as_deref(), Some("high"));
    }

    #[test]
    fn fill_from_catalog_keeps_explicit_empty_thinking_levels() {
        let mut config = apply_catalog_defaults("deepseek-reasoner");
        config.thinking_enabled = Some(false);
        config.thinking_levels = Some(vec![]);
        config.thinking_level = Some("medium".to_string());
        fill_from_catalog(&mut config);
        assert_eq!(config.thinking_levels.as_deref(), Some([].as_slice()));
        assert_eq!(config.thinking_level, None);
    }

    #[test]
    fn fill_from_catalog_falls_back_when_default_level_is_not_allowed() {
        let mut config = apply_catalog_defaults("deepseek-reasoner");
        config.thinking_enabled = Some(true);
        config.thinking_levels = Some(vec!["low".to_string(), "high".to_string()]);
        config.thinking_level = Some("medium".to_string());
        fill_from_catalog(&mut config);
        assert_eq!(config.thinking_level.as_deref(), Some("low"));
    }

    fn unknown_thinking_on() -> ChannelModelConfig {
        ChannelModelConfig {
            id: "custom-local-model".to_string(),
            context_tokens: None,
            max_output_tokens: None,
            thinking_enabled: Some(true),
            thinking_level: Some("high".to_string()),
            thinking_levels: None,
            input_types: None,
        }
    }

    fn types(values: &[&str]) -> Option<Vec<String>> {
        Some(values.iter().map(|item| (*item).to_string()).collect())
    }

    #[test]
    fn input_types_default_to_text_and_normalize() {
        // 目录声明多模态的模型采用目录；文本模型与未知模型回退仅文本。
        assert_eq!(
            apply_catalog_defaults("gpt-4o").input_types,
            types(&["text", "image"])
        );
        assert_eq!(
            apply_catalog_defaults("gemini-2.5-pro").input_types,
            types(&["text", "image", "video"])
        );
        assert_eq!(
            apply_catalog_defaults("deepseek-chat").input_types,
            types(&["text"])
        );
        assert_eq!(
            apply_catalog_defaults("custom-local-model").input_types,
            types(&["text"])
        );

        // 用户已保存的集合不被目录覆盖。
        let mut config = apply_catalog_defaults("gpt-4o");
        config.input_types = types(&["text"]);
        fill_from_catalog(&mut config);
        assert_eq!(config.input_types, types(&["text"]));

        assert_eq!(
            normalize_input_types(["video", " Image ", "text", "text"]),
            vec!["text".to_string(), "image".to_string(), "video".to_string()]
        );
        assert_eq!(normalize_input_types(["image"]), vec!["text", "image"]);
        assert_eq!(normalize_input_types(Vec::<String>::new()), vec!["text"]);
        assert_eq!(normalize_input_types(["audio", "text"]), vec!["text"]);
    }

    #[test]
    fn normalize_rejects_unknown_input_types_but_keeps_known_selection() {
        let mut config = apply_catalog_defaults("gpt-4o");
        config.input_types = types(&["audio"]);
        assert_eq!(
            normalize_channel_model_config(&mut config).unwrap_err(),
            "模型「gpt-4o」的输入类型「audio」不受支持"
        );

        let mut config = apply_catalog_defaults("gpt-4o");
        config.input_types = types(&["image", "video"]);
        normalize_channel_model_config(&mut config).expect("known types are valid");
        assert_eq!(config.input_types, types(&["text", "image", "video"]));
    }

    #[test]
    fn catalog_input_types_are_known_values() {
        for entry in list_catalog() {
            assert!(
                entry.input_types.contains(&INPUT_TYPE_TEXT.to_string()),
                "{}",
                entry.id
            );
            for kind in &entry.input_types {
                assert!(
                    CHANNEL_INPUT_TYPES.contains(&kind.as_str()),
                    "{}: {kind}",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn unknown_model_null_levels_materialize_fallback_set() {
        let mut config = unknown_thinking_on();
        normalize_channel_model_config(&mut config).expect("legacy unknown model is valid");
        assert_eq!(
            config.thinking_levels.as_deref(),
            Some(["low".to_string(), "medium".to_string(), "high".to_string()].as_slice())
        );
        assert_eq!(config.thinking_level.as_deref(), Some("high"));
    }

    #[test]
    fn thinking_on_rejects_explicit_empty_levels() {
        let mut config = apply_catalog_defaults("deepseek-reasoner");
        config.thinking_enabled = Some(true);
        config.thinking_levels = Some(vec![]);
        assert_eq!(
            normalize_channel_model_config(&mut config).unwrap_err(),
            "模型「deepseek-reasoner」开启思考时请至少勾选一个等级"
        );

        let mut unknown = unknown_thinking_on();
        unknown.thinking_levels = Some(vec![]);
        assert!(normalize_channel_model_config(&mut unknown).is_err());
    }

    #[test]
    fn runtime_effort_falls_back_when_channel_subset_shrinks() {
        let mut config = apply_catalog_defaults("gpt-5.6-luna");
        config.thinking_enabled = Some(true);
        config.thinking_levels = Some(vec!["low".to_string(), "high".to_string()]);
        config.thinking_level = Some("high".to_string());
        fill_from_catalog(&mut config);
        assert_eq!(
            resolve_runtime_reasoning_effort(&config, Some("max")).as_deref(),
            Some("high")
        );
        assert_eq!(
            resolve_runtime_reasoning_effort(&config, Some("low")).as_deref(),
            Some("low")
        );
        assert_eq!(
            resolve_runtime_reasoning_effort(&config, None).as_deref(),
            Some("high")
        );
    }
}
