use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

const SETTINGS_FILE_NAME: &str = "quick-prompts.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuickPrompt {
    pub id: String,
    pub label: String,
    pub prompt: String,
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE_NAME)
}

pub(crate) fn default_quick_prompts() -> Vec<QuickPrompt> {
    vec![
        QuickPrompt {
            id: "explore".to_string(),
            label: "解读代码库".to_string(),
            prompt: "分析当前工作区的架构与模块职责，输出结构化报告".to_string(),
        },
        QuickPrompt {
            id: "fix".to_string(),
            label: "修复报错".to_string(),
            prompt: "我遇到这个报错：（粘贴），定位原因并修复".to_string(),
        },
        QuickPrompt {
            id: "test".to_string(),
            label: "补测试".to_string(),
            prompt: "为（文件/函数）补充测试用例并跑通".to_string(),
        },
        QuickPrompt {
            id: "commit".to_string(),
            label: "写提交信息".to_string(),
            prompt: "看当前 git 变更，生成 Conventional Commit 信息".to_string(),
        },
    ]
}

fn normalize_quick_prompts(prompts: Vec<QuickPrompt>) -> Result<Vec<QuickPrompt>, String> {
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for prompt in prompts {
        let id = prompt.id.trim().to_string();
        let label = prompt.label.trim().to_string();
        let body = prompt.prompt.trim().to_string();
        if id.is_empty() {
            return Err("快捷提示 id 不能为空".to_string());
        }
        if label.is_empty() {
            return Err("快捷提示标题不能为空".to_string());
        }
        if body.is_empty() {
            return Err("快捷提示内容不能为空".to_string());
        }
        if !seen.insert(id.clone()) {
            return Err(format!("快捷提示 id 重复: {id}"));
        }
        normalized.push(QuickPrompt {
            id,
            label,
            prompt: body,
        });
    }
    if normalized.is_empty() {
        return Err("至少保留一条快捷提示".to_string());
    }
    Ok(normalized)
}

pub(crate) fn load_quick_prompts_from(config_dir: &Path) -> Result<Vec<QuickPrompt>, String> {
    let path = settings_path(config_dir);
    if !path.exists() {
        return Ok(default_quick_prompts());
    }
    let raw = fs::read_to_string(&path).map_err(|error| format!("读取快捷提示失败: {error}"))?;
    let parsed: Vec<QuickPrompt> =
        serde_json::from_str(&raw).map_err(|error| format!("解析快捷提示失败: {error}"))?;
    normalize_quick_prompts(parsed)
}

pub(crate) fn save_quick_prompts_to(
    config_dir: &Path,
    prompts: &[QuickPrompt],
) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|error| format!("创建快捷提示目录失败: {error}"))?;
    let raw = serde_json::to_string_pretty(prompts)
        .map_err(|error| format!("序列化快捷提示失败: {error}"))?;
    let tmp_path = config_dir.join(format!(".{SETTINGS_FILE_NAME}.{}.tmp", std::process::id()));
    fs::write(&tmp_path, raw.as_bytes()).map_err(|error| format!("写入快捷提示失败: {error}"))?;
    if let Err(error) = fs::rename(&tmp_path, settings_path(config_dir)) {
        let _ = fs::remove_file(settings_path(config_dir));
        fs::rename(&tmp_path, settings_path(config_dir)).map_err(|rename_error| {
            let _ = fs::remove_file(&tmp_path);
            format!("写入快捷提示失败: {error}; 重试: {rename_error}")
        })?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_quick_prompts<R: Runtime>(app: AppHandle<R>) -> Result<Vec<QuickPrompt>, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    load_quick_prompts_from(&config_dir)
}

#[tauri::command]
pub async fn update_quick_prompts<R: Runtime>(
    app: AppHandle<R>,
    payload: Vec<QuickPrompt>,
) -> Result<Vec<QuickPrompt>, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    let normalized = normalize_quick_prompts(payload)?;
    save_quick_prompts_to(&config_dir, &normalized)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noxcode-qp-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = temp_dir();
        let loaded = load_quick_prompts_from(&dir).expect("load");
        assert_eq!(loaded, default_quick_prompts());
        cleanup(&dir);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir();
        let prompts = vec![QuickPrompt {
            id: "custom".to_string(),
            label: "自定义".to_string(),
            prompt: "做一件事".to_string(),
        }];
        save_quick_prompts_to(&dir, &prompts).expect("save");
        let loaded = load_quick_prompts_from(&dir).expect("load");
        assert_eq!(loaded, prompts);
        cleanup(&dir);
    }

    #[test]
    fn rejects_empty_and_duplicate() {
        assert!(normalize_quick_prompts(vec![])
            .expect_err("empty")
            .contains("至少"));
        let err = normalize_quick_prompts(vec![
            QuickPrompt {
                id: "a".to_string(),
                label: "A".to_string(),
                prompt: "one".to_string(),
            },
            QuickPrompt {
                id: "a".to_string(),
                label: "B".to_string(),
                prompt: "two".to_string(),
            },
        ])
        .expect_err("dup");
        assert!(err.contains("重复"));
    }
}
