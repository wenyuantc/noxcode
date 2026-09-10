use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Runtime,
    Window,
};

const MAIN_WINDOW_LABEL: &str = "main";
const WINDOW_STATE_FILE_NAME: &str = "window-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct PersistedWindowState {
    width: u32,
    height: u32,
    #[serde(default)]
    logical: bool,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
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

fn state_from_logical_size(
    size: LogicalSize<u32>,
    position: Option<LogicalPosition<f64>>,
) -> Option<PersistedWindowState> {
    normalize_window_state(PersistedWindowState {
        width: size.width,
        height: size.height,
        logical: true,
        x: position.map(|position| position.x),
        y: position.map(|position| position.y),
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

type MonitorInfo = (PhysicalPosition<i32>, PhysicalSize<u32>, f64);

fn window_intersects_monitor(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    monitors: &[MonitorInfo],
) -> bool {
    let (left, top) = (i64::from(position.x), i64::from(position.y));
    let (right, bottom) = (left + i64::from(size.width), top + i64::from(size.height));
    monitors.iter().any(|(monitor_position, monitor_size, _)| {
        let (m_left, m_top) = (i64::from(monitor_position.x), i64::from(monitor_position.y));
        let (m_right, m_bottom) = (
            m_left + i64::from(monitor_size.width),
            m_top + i64::from(monitor_size.height),
        );
        left < m_right && right > m_left && top < m_bottom && bottom > m_top
    })
}

fn restore_position_decision(
    state: PersistedWindowState,
    monitors: &[MonitorInfo],
) -> Option<PhysicalPosition<i32>> {
    let (x, y) = (state.x?, state.y?);
    // 逻辑尺寸与缩放无关，scale 1.0 转换后保持不变。
    let size = logical_size_to_restore(state, 1.0)?;
    monitors
        .iter()
        .find_map(|(monitor_position, monitor_size, scale)| {
            let scale = sanitize_scale(*scale);
            let position: PhysicalPosition<f64> = LogicalPosition::new(x, y).to_physical(scale);
            let position =
                PhysicalPosition::new(position.x.round() as i32, position.y.round() as i32);
            let size = size.to_physical(scale);
            window_intersects_monitor(position, size, &[(*monitor_position, *monitor_size, scale)])
                .then_some(position)
        })
}

fn persist_logical_size<R: Runtime>(
    app: &AppHandle<R>,
    size: LogicalSize<u32>,
    position: Option<LogicalPosition<f64>>,
) -> Result<(), String> {
    let Some(state) = state_from_logical_size(size, position) else {
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
    position: PhysicalPosition<i32>,
) -> Result<(), String> {
    persist_logical_size(
        app,
        size.to_logical(sanitize_scale(scale_factor)),
        Some(position.to_logical(sanitize_scale(scale_factor))),
    )
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

    let monitors = window
        .available_monitors()
        .map_err(|error| format!("读取显示器信息失败: {error}"))?;
    let monitor_infos: Vec<MonitorInfo> = monitors
        .iter()
        .map(|monitor| (*monitor.position(), *monitor.size(), monitor.scale_factor()))
        .collect();

    if let Some(position) = restore_position_decision(state, &monitor_infos) {
        window
            .set_position(position)
            .map_err(|error| format!("恢复窗口位置失败: {error}"))?;
        return Ok(());
    }

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
    let position = window
        .outer_position()
        .map_err(|error| format!("读取窗口位置失败: {error}"))?;
    persist_physical_size(window.app_handle(), size, scale, position)
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
            let position = window.outer_position().map_err(|error| error.to_string())?;
            Ok::<_, String>(Some((
                size.to_logical(sanitize_scale(scale)),
                position.to_logical(sanitize_scale(scale)),
            )))
        })();
        let _ = tx.send(snapshot);
    })
    .map_err(|error| error.to_string())?;
    if let Some((size, position)) = rx.await.map_err(|error| error.to_string())?? {
        let app = app.clone();
        tokio::task::spawn_blocking(move || persist_logical_size(&app, size, Some(position)))
            .await
            .map_err(|error| error.to_string())??;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        logical_size_to_restore, normalize_window_state, parse_window_state,
        restore_position_decision, state_from_logical_size, window_intersects_monitor,
        PersistedWindowState,
    };
    use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};

    #[test]
    fn normalize_window_state_rejects_zero_dimensions() {
        assert_eq!(
            normalize_window_state(PersistedWindowState {
                width: 0,
                height: 800,
                logical: true,
                x: None,
                y: None,
            }),
            None
        );
        assert_eq!(
            normalize_window_state(PersistedWindowState {
                width: 1280,
                height: 0,
                logical: true,
                x: None,
                y: None,
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
                x: None,
                y: None,
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
                x: None,
                y: None,
            })
        );
    }

    #[test]
    fn parse_window_state_accepts_position_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":1440,"height":900,"logical":true,"x":100.0,"y":200.0}"#)
                .unwrap(),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
                logical: true,
                x: Some(100.0),
                y: Some(200.0),
            })
        );
    }

    #[test]
    fn parse_window_state_legacy_payload_defaults_position_to_none() {
        assert_eq!(
            parse_window_state(r#"{"width":1440,"height":900,"logical":true}"#).unwrap(),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
                logical: true,
                x: None,
                y: None,
            })
        );
    }

    #[test]
    fn parse_window_state_rejects_invalid_payload() {
        assert!(parse_window_state("not-json").is_err());
    }

    #[test]
    fn state_from_logical_size_skips_invalid_dimensions() {
        assert_eq!(
            state_from_logical_size(LogicalSize::new(0, 800), None),
            None
        );
        assert_eq!(
            state_from_logical_size(LogicalSize::new(1280, 0), None),
            None
        );
    }

    #[test]
    fn state_from_logical_size_keeps_valid_dimensions() {
        assert_eq!(
            state_from_logical_size(LogicalSize::new(1440, 900), None),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
                logical: true,
                x: None,
                y: None,
            })
        );
    }

    #[test]
    fn state_from_logical_size_keeps_position() {
        assert_eq!(
            state_from_logical_size(
                LogicalSize::new(1440, 900),
                Some(LogicalPosition::new(100.0, 200.0)),
            ),
            Some(PersistedWindowState {
                width: 1440,
                height: 900,
                logical: true,
                x: Some(100.0),
                y: Some(200.0),
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
                    x: None,
                    y: None,
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
                    x: None,
                    y: None,
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
                    x: None,
                    y: None,
                },
                2.0,
            ),
            None
        );
    }

    #[test]
    fn window_intersects_monitor_detects_overlap() {
        let monitors = [(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            1.0,
        )];
        assert!(window_intersects_monitor(
            PhysicalPosition::new(100, 100),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
        // 部分在屏幕外仍算相交
        assert!(window_intersects_monitor(
            PhysicalPosition::new(-100, 0),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
        // 完全在屏幕外
        assert!(!window_intersects_monitor(
            PhysicalPosition::new(2000, 0),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
        // 副屏已拔除：位置落在旧副屏区域
        assert!(!window_intersects_monitor(
            PhysicalPosition::new(1920, 0),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
    }

    #[test]
    fn restore_position_decision_keeps_valid_position() {
        let state = PersistedWindowState {
            width: 1440,
            height: 900,
            logical: true,
            x: Some(100.0),
            y: Some(200.0),
        };
        let monitors = [(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            2.0,
        )];
        assert_eq!(
            restore_position_decision(state, &monitors),
            Some(PhysicalPosition::new(200, 400))
        );
    }

    #[test]
    fn restore_position_decision_uses_each_monitor_scale() {
        // 保存时窗口位于 scale 1.0 的副屏（逻辑 x=1920），
        // 启动时窗口创建在 scale 2.0 的主屏上，也必须按副屏自身缩放恢复。
        let state = PersistedWindowState {
            width: 1800,
            height: 1125,
            logical: true,
            x: Some(1920.0),
            y: Some(-7.0),
        };
        let monitors = [
            (
                PhysicalPosition::new(0, 0),
                PhysicalSize::new(3024, 1964),
                2.0,
            ),
            (
                PhysicalPosition::new(1920, 0),
                PhysicalSize::new(1920, 1080),
                1.0,
            ),
        ];
        assert_eq!(
            restore_position_decision(state, &monitors),
            Some(PhysicalPosition::new(1920, -7))
        );
    }

    #[test]
    fn restore_position_decision_falls_back_when_monitor_missing() {
        let state = PersistedWindowState {
            width: 1440,
            height: 900,
            logical: true,
            x: Some(3000.0),
            y: Some(200.0),
        };
        let monitors = [(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            1.0,
        )];
        assert_eq!(restore_position_decision(state, &monitors), None);
    }

    #[test]
    fn restore_position_decision_falls_back_without_position() {
        let state = PersistedWindowState {
            width: 1440,
            height: 900,
            logical: true,
            x: None,
            y: None,
        };
        let monitors = [(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            1.0,
        )];
        assert_eq!(restore_position_decision(state, &monitors), None);
    }
}
