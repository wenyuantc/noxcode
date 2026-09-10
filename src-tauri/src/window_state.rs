use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, Window};

const MAIN_WINDOW_LABEL: &str = "main";
const WINDOW_STATE_FILE_NAME: &str = "window-state.json";

#[derive(Debug, Default)]
pub struct WindowStateCache {
    last: Mutex<Option<PhysicalSnapshot>>,
    restoring: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhysicalSnapshot {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct PersistedWindowState {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
struct RawWindowState {
    width: u32,
    height: u32,
    #[serde(default)]
    logical: bool,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LoadedWindowState {
    width: u32,
    height: u32,
    logical: bool,
    x: Option<f64>,
    y: Option<f64>,
}

type MonitorRect = (PhysicalPosition<i32>, PhysicalSize<u32>);

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

fn f64_to_i32(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }
    let rounded = value.round();
    if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return None;
    }
    Some(rounded as i32)
}

fn normalize_window_state(state: LoadedWindowState) -> Option<LoadedWindowState> {
    (state.width > 0 && state.height > 0).then_some(state)
}

fn parse_window_state(raw: &str) -> Result<Option<LoadedWindowState>, String> {
    let raw = serde_json::from_str::<RawWindowState>(raw)
        .map_err(|error| format!("解析窗口状态失败: {error}"))?;
    Ok(normalize_window_state(LoadedWindowState {
        width: raw.width,
        height: raw.height,
        logical: raw.logical,
        x: raw.x,
        y: raw.y,
    }))
}

fn load_window_state<R: Runtime>(app: &AppHandle<R>) -> Result<Option<LoadedWindowState>, String> {
    let path = window_state_file_path(app)?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|error| format!("读取窗口状态失败: {error}"))?;
    parse_window_state(&raw)
}

fn snapshot_from_physical(
    size: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
) -> Option<PhysicalSnapshot> {
    (size.width > 0 && size.height > 0).then_some(PhysicalSnapshot {
        width: size.width,
        height: size.height,
        x: position.x,
        y: position.y,
    })
}

fn physical_size_to_restore(
    state: LoadedWindowState,
    scale_factor: f64,
) -> Option<PhysicalSize<u32>> {
    let state = normalize_window_state(state)?;
    if !state.logical {
        return Some(PhysicalSize::new(state.width, state.height));
    }

    let scale = sanitize_scale(scale_factor);
    let width = (f64::from(state.width) * scale).round();
    let height = (f64::from(state.height) * scale).round();
    if width < 1.0 || height < 1.0 || width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return None;
    }
    Some(PhysicalSize::new(width as u32, height as u32))
}

fn window_intersects_monitor(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    monitors: &[MonitorRect],
) -> bool {
    let (left, top) = (i64::from(position.x), i64::from(position.y));
    let (right, bottom) = (left + i64::from(size.width), top + i64::from(size.height));
    monitors.iter().any(|(monitor_position, monitor_size)| {
        let (m_left, m_top) = (i64::from(monitor_position.x), i64::from(monitor_position.y));
        let (m_right, m_bottom) = (
            m_left + i64::from(monitor_size.width),
            m_top + i64::from(monitor_size.height),
        );
        left < m_right && right > m_left && top < m_bottom && bottom > m_top
    })
}

fn restore_position_decision(
    x: Option<f64>,
    y: Option<f64>,
    size: PhysicalSize<u32>,
    monitors: &[MonitorRect],
) -> Option<PhysicalPosition<i32>> {
    let position = PhysicalPosition::new(f64_to_i32(x?)?, f64_to_i32(y?)?);
    window_intersects_monitor(position, size, monitors).then_some(position)
}

fn persist_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: PhysicalSnapshot,
) -> Result<(), String> {
    let config_dir = app_config_dir(app)?;
    fs::create_dir_all(&config_dir).map_err(|error| format!("创建应用配置目录失败: {error}"))?;

    let raw = serde_json::to_string_pretty(&PersistedWindowState {
        width: snapshot.width,
        height: snapshot.height,
        x: snapshot.x,
        y: snapshot.y,
    })
    .map_err(|error| format!("序列化窗口状态失败: {error}"))?;
    fs::write(window_state_file_path(app)?, raw)
        .map_err(|error| format!("写入窗口状态失败: {error}"))
}

fn cache_of<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::State<'_, WindowStateCache>> {
    app.try_state::<WindowStateCache>()
}

fn is_restoring<R: Runtime>(app: &AppHandle<R>) -> bool {
    cache_of(app).is_some_and(|cache| cache.restoring.load(Ordering::SeqCst))
}

fn cached_snapshot<R: Runtime>(app: &AppHandle<R>) -> Option<PhysicalSnapshot> {
    cache_of(app)?.last.lock().ok().and_then(|guard| *guard)
}

fn store_snapshot<R: Runtime>(app: &AppHandle<R>, snapshot: PhysicalSnapshot) {
    if let Some(cache) = cache_of(app) {
        if let Ok(mut last) = cache.last.lock() {
            *last = Some(snapshot);
        }
    }
}

fn snapshot_from_live(
    size: Option<PhysicalSize<u32>>,
    position: Option<PhysicalPosition<i32>>,
) -> Option<PhysicalSnapshot> {
    snapshot_from_physical(size?, position?)
}

fn should_ignore_live_event<R: Runtime>(window: &Window<R>) -> bool {
    is_restoring(window.app_handle()) || window.is_minimized().unwrap_or(false)
}

fn snapshot_for_persist<R: Runtime>(app: &AppHandle<R>) -> Option<PhysicalSnapshot> {
    if let Some(snapshot) = cached_snapshot(app) {
        return Some(snapshot);
    }
    let window = app.get_webview_window(MAIN_WINDOW_LABEL)?;
    if !window.is_visible().unwrap_or(false) {
        return None;
    }
    snapshot_from_live(window.inner_size().ok(), window.outer_position().ok())
}

fn collect_monitor_rects<R: Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Result<Vec<MonitorRect>, String> {
    let monitors = window
        .available_monitors()
        .map_err(|error| format!("读取显示器信息失败: {error}"))?;
    Ok(monitors
        .iter()
        .map(|monitor| (*monitor.position(), *monitor.size()))
        .collect())
}

fn apply_restored_state<R: Runtime>(
    window: &tauri::WebviewWindow<R>,
    size: PhysicalSize<u32>,
    position: Option<PhysicalPosition<i32>>,
) -> Result<PhysicalSnapshot, String> {
    if let Some(position) = position {
        window
            .set_position(position)
            .map_err(|error| format!("恢复窗口位置失败: {error}"))?;
    }

    window
        .set_size(size)
        .map_err(|error| format!("恢复窗口尺寸失败: {error}"))?;

    if position.is_none() {
        let _ = window.center();
    }

    if let Some(snapshot) =
        snapshot_from_live(window.inner_size().ok(), window.outer_position().ok())
    {
        return Ok(snapshot);
    }

    Ok(PhysicalSnapshot {
        width: size.width,
        height: size.height,
        x: position.map(|position| position.x).unwrap_or(0),
        y: position.map(|position| position.y).unwrap_or(0),
    })
}

pub fn begin_restore<R: Runtime>(app: &AppHandle<R>) {
    if let Some(cache) = cache_of(app) {
        cache.restoring.store(true, Ordering::SeqCst);
    }
}

pub fn end_restore<R: Runtime>(app: &AppHandle<R>) {
    if let Some(cache) = cache_of(app) {
        cache.restoring.store(false, Ordering::SeqCst);
    }
}

pub fn remember_moved<R: Runtime>(window: &Window<R>, position: PhysicalPosition<i32>) {
    if should_ignore_live_event(window) {
        return;
    }
    if let Some(mut snapshot) = cached_snapshot(window.app_handle())
        .or_else(|| snapshot_from_live(window.inner_size().ok(), window.outer_position().ok()))
    {
        snapshot.x = position.x;
        snapshot.y = position.y;
        store_snapshot(window.app_handle(), snapshot);
    }
}

pub fn remember_resized<R: Runtime>(window: &Window<R>, size: PhysicalSize<u32>) {
    if should_ignore_live_event(window) || size.width == 0 || size.height == 0 {
        return;
    }
    if let Some(mut snapshot) = cached_snapshot(window.app_handle())
        .or_else(|| snapshot_from_live(window.inner_size().ok(), window.outer_position().ok()))
    {
        snapshot.width = size.width;
        snapshot.height = size.height;
        store_snapshot(window.app_handle(), snapshot);
    }
}

pub fn remember_window<R: Runtime>(window: &Window<R>) {
    if should_ignore_live_event(window) || !window.is_visible().unwrap_or(false) {
        return;
    }
    if let Some(snapshot) =
        snapshot_from_live(window.inner_size().ok(), window.outer_position().ok())
    {
        store_snapshot(window.app_handle(), snapshot);
    }
}

pub fn restore_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(());
    };

    let Some(state) = load_window_state(app)? else {
        return Ok(());
    };

    let scale = sanitize_scale(window.scale_factor().unwrap_or(1.0));
    let Some(size) = physical_size_to_restore(state, scale) else {
        return Ok(());
    };

    let monitors = collect_monitor_rects(&window)?;
    let position = restore_position_decision(state.x, state.y, size, &monitors);
    let snapshot = apply_restored_state(&window, size, position)?;
    store_snapshot(app, snapshot);
    Ok(())
}

pub fn persist_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(snapshot) = snapshot_for_persist(app) else {
        return Ok(());
    };
    persist_snapshot(app, snapshot)
}

pub fn persist_window<R: Runtime>(window: &Window<R>) -> Result<(), String> {
    remember_window(window);
    persist_main_window(window.app_handle())
}

pub async fn persist_main_window_async<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let snapshot_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(snapshot_for_persist(&snapshot_app));
    })
    .map_err(|error| error.to_string())?;
    if let Some(snapshot) = rx.await.map_err(|error| error.to_string())? {
        let app = app.clone();
        tokio::task::spawn_blocking(move || persist_snapshot(&app, snapshot))
            .await
            .map_err(|error| error.to_string())??;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_window_state, parse_window_state, physical_size_to_restore,
        restore_position_decision, snapshot_from_physical, window_intersects_monitor,
        LoadedWindowState, PersistedWindowState,
    };
    use tauri::{PhysicalPosition, PhysicalSize};

    fn loaded(
        width: u32,
        height: u32,
        logical: bool,
        x: Option<f64>,
        y: Option<f64>,
    ) -> LoadedWindowState {
        LoadedWindowState {
            width,
            height,
            logical,
            x,
            y,
        }
    }

    #[test]
    fn normalize_window_state_rejects_zero_dimensions() {
        assert_eq!(
            normalize_window_state(loaded(0, 800, false, None, None)),
            None
        );
        assert_eq!(
            normalize_window_state(loaded(1280, 0, false, None, None)),
            None
        );
    }

    #[test]
    fn parse_window_state_accepts_physical_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":3600,"height":2110,"x":2028,"y":24}"#).unwrap(),
            Some(loaded(3600, 2110, false, Some(2028.0), Some(24.0)))
        );
    }

    #[test]
    fn parse_window_state_accepts_legacy_physical_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":3600,"height":2250}"#).unwrap(),
            Some(loaded(3600, 2250, false, None, None))
        );
    }

    #[test]
    fn parse_window_state_accepts_legacy_logical_payload() {
        assert_eq!(
            parse_window_state(r#"{"width":1440,"height":900,"logical":true}"#).unwrap(),
            Some(loaded(1440, 900, true, None, None))
        );
    }

    #[test]
    fn parse_window_state_accepts_legacy_logical_position() {
        assert_eq!(
            parse_window_state(
                r#"{"width":1800,"height":1055,"logical":true,"x":2028.0,"y":24.0}"#
            )
            .unwrap(),
            Some(loaded(1800, 1055, true, Some(2028.0), Some(24.0)))
        );
    }

    #[test]
    fn parse_window_state_rejects_invalid_payload() {
        assert!(parse_window_state("not-json").is_err());
    }

    #[test]
    fn persisted_payload_omits_logical_flag() {
        let raw = serde_json::to_string(&PersistedWindowState {
            width: 3600,
            height: 2110,
            x: 2028,
            y: 24,
        })
        .unwrap();
        assert!(!raw.contains("logical"));
        assert!(raw.contains("\"x\":2028"));
    }

    #[test]
    fn snapshot_from_physical_skips_invalid_dimensions() {
        assert_eq!(
            snapshot_from_physical(PhysicalSize::new(0, 800), PhysicalPosition::new(10, 20)),
            None
        );
        assert_eq!(
            snapshot_from_physical(PhysicalSize::new(1280, 0), PhysicalPosition::new(10, 20)),
            None
        );
    }

    #[test]
    fn restore_keeps_physical_size() {
        assert_eq!(
            physical_size_to_restore(loaded(3600, 2250, false, None, None), 2.0),
            Some(PhysicalSize::new(3600, 2250))
        );
    }

    #[test]
    fn restore_converts_legacy_logical_size() {
        assert_eq!(
            physical_size_to_restore(loaded(1800, 1055, true, Some(2028.0), Some(24.0)), 2.0),
            Some(PhysicalSize::new(3600, 2110))
        );
    }

    #[test]
    fn restore_rejects_zero_size() {
        assert_eq!(
            physical_size_to_restore(loaded(0, 800, false, None, None), 2.0),
            None
        );
    }

    #[test]
    fn window_intersects_monitor_detects_overlap() {
        let monitors = [(PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080))];
        assert!(window_intersects_monitor(
            PhysicalPosition::new(100, 100),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
        assert!(window_intersects_monitor(
            PhysicalPosition::new(-100, 0),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
        assert!(!window_intersects_monitor(
            PhysicalPosition::new(2000, 0),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
        assert!(!window_intersects_monitor(
            PhysicalPosition::new(1920, 0),
            PhysicalSize::new(800, 600),
            &monitors,
        ));
    }

    #[test]
    fn restore_position_decision_keeps_physical_position() {
        let monitors = [(
            PhysicalPosition::new(1920, 0),
            PhysicalSize::new(1920, 1080),
        )];
        assert_eq!(
            restore_position_decision(
                Some(2028.0),
                Some(24.0),
                PhysicalSize::new(1800, 1055),
                &monitors,
            ),
            Some(PhysicalPosition::new(2028, 24))
        );
    }

    #[test]
    fn restore_position_decision_treats_legacy_logical_as_physical() {
        let monitors = [
            (PhysicalPosition::new(0, 0), PhysicalSize::new(3024, 1964)),
            (
                PhysicalPosition::new(1920, 0),
                PhysicalSize::new(1920, 1080),
            ),
        ];
        assert_eq!(
            restore_position_decision(
                Some(2028.0),
                Some(24.0),
                PhysicalSize::new(3600, 2110),
                &monitors,
            ),
            Some(PhysicalPosition::new(2028, 24))
        );
    }

    #[test]
    fn restore_position_decision_falls_back_when_monitor_missing() {
        let monitors = [(PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080))];
        assert_eq!(
            restore_position_decision(
                Some(3000.0),
                Some(200.0),
                PhysicalSize::new(1440, 900),
                &monitors,
            ),
            None
        );
    }

    #[test]
    fn restore_position_decision_falls_back_without_position() {
        let monitors = [(PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080))];
        assert_eq!(
            restore_position_decision(None, None, PhysicalSize::new(1440, 900), &monitors),
            None
        );
    }
}
