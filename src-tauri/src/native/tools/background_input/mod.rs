//! 按应用投递电脑控制动作：默认后台，禁止悄悄回退到全局 enigo。

use serde::{Deserialize, Serialize};

use super::app_target::{scale_model_point, AppElement, AppTarget, ComputerAppState, WindowBounds};
use super::desktop::{ComputerAction, ComputerArgs, ComputerButton, ComputerDispatch};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundError {
    Unavailable(String),
    Failed(String),
}

impl BackgroundError {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable(reason.into())
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed(reason.into())
    }

    pub fn into_message(self) -> String {
        match self {
            Self::Unavailable(reason) => format_background_unavailable(&reason),
            Self::Failed(reason) => reason,
        }
    }
}

pub fn format_background_unavailable(reason: &str) -> String {
    format!("background_unavailable: {reason}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputDispatch {
    Background,
    Foreground,
}

impl From<ComputerDispatch> for InputDispatch {
    fn from(value: ComputerDispatch) -> Self {
        match value {
            ComputerDispatch::Background => Self::Background,
            ComputerDispatch::Foreground => Self::Foreground,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedAction {
    pub action: ComputerAction,
    pub element: Option<AppElement>,
    pub point: Option<(i32, i32)>,
    pub path: Vec<(i32, i32)>,
    pub button: ComputerButton,
    pub scroll_x: i32,
    pub scroll_y: i32,
    pub text: String,
    pub keys: Vec<String>,
}

pub fn collect_tree(target: &AppTarget) -> (Vec<AppElement>, Vec<String>) {
    #[cfg(target_os = "macos")]
    {
        macos::collect_tree(target)
    }
    #[cfg(target_os = "windows")]
    {
        windows::collect_tree(target)
    }
    #[cfg(target_os = "linux")]
    {
        linux::collect_tree(target)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        (
            vec![window_root_element(target)],
            vec!["当前系统未实现无障碍树，仅返回窗口节点".to_string()],
        )
    }
}

pub fn capture_app_window(target: &AppTarget) -> Result<super::app_target::WindowImage, String> {
    #[cfg(target_os = "macos")]
    {
        match macos::capture_window_image(target) {
            Ok(image) => return Ok(image),
            Err(error) => {
                if let Ok(fallback) = super::app_target::capture_window(target) {
                    return Ok(fallback);
                }
                return Err(error);
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        match windows::capture_window_image(target) {
            Ok(image) => return Ok(image),
            Err(error) => {
                if let Ok(fallback) = super::app_target::capture_window(target) {
                    return Ok(fallback);
                }
                return Err(error);
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        super::app_target::capture_window(target)
    }
}

pub fn apply_action(
    dispatch: ComputerDispatch,
    args: &ComputerArgs,
    state: &ComputerAppState,
    resolved: &ResolvedAction,
) -> Result<(), String> {
    match dispatch {
        ComputerDispatch::Background => {
            apply_background(args, state, resolved).map_err(BackgroundError::into_message)
        }
        ComputerDispatch::Foreground => apply_foreground(state, resolved),
    }
}

pub fn apply_background(
    _args: &ComputerArgs,
    state: &ComputerAppState,
    resolved: &ResolvedAction,
) -> Result<(), BackgroundError> {
    if let Some(reason) = background_drop_reason(&state.target, resolved.action, current_platform())
    {
        return Err(BackgroundError::unavailable(reason));
    }
    #[cfg(target_os = "macos")]
    {
        macos::apply_background(&state.target, resolved)
    }
    #[cfg(target_os = "windows")]
    {
        windows::apply_background(&state.target, resolved)
    }
    #[cfg(target_os = "linux")]
    {
        linux::apply_background(&state.target, resolved)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(BackgroundError::unavailable(
            "当前系统未实现后台投递，不会改用全局键鼠".to_string(),
        ))
    }
}

pub fn apply_foreground(state: &ComputerAppState, resolved: &ResolvedAction) -> Result<(), String> {
    use enigo::{Axis, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};

    let mut enigo = Enigo::new(&Settings::default()).map_err(map_foreground_error)?;
    match resolved.action {
        ComputerAction::Click => {
            let (x, y) = require_abs_point(resolved)?;
            enigo
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(map_foreground_error)?;
            enigo
                .button(enigo_button(resolved.button), Direction::Click)
                .map_err(map_foreground_error)?;
        }
        ComputerAction::SetValue | ComputerAction::TypeText => {
            if let Some((x, y)) = resolved.point {
                let (abs_x, abs_y) = window_to_screen(state, x, y);
                enigo
                    .move_mouse(abs_x, abs_y, Coordinate::Abs)
                    .map_err(map_foreground_error)?;
                enigo
                    .button(enigo::Button::Left, Direction::Click)
                    .map_err(map_foreground_error)?;
            }
            enigo.text(&resolved.text).map_err(map_foreground_error)?;
        }
        ComputerAction::PressKey => press_keys(&mut enigo, &resolved.keys)?,
        ComputerAction::Scroll => {
            if let Some((x, y)) = resolved.point {
                let (abs_x, abs_y) = window_to_screen(state, x, y);
                enigo
                    .move_mouse(abs_x, abs_y, Coordinate::Abs)
                    .map_err(map_foreground_error)?;
            }
            if resolved.scroll_x != 0 {
                enigo
                    .scroll(resolved.scroll_x, Axis::Horizontal)
                    .map_err(map_foreground_error)?;
            }
            if resolved.scroll_y != 0 {
                enigo
                    .scroll(resolved.scroll_y, Axis::Vertical)
                    .map_err(map_foreground_error)?;
            }
        }
        ComputerAction::Drag => {
            if resolved.path.len() < 2 {
                return Err("拖拽需要至少两个坐标点".to_string());
            }
            let (start_x, start_y) = resolved.path[0];
            enigo
                .move_mouse(start_x, start_y, Coordinate::Abs)
                .map_err(map_foreground_error)?;
            let button = enigo_button(resolved.button);
            enigo
                .button(button, Direction::Press)
                .map_err(map_foreground_error)?;
            for (x, y) in resolved.path.iter().skip(1) {
                enigo
                    .move_mouse(*x, *y, Coordinate::Abs)
                    .map_err(map_foreground_error)?;
            }
            enigo
                .button(button, Direction::Release)
                .map_err(map_foreground_error)?;
        }
        ComputerAction::ListApps | ComputerAction::GetAppState | ComputerAction::Wait => {}
    }
    Ok(())
}

pub fn resolve_action(
    args: &ComputerArgs,
    state: &ComputerAppState,
) -> Result<ResolvedAction, String> {
    let element = match args.element_index {
        Some(index) => Some(state.element(index)?.clone()),
        None => None,
    };
    let point = match (&element, args.x, args.y) {
        (Some(element), None, None) => Some(element_click_point(element)),
        (_, Some(x), Some(y)) => Some(model_to_window(state, x, y)),
        _ => None,
    };
    let path = match args.path.as_ref() {
        Some(points) if points.len() >= 2 => points
            .iter()
            .map(|point| {
                let (x, y) = model_to_window(state, point.x, point.y);
                window_to_screen(state, x, y)
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(ResolvedAction {
        action: args.action,
        element,
        point,
        path,
        button: args.button(),
        scroll_x: args.scroll_x.unwrap_or(0),
        scroll_y: args.scroll_y.unwrap_or(0),
        text: args
            .text
            .clone()
            .or_else(|| args.value.clone())
            .unwrap_or_default(),
        keys: args.keys.clone().unwrap_or_default(),
    })
}

pub fn window_root_element(target: &AppTarget) -> AppElement {
    AppElement {
        index: 0,
        role: "window".to_string(),
        title: target.window_title.clone(),
        value: String::new(),
        bounds: WindowBounds {
            x: 0,
            y: 0,
            width: target.bounds.width,
            height: target.bounds.height,
        },
        actions: vec!["focus".to_string()],
    }
}

pub fn background_drop_reason(
    target: &AppTarget,
    action: ComputerAction,
    platform: &str,
) -> Option<String> {
    let hay = format!(
        "{} {} {}",
        target.name,
        target.identifier,
        target.class_name.clone().unwrap_or_default()
    )
    .to_ascii_lowercase();
    let chromium = [
        "chrome",
        "chromium",
        "msedge",
        "electron",
        "chrome_widgetwin",
    ]
    .iter()
    .any(|needle| hay.contains(needle));
    let uwp = hay.contains("windows.ui.core.corewindow") || hay.contains("applicationframewindow");
    let catalyst =
        hay.contains("iossupport") || hay.contains("/wrapper/") || hay.contains(".appex");
    match (platform, action) {
        ("windows", _) if chromium || uwp => Some(
            "该应用会丢弃后台 PostMessage（Chromium / UWP）。请改用 dispatch=foreground（会移动用户光标）"
                .to_string(),
        ),
        ("macos", ComputerAction::Scroll | ComputerAction::Drag) if chromium => Some(
            "Chromium 会丢弃后台 move/scroll。请改用 dispatch=foreground（会移动用户光标）"
                .to_string(),
        ),
        ("macos", _) if catalyst => Some(
            "Catalyst 应用会丢掉后台事件。请改用 dispatch=foreground（会移动用户光标）".to_string(),
        ),
        ("linux", ComputerAction::PressKey | ComputerAction::TypeText | ComputerAction::SetValue)
            if linux_session_type().as_deref() == Some("wayland") =>
        {
            Some(
                "Wayland 后台无法向该应用投递键盘事件，且不会改用全局 XTEST / enigo".to_string(),
            )
        }
        _ => None,
    }
}

pub fn current_platform() -> &'static str {
    std::env::consts::OS
}

pub fn linux_session_type() -> Option<String> {
    std::env::var("XDG_SESSION_TYPE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                Some("wayland".to_string())
            } else if std::env::var_os("DISPLAY").is_some() {
                Some("x11".to_string())
            } else {
                None
            }
        })
}

fn model_to_window(state: &ComputerAppState, x: f64, y: f64) -> (i32, i32) {
    scale_model_point(x, y, state.scale_x, state.scale_y)
}

fn window_to_screen(state: &ComputerAppState, x: i32, y: i32) -> (i32, i32) {
    (
        state.target.bounds.x.saturating_add(x),
        state.target.bounds.y.saturating_add(y),
    )
}

fn element_click_point(element: &AppElement) -> (i32, i32) {
    (
        element
            .bounds
            .x
            .saturating_add((element.bounds.width / 2) as i32),
        element
            .bounds
            .y
            .saturating_add((element.bounds.height / 2) as i32),
    )
}

fn require_abs_point(resolved: &ResolvedAction) -> Result<(i32, i32), String> {
    resolved
        .point
        .ok_or_else(|| "该动作需要 element_index 或 x / y".to_string())
}

fn enigo_button(button: ComputerButton) -> enigo::Button {
    match button {
        ComputerButton::Left => enigo::Button::Left,
        ComputerButton::Right => enigo::Button::Right,
        ComputerButton::Middle => enigo::Button::Middle,
    }
}

fn press_keys(enigo: &mut enigo::Enigo, keys: &[String]) -> Result<(), String> {
    use enigo::{Direction, Keyboard};

    let mapped = super::desktop::map_keys(keys)?;
    if mapped.is_empty() {
        return Err("按键动作需要 keys".to_string());
    }
    let last = mapped.len() - 1;
    for key in &mapped[..last] {
        enigo
            .key(*key, Direction::Press)
            .map_err(map_foreground_error)?;
    }
    enigo
        .key(mapped[last], Direction::Click)
        .map_err(map_foreground_error)?;
    for key in mapped[..last].iter().rev() {
        enigo
            .key(*key, Direction::Release)
            .map_err(map_foreground_error)?;
    }
    Ok(())
}

fn map_foreground_error(error: impl std::fmt::Display) -> String {
    format!("前台注入失败（会移动用户光标）：{error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_unavailable_keeps_prefix() {
        let message = BackgroundError::unavailable("Chromium 会丢弃后台消息").into_message();
        assert!(message.starts_with("background_unavailable:"), "{message}");
        assert!(message.contains("Chromium"), "{message}");
    }

    #[test]
    fn windows_chromium_is_unavailable_without_fallback() {
        let target = AppTarget {
            name: "Google Chrome".into(),
            identifier: r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
            pid: 1,
            window_id: 1,
            window_title: "Chrome".into(),
            bounds: WindowBounds {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            class_name: Some("Chrome_WidgetWin_1".into()),
            focused: false,
        };
        let reason = background_drop_reason(&target, ComputerAction::Click, "windows");
        assert!(reason.is_some(), "chrome should drop PostMessage");
        assert!(
            format_background_unavailable(&reason.unwrap()).starts_with("background_unavailable:")
        );
    }

    #[test]
    fn ordinary_app_is_not_marked_unavailable() {
        let target = AppTarget {
            name: "Notes".into(),
            identifier: "com.apple.Notes".into(),
            pid: 2,
            window_id: 2,
            window_title: "Notes".into(),
            bounds: WindowBounds {
                x: 0,
                y: 0,
                width: 400,
                height: 400,
            },
            class_name: None,
            focused: true,
        };
        assert_eq!(
            background_drop_reason(&target, ComputerAction::Click, "macos"),
            None
        );
    }
}
