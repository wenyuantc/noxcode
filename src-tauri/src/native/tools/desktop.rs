//! Native Agent `Computer` 工具：按应用读取状态并默认后台投递。
//!
//! 默认 `dispatch=background`，不会悄悄回退到全局 enigo。参数解析、状态校验
//! 与按应用规则可以在无显示器环境单测。

use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::app_target::{
    format_app_list, format_app_tree, list_apps, resolve_app, ComputerAppState,
};
use super::background_input::{
    apply_action, capture_app_window, collect_tree, format_background_unavailable, resolve_action,
};
use super::dispatch::{ToolCtx, ToolOutput};

pub use super::app_target::{fit_long_edge, scale_model_point, MAX_IMAGE_EDGE};

pub const DEFAULT_WAIT_MS: u64 = 500;
pub const MAX_WAIT_MS: u64 = 10_000;

static DESKTOP_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerAction {
    ListApps,
    #[serde(alias = "screenshot")]
    GetAppState,
    Click,
    SetValue,
    #[serde(alias = "type")]
    TypeText,
    #[serde(alias = "keypress")]
    PressKey,
    Scroll,
    Drag,
    Wait,
}

impl ComputerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListApps => "list_apps",
            Self::GetAppState => "get_app_state",
            Self::Click => "click",
            Self::SetValue => "set_value",
            Self::TypeText => "type_text",
            Self::PressKey => "press_key",
            Self::Scroll => "scroll",
            Self::Drag => "drag",
            Self::Wait => "wait",
        }
    }

    pub fn zh_label(self) -> &'static str {
        match self {
            Self::ListApps => "列出应用",
            Self::GetAppState => "读取状态",
            Self::Click => "点击",
            Self::SetValue => "设值",
            Self::TypeText => "输入",
            Self::PressKey => "按键",
            Self::Scroll => "滚动",
            Self::Drag => "拖拽",
            Self::Wait => "等待",
        }
    }

    pub fn needs_app_state(self) -> bool {
        matches!(
            self,
            Self::Click
                | Self::SetValue
                | Self::TypeText
                | Self::PressKey
                | Self::Scroll
                | Self::Drag
        )
    }

    pub fn is_write(self) -> bool {
        self.needs_app_state()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerDispatch {
    #[default]
    Background,
    Foreground,
}

impl ComputerDispatch {
    pub fn zh_label(self) -> &'static str {
        match self {
            Self::Background => "后台",
            Self::Foreground => "前台",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ComputerPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComputerArgs {
    pub action: ComputerAction,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub dispatch: ComputerDispatch,
    #[serde(default)]
    pub element_index: Option<u32>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub button: Option<ComputerButton>,
    #[serde(default)]
    pub path: Option<Vec<ComputerPoint>>,
    #[serde(default)]
    pub scroll_x: Option<i32>,
    #[serde(default)]
    pub scroll_y: Option<i32>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

impl ComputerArgs {
    pub fn button(&self) -> ComputerButton {
        self.button.unwrap_or_default()
    }

    pub fn dispatch(&self) -> ComputerDispatch {
        self.dispatch
    }

    pub fn app_label(&self) -> Option<&str> {
        self.app
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
    }

    pub fn zh_brief(&self) -> String {
        let dispatch = match self.dispatch {
            ComputerDispatch::Background => None,
            ComputerDispatch::Foreground => Some("前台（会移动光标）"),
        };
        let app = self.app_label();
        let core = match self.action {
            ComputerAction::Click => {
                if let Some(index) = self.element_index {
                    format!("{} #{}", self.action.zh_label(), index)
                } else if let (Some(x), Some(y)) = (self.x, self.y) {
                    format!(
                        "{} ({}, {})",
                        self.action.zh_label(),
                        fmt_coord(x),
                        fmt_coord(y)
                    )
                } else {
                    self.action.zh_label().to_string()
                }
            }
            ComputerAction::PressKey => {
                let keys = self
                    .keys
                    .as_ref()
                    .map(|keys| keys.join("+"))
                    .unwrap_or_default();
                if keys.is_empty() {
                    self.action.zh_label().to_string()
                } else {
                    format!("{} {keys}", self.action.zh_label())
                }
            }
            ComputerAction::TypeText | ComputerAction::SetValue => {
                let preview = self
                    .text
                    .as_deref()
                    .or(self.value.as_deref())
                    .unwrap_or("")
                    .chars()
                    .take(24)
                    .collect::<String>();
                if preview.is_empty() {
                    self.action.zh_label().to_string()
                } else {
                    format!("{} {preview}", self.action.zh_label())
                }
            }
            ComputerAction::Wait => format!(
                "{} {} ms",
                self.action.zh_label(),
                self.duration_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS)
            ),
            ComputerAction::GetAppState
            | ComputerAction::ListApps
            | ComputerAction::Scroll
            | ComputerAction::Drag => self.action.zh_label().to_string(),
        };
        match (app, dispatch) {
            (Some(app), Some(mode)) => format!("{mode}{core} {app}"),
            (Some(app), None) => format!("{core} {app}"),
            (None, Some(mode)) => format!("{mode}{core}"),
            (None, None) => core,
        }
    }

    pub fn zh_title(&self) -> String {
        format!("电脑控制 · {}", self.zh_brief())
    }
}

fn fmt_coord(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// 权限弹窗摘要：`电脑控制：点击 Safari #3`。
pub fn risk_summary(arguments: &str) -> String {
    match parse_computer_args(arguments) {
        Ok(args) => format!("电脑控制：{}", args.zh_brief()),
        Err(_) => "电脑控制：未知动作".to_string(),
    }
}

pub fn parse_computer_args(arguments: &str) -> Result<ComputerArgs, String> {
    let raw = if arguments.trim().is_empty() {
        "{}".to_string()
    } else {
        arguments.to_string()
    };
    let args: ComputerArgs = serde_json::from_str(&raw)
        .map_err(|error| format!("电脑控制参数不是合法 JSON: {error}"))?;
    validate_computer_args(&args)?;
    Ok(args)
}

pub fn validate_computer_args(args: &ComputerArgs) -> Result<(), String> {
    match args.action {
        ComputerAction::GetAppState => {
            if args.app_label().is_none() {
                return Err(
                    "get_app_state 需要 app（名称、bundle id 或可执行文件路径）".to_string()
                );
            }
        }
        ComputerAction::Click => {
            if args.element_index.is_none() {
                require_point(args.x, args.y)?;
            }
        }
        ComputerAction::SetValue => {
            if args.element_index.is_none() {
                return Err("set_value 需要 element_index".to_string());
            }
            if args
                .text
                .as_deref()
                .or(args.value.as_deref())
                .unwrap_or("")
                .is_empty()
            {
                return Err("set_value 需要 text".to_string());
            }
        }
        ComputerAction::Drag => {
            let path = args
                .path
                .as_ref()
                .filter(|path| path.len() >= 2)
                .ok_or_else(|| "拖拽需要 path，至少包含两个 {x,y} 点".to_string())?;
            for point in path {
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err("拖拽路径包含无效坐标".to_string());
                }
            }
        }
        ComputerAction::Scroll => {
            if args.scroll_x.unwrap_or(0) == 0 && args.scroll_y.unwrap_or(0) == 0 {
                return Err("滚动需要 scroll_x 或 scroll_y".to_string());
            }
        }
        ComputerAction::TypeText => {
            if args.text.as_deref().unwrap_or("").is_empty() {
                return Err("输入动作需要 text".to_string());
            }
        }
        ComputerAction::PressKey => {
            let keys = args
                .keys
                .as_ref()
                .ok_or_else(|| "按键动作需要 keys".to_string())?;
            if keys.iter().all(|key| key.trim().is_empty()) {
                return Err("按键动作需要 keys".to_string());
            }
        }
        ComputerAction::Wait | ComputerAction::ListApps => {}
    }
    Ok(())
}

fn require_point(x: Option<f64>, y: Option<f64>) -> Result<(f64, f64), String> {
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok((x, y)),
        _ => Err("该动作需要 element_index 或 x / y（相对本次窗口截图左上角）".to_string()),
    }
}

pub fn unavailable_reason(ctx: &ToolCtx) -> Option<String> {
    if ctx.ssh.is_some() {
        return Some("SSH 工作区不可用电脑控制".to_string());
    }
    if !ctx.computer_control_enabled {
        return Some("电脑控制未开启".to_string());
    }
    if ctx.is_plan_mode() || ctx.is_read_only() {
        return Some("计划模式不可用电脑控制".to_string());
    }
    None
}

pub fn require_existing_state(
    state: Option<&ComputerAppState>,
    args: &ComputerArgs,
) -> Result<ComputerAppState, String> {
    if !args.action.needs_app_state() {
        return Err("该动作不需要 get_app_state".to_string());
    }
    let Some(state) = state else {
        return Err("请先调用 get_app_state 再执行该动作；元素下标只对这一轮有效".to_string());
    };
    if let Some(app) = args.app_label() {
        if !state.target.matches_query(app) {
            return Err(format!(
                "get_app_state 的应用是 {}，与当前动作 {app} 不一致，请先刷新状态",
                state.target.identifier
            ));
        }
    }
    Ok(state.clone())
}

pub async fn execute(ctx: &ToolCtx, arguments: &str) -> Result<ToolOutput, String> {
    if let Some(reason) = unavailable_reason(ctx) {
        return Err(reason);
    }
    let args = parse_computer_args(arguments)?;
    let state = ctx.computer_app_state.clone();
    let _guard = DESKTOP_LOCK.lock().await;
    tokio::task::spawn_blocking(move || run_computer_action(args, &state))
        .await
        .map_err(|error| format!("电脑控制任务失败: {error}"))?
}

fn run_computer_action(
    args: ComputerArgs,
    state: &std::sync::Arc<std::sync::Mutex<Option<ComputerAppState>>>,
) -> Result<ToolOutput, String> {
    match args.action {
        ComputerAction::Wait => {
            let ms = args
                .duration_ms
                .unwrap_or(DEFAULT_WAIT_MS)
                .clamp(1, MAX_WAIT_MS);
            thread::sleep(Duration::from_millis(ms));
            Ok(ToolOutput::text(format!(
                "{}\nduration_ms={ms}",
                args.zh_title()
            )))
        }
        ComputerAction::ListApps => {
            let apps = list_apps()?;
            Ok(ToolOutput::text(format!(
                "{}\n{}",
                args.zh_title(),
                format_app_list(&apps)
            )))
        }
        ComputerAction::GetAppState => {
            let snapshot = snapshot_app(args.app_label().unwrap_or_default())?;
            if let Ok(mut slot) = state.lock() {
                *slot = Some(snapshot.clone());
            }
            Ok(state_output(&args, &snapshot))
        }
        _ => {
            let current = {
                let guard = state.lock().map_err(|_| "电脑控制状态锁损坏".to_string())?;
                require_existing_state(guard.as_ref(), &args)?
            };
            let resolved = resolve_action(&args, &current)?;
            apply_action(args.dispatch(), &args, &current, &resolved)?;
            match snapshot_app(&current.target.identifier) {
                Ok(fresh) => {
                    if let Ok(mut slot) = state.lock() {
                        *slot = Some(fresh.clone());
                    }
                    Ok(state_output(&args, &fresh))
                }
                Err(error) => Ok(ToolOutput::text(format!(
                    "{}\n动作已执行，但刷新窗口状态失败：{error}",
                    args.zh_title()
                ))),
            }
        }
    }
}

fn snapshot_app(query: &str) -> Result<ComputerAppState, String> {
    let target = resolve_app(query)?;
    let (elements, mut notes) = collect_tree(&target);
    let image = match capture_app_window(&target) {
        Ok(image) => Some(image),
        Err(error) => {
            notes.push(error);
            None
        }
    };
    Ok(ComputerAppState {
        target,
        elements,
        width: image.as_ref().map(|item| item.width).unwrap_or(1),
        height: image.as_ref().map(|item| item.height).unwrap_or(1),
        scale_x: image.as_ref().map(|item| item.scale_x).unwrap_or(1.0),
        scale_y: image.as_ref().map(|item| item.scale_y).unwrap_or(1.0),
        notes,
        image,
    })
}

fn state_output(args: &ComputerArgs, state: &ComputerAppState) -> ToolOutput {
    let mut text = format!("{}\n{}", args.zh_title(), format_app_tree(state));
    if args.dispatch() == ComputerDispatch::Foreground {
        text.push_str("\nnote: 已使用 dispatch=foreground，会移动用户光标");
    }
    ToolOutput {
        text,
        images: state
            .image
            .as_ref()
            .map(|image| vec![image.image.clone()])
            .unwrap_or_default(),
        ok: true,
    }
}

pub fn map_keys(keys: &[String]) -> Result<Vec<enigo::Key>, String> {
    let mut mapped = Vec::new();
    for raw in keys {
        for part in raw.split(['+', '-']) {
            let part = part.trim();
            if !part.is_empty() {
                mapped.push(map_key(part)?);
            }
        }
    }
    Ok(mapped)
}

pub fn map_key(name: &str) -> Result<enigo::Key, String> {
    use enigo::Key;
    let normalized = name.trim().to_ascii_lowercase();
    Ok(match normalized.as_str() {
        "ctrl" | "control" => Key::Control,
        "alt" | "option" => Key::Alt,
        "shift" => Key::Shift,
        "super" | "meta" | "cmd" | "command" | "win" | "windows" => Key::Meta,
        "enter" | "return" => Key::Return,
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "capslock" => Key::CapsLock,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Key::Unicode(ch),
                _ => return Err(format!("不支持的按键: {name}")),
            }
        }
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputerPermissionFlag {
    pub granted: Option<bool>,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputerPermissionStatus {
    pub platform: String,
    pub session_type: Option<String>,
    pub screenshot: ComputerPermissionFlag,
    pub input: ComputerPermissionFlag,
    pub can_open_settings: bool,
    pub hint: String,
}

pub fn query_permission_status() -> ComputerPermissionStatus {
    let hint = "默认后台控制已打开的应用，不移动用户光标。后台做不到时返回 background_unavailable，不会悄悄改用全局键鼠。请只在信任当前渠道与工作区时开启。".to_string();
    #[cfg(target_os = "macos")]
    {
        let screen = macos_screen_granted();
        let input = macos_input_granted();
        ComputerPermissionStatus {
            platform: "macos".to_string(),
            session_type: None,
            screenshot: ComputerPermissionFlag {
                granted: Some(screen),
                label: if screen {
                    "已授权屏幕录制".to_string()
                } else {
                    "未授权屏幕录制".to_string()
                },
                detail: "按窗口截图需要系统设置 → 隐私与安全性 → 屏幕录制。窗口在其他 Space 上时没有像素。".to_string(),
            },
            input: ComputerPermissionFlag {
                granted: Some(input),
                label: if input {
                    "已授权辅助功能".to_string()
                } else {
                    "未授权辅助功能".to_string()
                },
                detail: "后台 AX / CGEventPostToPid 需要系统设置 → 隐私与安全性 → 辅助功能。".to_string(),
            },
            can_open_settings: true,
            hint,
        }
    }
    #[cfg(target_os = "windows")]
    {
        ComputerPermissionStatus {
            platform: "windows".to_string(),
            session_type: None,
            screenshot: ComputerPermissionFlag {
                granted: None,
                label: "PrintWindow 按窗口截图".to_string(),
                detail: "截窗走 PrintWindow，并用 DWM 扩展边框。UAC 安全桌面无法注入。".to_string(),
            },
            input: ComputerPermissionFlag {
                granted: None,
                label: "UIA + PostMessage 后台投递".to_string(),
                detail: "部分 Chromium / UWP 会丢弃后台消息并返回 background_unavailable，不会改用 SendInput。".to_string(),
            },
            can_open_settings: true,
            hint,
        }
    }
    #[cfg(target_os = "linux")]
    {
        let session = super::background_input::linux_session_type();
        let wayland = session.as_deref() == Some("wayland");
        ComputerPermissionStatus {
            platform: "linux".to_string(),
            session_type: session.clone(),
            screenshot: ComputerPermissionFlag {
                granted: None,
                label: if wayland {
                    "Wayland 可能只能返回树".to_string()
                } else {
                    "X11 按窗口截图".to_string()
                },
                detail: if wayland {
                    "Wayland 能抓到的窗口才有像素；否则只返回 AT-SPI 树。".to_string()
                } else {
                    "X11 按 window id 抓 backing store。".to_string()
                },
            },
            input: ComputerPermissionFlag {
                granted: None,
                label: "AT-SPI / 窗口消息".to_string(),
                detail: "后台使用 AT-SPI 或 XSendEvent，不会使用全局 XTEST / enigo。".to_string(),
            },
            can_open_settings: false,
            hint,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        ComputerPermissionStatus {
            platform: std::env::consts::OS.to_string(),
            session_type: None,
            screenshot: ComputerPermissionFlag {
                granted: None,
                label: "未知平台".to_string(),
                detail: "当前系统未实现电脑控制权限探测。".to_string(),
            },
            input: ComputerPermissionFlag {
                granted: None,
                label: "未知平台".to_string(),
                detail: "当前系统未实现电脑控制权限探测。".to_string(),
            },
            can_open_settings: false,
            hint,
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_screen_granted() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[cfg(target_os = "macos")]
fn macos_input_granted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

fn open_privacy_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    #[cfg(target_os = "macos")]
    {
        app.opener()
            .open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                None::<&str>,
            )
            .map_err(|error| format!("无法打开屏幕录制设置: {error}"))?;
        let _ = app.opener().open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            None::<&str>,
        );
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        app.opener()
            .open_url("ms-settings:privacy", None::<&str>)
            .map_err(|error| format!("无法打开系统隐私设置: {error}"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        Err(
            "当前系统没有统一的隐私设置入口。Wayland 请在 portal 提示中授权，或改用 X11。"
                .to_string(),
        )
    }
}

#[tauri::command]
pub async fn get_computer_permission_status() -> Result<ComputerPermissionStatus, String> {
    Ok(query_permission_status())
}

#[tauri::command]
pub async fn open_computer_privacy_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<(), String> {
    open_privacy_settings(&app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tools::app_target::{AppElement, AppTarget, WindowBounds};
    use crate::native::tools::dispatch::ToolCtx;
    use crate::native::tools::local::LocalWorkspace;

    fn sample_state() -> ComputerAppState {
        ComputerAppState {
            target: AppTarget {
                name: "Safari".into(),
                identifier: "com.apple.Safari".into(),
                pid: 12,
                window_id: 34,
                window_title: "Start".into(),
                bounds: WindowBounds {
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                },
                class_name: None,
                focused: true,
            },
            elements: vec![AppElement {
                index: 0,
                role: "window".into(),
                title: "Start".into(),
                value: String::new(),
                bounds: WindowBounds {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                actions: vec!["press".into()],
            }],
            width: 400,
            height: 300,
            scale_x: 2.0,
            scale_y: 2.0,
            notes: Vec::new(),
            image: None,
        }
    }

    #[test]
    fn parse_app_centric_actions_and_aliases() {
        let listed = parse_computer_args(r#"{"action":"list_apps"}"#).expect("list");
        assert_eq!(listed.action, ComputerAction::ListApps);
        assert_eq!(listed.dispatch, ComputerDispatch::Background);
        let state =
            parse_computer_args(r#"{"action":"screenshot","app":"Safari"}"#).expect("alias");
        assert_eq!(state.action, ComputerAction::GetAppState);
        let click = parse_computer_args(
            r#"{"action":"click","app":"Safari","element_index":3,"dispatch":"background"}"#,
        )
        .expect("click");
        assert_eq!(click.element_index, Some(3));
        let typed = parse_computer_args(r#"{"action":"type","text":"hi"}"#).expect("type");
        assert_eq!(typed.action, ComputerAction::TypeText);
        let keys = parse_computer_args(r#"{"action":"keypress","keys":["enter"]}"#).expect("keys");
        assert_eq!(keys.action, ComputerAction::PressKey);
        let (w, h) = fit_long_edge(2560, 1440, MAX_IMAGE_EDGE);
        assert_eq!((w, h), (1280, 720));
        assert_eq!(scale_model_point(100.0, 50.0, 2.0, 2.0), (200, 100));
    }

    #[test]
    fn parse_rejects_incomplete_actions() {
        assert!(parse_computer_args(r#"{"action":"click"}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"get_app_state"}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"drag","path":[{"x":1,"y":1}]}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"scroll"}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"type_text","text":""}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"press_key","keys":[]}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"set_value","element_index":0}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"list_apps"}"#).is_ok());
        assert!(parse_computer_args(r#"{"action":"wait","duration_ms":200}"#).is_ok());
        assert!(parse_computer_args(r#"{"action":"click","x":1,"y":2}"#).is_ok());
    }

    #[tokio::test]
    async fn execute_click_without_state_does_not_touch_os() {
        let mut ctx = ToolCtx::new(LocalWorkspace::new(std::env::temp_dir()));
        ctx.computer_control_enabled = true;
        let err = execute(&ctx, r#"{"action":"click","element_index":0}"#)
            .await
            .expect_err("need state");
        assert!(err.contains("get_app_state"), "{err}");
    }

    #[test]
    fn actions_require_fresh_get_app_state() {
        let args = parse_computer_args(r#"{"action":"click","element_index":0,"app":"Safari"}"#)
            .expect("parse");
        assert!(args.action.needs_app_state());
        let missing = require_existing_state(None, &args).expect_err("no state");
        assert!(missing.contains("get_app_state"), "{missing}");
        let ok = require_existing_state(Some(&sample_state()), &args).expect("ok");
        assert_eq!(ok.target.identifier, "com.apple.Safari");
        let mismatch = parse_computer_args(r#"{"action":"click","element_index":0,"app":"Notes"}"#)
            .expect("parse");
        assert!(require_existing_state(Some(&sample_state()), &mismatch).is_err());
    }

    #[test]
    fn background_failure_never_looks_like_success() {
        let message = format_background_unavailable("Chromium 会丢弃后台消息");
        assert!(message.starts_with("background_unavailable:"));
        assert!(!message.contains("enigo"));
    }

    #[test]
    fn maps_super_to_meta() {
        assert!(matches!(map_key("super").unwrap(), enigo::Key::Meta));
        assert!(matches!(map_key("ctrl").unwrap(), enigo::Key::Control));
        assert!(matches!(map_key("s").unwrap(), enigo::Key::Unicode('s')));
        assert!(map_key("not-a-key").is_err());
    }

    #[test]
    fn gate_rejects_disabled_ssh_and_plan() {
        let ctx = ToolCtx::new(LocalWorkspace::new(std::env::temp_dir()));
        assert_eq!(unavailable_reason(&ctx).as_deref(), Some("电脑控制未开启"));
        let mut enabled = ctx.clone();
        enabled.computer_control_enabled = true;
        enabled.set_plan_mode(true);
        assert_eq!(
            unavailable_reason(&enabled).as_deref(),
            Some("计划模式不可用电脑控制")
        );
    }

    #[test]
    fn risk_summary_includes_app_and_action() {
        let summary = risk_summary(r#"{"action":"click","app":"Safari","element_index":12}"#);
        assert!(summary.contains("电脑控制"));
        assert!(summary.contains("点击"));
        assert!(summary.contains("Safari"));
        assert!(summary.contains("12"));
    }
}
