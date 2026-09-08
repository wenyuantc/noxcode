use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalSize, Manager, PhysicalSize, Runtime, Window};

const MAIN_WINDOW_LABEL: &str = "main";
const WINDOW_STATE_FILE_NAME: &str = "window-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedWindowState {
    width: u32,
    height: u32,
    #[serde(default)]
    logical: bool,
}

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

fn window_state_file_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app_config_dir(app)?.join(WINDOW_STATE_FILE_NAME))
}

fn sanitize_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
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

fn state_from_logical_size(size: LogicalSize<u32>) -> Option<PersistedWindowState> {
    normalize_window_state(PersistedWindowState {
        width: size.width,
        height: size.height,
        logical: true,
    })
}

fn logical_size_to_restore(
    state: PersistedWindowState,
    scale_factor: f64,
) -> Option<LogicalSize<u32>> {
    let state = normalize_window_state(state)?;
    if state.logical {
        return Some(LogicalSize::new(state.width, state.height));
    }

    Some(PhysicalSize::new(state.width, state.height).to_logical(sanitize_scale(scale_factor)))
}

fn persist_logical_size<R: Runtime>(
    app: &AppHandle<R>,
    size: LogicalSize<u32>,
) -> Result<(), String> {
    let Some(state) = state_from_logical_size(size) else {
        return Ok(());
    };

    let config_dir = app_config_dir(app)?;
    fs::create_dir_all(&config_dir).map_err(|error| format!("创建应用配置目录失败: {error}"))?;

    let raw = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("序列化窗口状态失败: {error}"))?;
    fs::write(window_state_file_path(app)?, raw)
        .map_err(|error| format!("写入窗口状态失败: {error}"))
}

fn persist_physical_size<R: Runtime>(
    app: &AppHandle<R>,
    size: PhysicalSize<u32>,
    scale_factor: f64,
) -> Result<(), String> {
    persist_logical_size(app, size.to_logical(sanitize_scale(scale_factor)))
}

pub fn restore_main_window_size<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(());
    };

    let Some(state) = load_window_state(app)? else {
        return Ok(());
    };

    let scale = sanitize_scale(window.scale_factor().unwrap_or(1.0));
    let Some(size) = logical_size_to_restore(state, scale) else {
        return Ok(());
    };

    window
        .set_size(size)
        .map_err(|error| format!("恢复窗口尺寸失败: {error}"))?;

    let _ = window.center();

    Ok(())
}

pub fn save_window_size<R: Runtime>(window: &Window<R>) -> Result<(), String> {
    let size = window
        .inner_size()
        .map_err(|error| format!("读取窗口尺寸失败: {error}"))?;
    let scale = window
        .scale_factor()
        .map_err(|error| format!("读取窗口缩放失败: {error}"))?;
    persist_physical_size(window.app_handle(), size, scale)
}

pub async fn save_main_window_size_async<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let snapshot_app = app.clone();
    app.run_on_main_thread(move || {
        let snapshot = (|| {
            let Some(window) = snapshot_app.get_webview_window(MAIN_WINDOW_LABEL) else {
                return Ok(None);
            };
            let size = window.inner_size().map_err(|error| error.to_string())?;
            let scale = window.scale_factor().map_err(|error| error.to_string())?;
            Ok::<_, String>(Some(size.to_logical(sanitize_scale(scale))))
        })();
        let _ = tx.send(snapshot);
    })
    .map_err(|error| error.to_string())?;
    if let Some(size) = rx.await.map_err(|error| error.to_string())?? {
        let app = app.clone();
        tokio::task::spawn_blocking(move || persist_logical_size(&app, size))
            .await
            .map_err(|error| error.to_string())??;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        logical_size_to_restore, normalize_window_state, parse_window_state,
        state_from_logical_size, PersistedWindowState,
    };
    use tauri::LogicalSize;

    #[test]
    fn normalize_window_state_rejects_zero_dimensions() {
        assert_eq!(
            normalize_window_state(PersistedWindowState {
                width: 0,
                height: 800,
                logical: true,
            }),
            None
        );
        assert_eq!(
            normalize_window_state(PersistedWindowState {
                width: 1280,
                height: 0,
                logical: true,
            }),
            None
        );
    }

    #[test]
    fn parse_window_state_accepts_legacy_physical_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":3600,"height":2250}"#).unwrap(),
            Some(PersistedWindowState {
                width: 3600,
                height: 2250,
                logical: false,
            })
        );
    }

    #[test]
    fn parse_window_state_accepts_logical_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":1440,"height":900,"logical":true}"#).unwrap(),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
                logical: true,
            })
        );
    }

    #[test]
    fn parse_window_state_rejects_invalid_payload() {
        assert!(parse_window_state("not-json").is_err());
    }

    #[test]
    fn state_from_logical_size_skips_invalid_dimensions() {
        assert_eq!(state_from_logical_size(LogicalSize::new(0, 800)), None);
        assert_eq!(state_from_logical_size(LogicalSize::new(1280, 0)), None);
    }

    #[test]
    fn state_from_logical_size_keeps_valid_dimensions() {
        assert_eq!(
            state_from_logical_size(LogicalSize::new(1440, 900)),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
                logical: true,
            })
        );
    }

    #[test]
    fn restore_converts_legacy_physical_size() {
        assert_eq!(
            logical_size_to_restore(
                PersistedWindowState {
                    width: 3600,
                    height: 2250,
                    logical: false,
                },
                2.0,
            ),
            Some(LogicalSize::new(1800, 1125))
        );
    }

    #[test]
    fn restore_keeps_logical_size() {
        assert_eq!(
            logical_size_to_restore(
                PersistedWindowState {
                    width: 1440,
                    height: 900,
                    logical: true,
                },
                2.0,
            ),
            Some(LogicalSize::new(1440, 900))
        );
    }

    #[test]
    fn restore_rejects_zero_legacy_physical_size() {
        assert_eq!(
            logical_size_to_restore(
                PersistedWindowState {
                    width: 0,
                    height: 800,
                    logical: false,
                },
                2.0,
            ),
            None
        );
    }
}
