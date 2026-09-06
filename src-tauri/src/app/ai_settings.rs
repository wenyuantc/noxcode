use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::{normalize_optional_text, sqlite_pool};
use crate::native::channels::fetch_channel_record;
use crate::native::model_catalog::selected_thinking_levels;
use crate::native::protocol::record_to_channel;

const SETTINGS_FILE_NAME: &str = "ai-settings.json";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommitMessageStyle {
    Concise,
    #[default]
    #[serde(other)]
    Detailed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiFeatureOverride {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiCommitMessageSettings {
    #[serde(flatten)]
    pub override_fields: AiFeatureOverride,
    #[serde(default)]
    pub style: CommitMessageStyle,
}

impl std::ops::Deref for AiCommitMessageSettings {
    type Target = AiFeatureOverride;

    fn deref(&self) -> &Self::Target {
        &self.override_fields
    }
}

impl AiCommitMessageSettings {
    pub(crate) fn as_override(&self) -> &AiFeatureOverride {
        &self.override_fields
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettings {
    #[serde(default)]
    pub commit_message: AiCommitMessageSettings,
    #[serde(default)]
    pub session_title: AiFeatureOverride,
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE_NAME)
}

pub(crate) fn normalize_ai_feature_override(value: AiFeatureOverride) -> AiFeatureOverride {
    let channel_id = normalize_optional_text(value.channel_id.as_deref());
    let model = if channel_id.is_some() {
        normalize_optional_text(value.model.as_deref())
    } else {
        None
    };
    let reasoning_effort = if model.is_some() {
        normalize_optional_text(value.reasoning_effort.as_deref())
    } else {
        None
    };
    AiFeatureOverride {
        enabled: value.enabled,
        channel_id,
        model,
        reasoning_effort,
    }
}

pub(crate) fn normalize_commit_message_settings(
    value: AiCommitMessageSettings,
) -> AiCommitMessageSettings {
    AiCommitMessageSettings {
        override_fields: normalize_ai_feature_override(value.override_fields),
        style: value.style,
    }
}

pub(crate) fn normalize_ai_settings(settings: AiSettings) -> AiSettings {
    AiSettings {
        commit_message: normalize_commit_message_settings(settings.commit_message),
        session_title: normalize_ai_feature_override(settings.session_title),
    }
}

pub(crate) async fn validate_ai_feature_override(
    pool: &SqlitePool,
    value: AiFeatureOverride,
) -> Result<AiFeatureOverride, String> {
    let value = normalize_ai_feature_override(value);
    let Some(channel_id) = value.channel_id.as_deref() else {
        return Ok(value);
    };
    let record = fetch_channel_record(pool, channel_id).await?;
    if record.enabled == 0 {
        return Err(format!("渠道「{}」已停用", record.name));
    }
    let channel = record_to_channel(record)?;
    let Some(model_id) = value.model.as_deref() else {
        return Ok(AiFeatureOverride {
            reasoning_effort: None,
            ..value
        });
    };
    let config = channel
        .models
        .iter()
        .find(|item| item.id == model_id)
        .ok_or_else(|| format!("渠道「{}」没有模型「{}」", channel.name, model_id))?;
    if config.thinking_enabled != Some(true) {
        return Ok(AiFeatureOverride {
            reasoning_effort: None,
            ..value
        });
    }
    if let Some(effort) = value.reasoning_effort.as_deref() {
        let allowed = selected_thinking_levels(config);
        if !allowed.iter().any(|item| item == effort) {
            return Err(format!(
                "推理强度「{effort}」不在模型「{model_id}」的允许范围内"
            ));
        }
    }
    Ok(value)
}

pub(crate) async fn validate_ai_settings(
    pool: &SqlitePool,
    settings: AiSettings,
) -> Result<AiSettings, String> {
    Ok(AiSettings {
        commit_message: AiCommitMessageSettings {
            override_fields: validate_ai_feature_override(
                pool,
                settings.commit_message.override_fields,
            )
            .await?,
            style: settings.commit_message.style,
        },
        session_title: validate_ai_feature_override(pool, settings.session_title).await?,
    })
}

pub(crate) fn load_ai_settings_from(config_dir: &Path) -> Result<AiSettings, String> {
    let path = settings_path(config_dir);
    if !path.exists() {
        return Ok(AiSettings::default());
    }
    let raw = fs::read_to_string(&path).map_err(|error| format!("读取 AI 设置失败: {error}"))?;
    let parsed: AiSettings =
        serde_json::from_str(&raw).map_err(|error| format!("解析 AI 设置失败: {error}"))?;
    Ok(normalize_ai_settings(parsed))
}

pub(crate) fn save_ai_settings_to(config_dir: &Path, settings: &AiSettings) -> Result<(), String> {
    let path = settings_path(config_dir);
    fs::create_dir_all(config_dir).map_err(|error| format!("创建 AI 设置目录失败: {error}"))?;
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("序列化 AI 设置失败: {error}"))?;
    let tmp_path = config_dir.join(format!(".{SETTINGS_FILE_NAME}.{}.tmp", std::process::id()));
    fs::write(&tmp_path, raw.as_bytes()).map_err(|error| format!("写入 AI 设置失败: {error}"))?;
    if let Err(error) = fs::rename(&tmp_path, &path) {
        let _ = fs::remove_file(&path);
        fs::rename(&tmp_path, &path).map_err(|rename_error| {
            let _ = fs::remove_file(&tmp_path);
            format!("写入 AI 设置失败: {error}; 重试: {rename_error}")
        })?;
    }
    Ok(())
}

pub(crate) fn load_ai_settings<R: Runtime>(app: &AppHandle<R>) -> Result<AiSettings, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    load_ai_settings_from(&config_dir)
}

#[tauri::command]
pub async fn get_ai_settings<R: Runtime>(app: AppHandle<R>) -> Result<AiSettings, String> {
    load_ai_settings(&app)
}

#[tauri::command]
pub async fn update_ai_settings<R: Runtime>(
    app: AppHandle<R>,
    payload: AiSettings,
) -> Result<AiSettings, String> {
    let pool = sqlite_pool(&app).await?;
    let normalized = validate_ai_settings(&pool, payload).await?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    save_ai_settings_to(&config_dir, &normalized)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("noxcode-ai-settings-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = temp_dir();
        let loaded = load_ai_settings_from(&dir).expect("load");
        assert_eq!(loaded, AiSettings::default());
        assert!(!loaded.commit_message.enabled);
        assert_eq!(loaded.commit_message.style, CommitMessageStyle::Detailed);
        assert!(!loaded.session_title.enabled);
        cleanup(&dir);
    }

    #[test]
    fn missing_or_unknown_style_defaults_to_detailed() {
        let parsed: AiSettings = serde_json::from_str(
            r#"{
              "commit_message": {
                "enabled": true,
                "channel_id": "ch-1",
                "model": "gpt-5.4",
                "reasoning_effort": "low"
              },
              "session_title": {}
            }"#,
        )
        .expect("legacy json");
        assert_eq!(parsed.commit_message.style, CommitMessageStyle::Detailed);
        assert!(parsed.commit_message.enabled);
        assert_eq!(parsed.commit_message.channel_id.as_deref(), Some("ch-1"));

        let unknown: AiSettings = serde_json::from_str(
            r#"{
              "commit_message": { "style": "verbose" },
              "session_title": {}
            }"#,
        )
        .expect("unknown style");
        assert_eq!(unknown.commit_message.style, CommitMessageStyle::Detailed);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir();
        let settings = AiSettings {
            commit_message: AiCommitMessageSettings {
                override_fields: AiFeatureOverride {
                    enabled: true,
                    channel_id: Some("ch-1".to_string()),
                    model: Some("gpt-5.4".to_string()),
                    reasoning_effort: Some("medium".to_string()),
                },
                style: CommitMessageStyle::Concise,
            },
            session_title: AiFeatureOverride {
                enabled: false,
                channel_id: None,
                model: None,
                reasoning_effort: None,
            },
        };
        save_ai_settings_to(&dir, &settings).expect("save");
        let loaded = load_ai_settings_from(&dir).expect("load");
        assert_eq!(loaded, settings);
        cleanup(&dir);
    }

    #[tokio::test]
    async fn validate_rejects_unknown_channel_and_effort() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url, models_json, enabled) VALUES ('ch-1', 'demo', 'openai', 'https://example.com', $1, 1)",
        )
        .bind(r#"[{"id":"gpt-5.4","thinking_enabled":true,"thinking_levels":["low","medium","high"]}]"#)
        .execute(&pool)
        .await
        .expect("insert channel");

        let missing = validate_ai_feature_override(
            &pool,
            AiFeatureOverride {
                enabled: true,
                channel_id: Some("missing".to_string()),
                model: None,
                reasoning_effort: None,
            },
        )
        .await
        .expect_err("missing channel");
        assert!(missing.contains("不存在"), "{missing}");

        let bad_effort = validate_ai_feature_override(
            &pool,
            AiFeatureOverride {
                enabled: true,
                channel_id: Some("ch-1".to_string()),
                model: Some("gpt-5.4".to_string()),
                reasoning_effort: Some("max".to_string()),
            },
        )
        .await
        .expect_err("bad effort");
        assert!(bad_effort.contains("允许范围"), "{bad_effort}");

        let ok = validate_ai_feature_override(
            &pool,
            AiFeatureOverride {
                enabled: true,
                channel_id: Some("ch-1".to_string()),
                model: Some("gpt-5.4".to_string()),
                reasoning_effort: Some("medium".to_string()),
            },
        )
        .await
        .expect("valid");
        assert_eq!(ok.reasoning_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn trims_empty_fields_and_drops_orphans() {
        let settings = normalize_ai_settings(AiSettings {
            commit_message: AiCommitMessageSettings {
                override_fields: AiFeatureOverride {
                    enabled: true,
                    channel_id: Some("   ".to_string()),
                    model: Some("gpt".to_string()),
                    reasoning_effort: Some("high".to_string()),
                },
                style: CommitMessageStyle::Concise,
            },
            session_title: AiFeatureOverride {
                enabled: false,
                channel_id: Some("ch-1".to_string()),
                model: Some("  ".to_string()),
                reasoning_effort: Some("low".to_string()),
            },
        });
        assert_eq!(
            settings.commit_message,
            AiCommitMessageSettings {
                override_fields: AiFeatureOverride {
                    enabled: true,
                    channel_id: None,
                    model: None,
                    reasoning_effort: None,
                },
                style: CommitMessageStyle::Concise,
            }
        );
        assert_eq!(
            settings.session_title,
            AiFeatureOverride {
                enabled: false,
                channel_id: Some("ch-1".to_string()),
                model: None,
                reasoning_effort: None,
            }
        );
    }
}
