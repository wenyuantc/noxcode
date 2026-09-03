use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalSize, Runtime, Window};

const MAIN_WINDOW_LABEL: &str = "main";
const WINDOW_STATE_FILE_NAME: &str = "window-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedWindowState {
    width: u32,
    height: u32,
}

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

fn window_state_file_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app_config_dir(app)?.join(WINDOW_STATE_FILE_NAME))
}

fn normalize_window_state(state: PersistedWindowState) -> Option<PersistedWindowState> {
    (state.width > 0 && state.height > 0).then_some(state)
}

fn parse_window_state(raw: &str) -> Result<Option<PersistedWindowState>, String> {
    let state = serde_json::from_str::<PersistedWindowState>(raw)
        .map_err(|error| format!("解析窗口状态失败: {error}"))?;

    Ok(normalize_window_state(state))
}

fn load_window_state<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Option<PersistedWindowState>, String> {
    let path = window_state_file_path(app)?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|error| format!("读取窗口状态失败: {error}"))?;
    parse_window_state(&raw)
}

pub fn restore_main_window_size<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(());
    };

    let Some(state) = load_window_state(app)? else {
        return Ok(());
    };

    window
        .set_size(PhysicalSize::new(state.width, state.height))
        .map_err(|error| format!("恢复窗口尺寸失败: {error}"))?;

    let _ = window.center();

    Ok(())
}

pub fn save_window_size<R: Runtime>(window: &Window<R>) -> Result<(), String> {
    let size = window
        .inner_size()
        .map_err(|error| format!("读取窗口尺寸失败: {error}"))?;
    let Some(state) = normalize_window_state(PersistedWindowState {
        width: size.width,
        height: size.height,
    }) else {
        return Ok(());
    };

    let app = window.app_handle();
    let config_dir = app_config_dir(app)?;
    fs::create_dir_all(&config_dir).map_err(|error| format!("创建应用配置目录失败: {error}"))?;

    let raw = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("序列化窗口状态失败: {error}"))?;
    fs::write(window_state_file_path(app)?, raw)
        .map_err(|error| format!("写入窗口状态失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{normalize_window_state, parse_window_state, PersistedWindowState};

    #[test]
    fn normalize_window_state_rejects_zero_dimensions() {
        assert_eq!(
            normalize_window_state(PersistedWindowState {
                width: 0,
                height: 800,
            }),
            None
        );
        assert_eq!(
            normalize_window_state(PersistedWindowState {
                width: 1280,
                height: 0,
            }),
            None
        );
    }

    #[test]
    fn parse_window_state_accepts_valid_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":1440,"height":900}"#).unwrap(),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
            })
        );
    }

    #[test]
    fn parse_window_state_rejects_invalid_payload() {
        assert!(parse_window_state("not-json").is_err());
    }
}
