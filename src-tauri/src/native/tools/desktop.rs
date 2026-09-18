//! Native Agent `Computer` 工具：截取本机桌面并按坐标注入键鼠。
//!
//! 实际截屏 / 注入依赖显示器与 OS 权限；参数解析、坐标缩放与开关门闸
//! 可以在无显示器环境单测。

use std::io::Cursor;
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::dispatch::{ToolCtx, ToolOutput};
use crate::native::model::types::NativeImage;

/// 送给模型的截图最长边（像素）。
pub const MAX_IMAGE_EDGE: u32 = 1280;
pub const DEFAULT_WAIT_MS: u64 = 500;
pub const MAX_WAIT_MS: u64 = 10_000;

static DESKTOP_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerAction {
    Screenshot,
    Click,
    DoubleClick,
    Move,
    Drag,
    Scroll,
    Type,
    Keypress,
    Wait,
}

impl ComputerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::Click => "click",
            Self::DoubleClick => "double_click",
            Self::Move => "move",
            Self::Drag => "drag",
            Self::Scroll => "scroll",
            Self::Type => "type",
            Self::Keypress => "keypress",
            Self::Wait => "wait",
        }
    }

    pub fn zh_label(self) -> &'static str {
        match self {
            Self::Screenshot => "截图",
            Self::Click => "点击",
            Self::DoubleClick => "双击",
            Self::Move => "移动",
            Self::Drag => "拖拽",
            Self::Scroll => "滚动",
            Self::Type => "输入",
            Self::Keypress => "按键",
            Self::Wait => "等待",
        }
    }

    pub fn is_write(self) -> bool {
        !matches!(self, Self::Screenshot | Self::Wait)
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
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub display: Option<u32>,
}

impl ComputerArgs {
    pub fn button(&self) -> ComputerButton {
        self.button.unwrap_or_default()
    }

    pub fn display_index(&self) -> Option<u32> {
        self.display
    }

    pub fn zh_brief(&self) -> String {
        match self.action {
            ComputerAction::Click | ComputerAction::DoubleClick | ComputerAction::Move => {
                match (self.x, self.y) {
                    (Some(x), Some(y)) => {
                        format!(
                            "{} ({}, {})",
                            self.action.zh_label(),
                            fmt_coord(x),
                            fmt_coord(y)
                        )
                    }
                    _ => self.action.zh_label().to_string(),
                }
            }
            ComputerAction::Keypress => {
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
            ComputerAction::Type => {
                let preview = self
                    .text
                    .as_deref()
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
            other => other.zh_label().to_string(),
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

/// 权限弹窗摘要：`电脑控制：点击 (x, y)`。
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
        ComputerAction::Click | ComputerAction::DoubleClick | ComputerAction::Move => {
            require_point(args.x, args.y)?;
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
        ComputerAction::Type => {
            if args.text.as_deref().unwrap_or("").is_empty() {
                return Err("输入动作需要 text".to_string());
            }
        }
        ComputerAction::Keypress => {
            let keys = args
                .keys
                .as_ref()
                .ok_or_else(|| "按键动作需要 keys".to_string())?;
            if keys.iter().all(|key| key.trim().is_empty()) {
                return Err("按键动作需要 keys".to_string());
            }
        }
        ComputerAction::Wait | ComputerAction::Screenshot => {}
    }
    Ok(())
}

fn require_point(x: Option<f64>, y: Option<f64>) -> Result<(f64, f64), String> {
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok((x, y)),
        _ => Err("该动作需要 x / y 坐标（相对本次截图左上角）".to_string()),
    }
}

/// 按最长边缩放后的目标尺寸。
pub fn fit_long_edge(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let long = width.max(height);
    if long == 0 || long <= max_edge {
        return (width.max(1), height.max(1));
    }
    let scale = f64::from(max_edge) / f64::from(long);
    let next_w = (f64::from(width) * scale).round().max(1.0) as u32;
    let next_h = (f64::from(height) * scale).round().max(1.0) as u32;
    (next_w, next_h)
}

/// 把模型坐标（相对已缩放截图）乘回物理 / 输入坐标系。
pub fn scale_model_point(x: f64, y: f64, scale_x: f64, scale_y: f64) -> (i32, i32) {
    ((x * scale_x).round() as i32, (y * scale_y).round() as i32)
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

pub async fn execute(ctx: &ToolCtx, arguments: &str) -> Result<ToolOutput, String> {
    if let Some(reason) = unavailable_reason(ctx) {
        return Err(reason);
    }
    let args = parse_computer_args(arguments)?;
    let _guard = DESKTOP_LOCK.lock().await;
    tokio::task::spawn_blocking(move || run_computer_action(args))
        .await
        .map_err(|error| format!("电脑控制任务失败: {error}"))?
}

fn run_computer_action(args: ComputerArgs) -> Result<ToolOutput, String> {
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
        ComputerAction::Screenshot => capture_output(&args),
        _ => {
            apply_input(&args)?;
            capture_output(&args)
        }
    }
}

fn capture_output(args: &ComputerArgs) -> Result<ToolOutput, String> {
    let shot = capture_screenshot(args.display_index())?;
    let text = format!(
        "{}\ndisplay={} width={} height={} scale_x={:.4} scale_y={:.4}",
        args.zh_title(),
        shot.display,
        shot.width,
        shot.height,
        shot.scale_x,
        shot.scale_y
    );
    Ok(ToolOutput {
        text,
        images: vec![shot.image],
        ok: true,
    })
}

struct ScreenshotPayload {
    image: NativeImage,
    display: u32,
    width: u32,
    height: u32,
    scale_x: f64,
    scale_y: f64,
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
}

fn capture_screenshot(display: Option<u32>) -> Result<ScreenshotPayload, String> {
    let (monitor, display_index) = select_monitor(display)?;
    let rgba = monitor.capture_image().map_err(map_screenshot_error)?;
    let capture_width = rgba.width();
    let capture_height = rgba.height();
    if capture_width == 0 || capture_height == 0 {
        return Err("截图为空".to_string());
    }
    let (width, height) = fit_long_edge(capture_width, capture_height, MAX_IMAGE_EDGE);
    let dynamic = DynamicImage::ImageRgba8(rgba);
    let resized = if width != capture_width || height != capture_height {
        dynamic.resize_exact(width, height, image::imageops::FilterType::Triangle)
    } else {
        dynamic
    };
    let mut png = Cursor::new(Vec::new());
    resized
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| format!("编码截图失败: {error}"))?;
    let image = NativeImage {
        name: format!("screenshot-{display_index}.png"),
        mime_type: "image/png".to_string(),
        data_base64: BASE64.encode(png.into_inner()),
    };
    let monitor_width = monitor_metric(monitor.width(), "宽度")?;
    let monitor_height = monitor_metric(monitor.height(), "高度")?;
    let monitor_x = monitor_metric_i32(monitor.x(), "原点 X")?;
    let monitor_y = monitor_metric_i32(monitor.y(), "原点 Y")?;
    Ok(ScreenshotPayload {
        image,
        display: display_index,
        width,
        height,
        scale_x: f64::from(capture_width) / f64::from(width),
        scale_y: f64::from(capture_height) / f64::from(height),
        monitor_x,
        monitor_y,
        monitor_width,
        monitor_height,
    })
}

fn monitor_metric<T, E: std::fmt::Display>(value: Result<T, E>, label: &str) -> Result<T, String> {
    value.map_err(|error| format!("无法读取显示器{label}：{error}"))
}

fn monitor_metric_i32<E: std::fmt::Display>(
    value: Result<i32, E>,
    label: &str,
) -> Result<i32, String> {
    value.map_err(|error| format!("无法读取显示器{label}：{error}"))
}

fn select_monitor(display: Option<u32>) -> Result<(xcap::Monitor, u32), String> {
    let mut monitors = xcap::Monitor::all().map_err(|error| format!("无法枚举显示器：{error}"))?;
    if monitors.is_empty() {
        return Err("未找到可用显示器".to_string());
    }
    let index = if let Some(index) = display {
        if (index as usize) >= monitors.len() {
            return Err(format!("显示器序号 {index} 不存在"));
        }
        index as usize
    } else {
        monitors.iter().position(monitor_is_primary).unwrap_or(0)
    };
    Ok((monitors.swap_remove(index), index as u32))
}

fn monitor_is_primary(monitor: &xcap::Monitor) -> bool {
    monitor.is_primary().unwrap_or(false)
}

fn map_screenshot_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    #[cfg(target_os = "macos")]
    {
        format!("无法截取屏幕，请在系统设置中授予屏幕录制权限：{text}")
    }
    #[cfg(target_os = "linux")]
    {
        if linux_session_type().as_deref() == Some("wayland") {
            format!("无法截取屏幕。Wayland 需要授权屏幕共享 portal，或改用 X11 会话：{text}")
        } else {
            format!("无法截取屏幕：{text}")
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        format!("无法截取屏幕：{text}")
    }
}

fn map_input_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    #[cfg(target_os = "windows")]
    {
        let lower = text.to_ascii_lowercase();
        if lower.contains("uac")
            || lower.contains("secure desktop")
            || lower.contains("elevation")
            || text.contains("安全桌面")
        {
            format!("无法向安全桌面注入输入（可能处于 UAC 提示）：{text}")
        } else {
            format!("无法注入键鼠：{text}")
        }
    }
    #[cfg(target_os = "macos")]
    {
        format!("无法注入键鼠，请在系统设置中授予辅助功能权限：{text}")
    }
    #[cfg(target_os = "linux")]
    {
        if linux_session_type().as_deref() == Some("wayland") {
            format!("无法注入键鼠。Wayland 需要授权远程桌面 portal，或改用 X11 会话：{text}")
        } else {
            format!("无法注入键鼠：{text}")
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        format!("无法注入键鼠：{text}")
    }
}

fn apply_input(args: &ComputerArgs) -> Result<(), String> {
    use enigo::{Axis, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};

    let shot = capture_screenshot(args.display_index()).ok();
    let mapping = InputMapping::from_screenshot(shot.as_ref());
    let mut enigo = Enigo::new(&Settings::default()).map_err(map_input_error)?;
    match args.action {
        ComputerAction::Move => {
            let (x, y) = require_point(args.x, args.y)?;
            let (abs_x, abs_y) = mapping.to_abs(x, y);
            enigo
                .move_mouse(abs_x, abs_y, Coordinate::Abs)
                .map_err(map_input_error)?;
        }
        ComputerAction::Click => {
            let (x, y) = require_point(args.x, args.y)?;
            let (abs_x, abs_y) = mapping.to_abs(x, y);
            enigo
                .move_mouse(abs_x, abs_y, Coordinate::Abs)
                .map_err(map_input_error)?;
            enigo
                .button(enigo_button(args.button()), Direction::Click)
                .map_err(map_input_error)?;
        }
        ComputerAction::DoubleClick => {
            let (x, y) = require_point(args.x, args.y)?;
            let (abs_x, abs_y) = mapping.to_abs(x, y);
            enigo
                .move_mouse(abs_x, abs_y, Coordinate::Abs)
                .map_err(map_input_error)?;
            let button = enigo_button(args.button());
            enigo
                .button(button, Direction::Click)
                .map_err(map_input_error)?;
            thread::sleep(Duration::from_millis(50));
            enigo
                .button(button, Direction::Click)
                .map_err(map_input_error)?;
        }
        ComputerAction::Drag => {
            let path = args.path.as_ref().expect("validated");
            let (start_x, start_y) = mapping.to_abs(path[0].x, path[0].y);
            enigo
                .move_mouse(start_x, start_y, Coordinate::Abs)
                .map_err(map_input_error)?;
            let button = enigo_button(args.button());
            enigo
                .button(button, Direction::Press)
                .map_err(map_input_error)?;
            for point in path.iter().skip(1) {
                let (abs_x, abs_y) = mapping.to_abs(point.x, point.y);
                enigo
                    .move_mouse(abs_x, abs_y, Coordinate::Abs)
                    .map_err(map_input_error)?;
            }
            enigo
                .button(button, Direction::Release)
                .map_err(map_input_error)?;
        }
        ComputerAction::Scroll => {
            if let (Some(x), Some(y)) = (args.x, args.y) {
                let (abs_x, abs_y) = mapping.to_abs(x, y);
                enigo
                    .move_mouse(abs_x, abs_y, Coordinate::Abs)
                    .map_err(map_input_error)?;
            }
            if let Some(dx) = args.scroll_x.filter(|value| *value != 0) {
                enigo
                    .scroll(dx, Axis::Horizontal)
                    .map_err(map_input_error)?;
            }
            if let Some(dy) = args.scroll_y.filter(|value| *value != 0) {
                enigo.scroll(dy, Axis::Vertical).map_err(map_input_error)?;
            }
        }
        ComputerAction::Type => {
            let text = args.text.as_deref().unwrap_or("");
            enigo.text(text).map_err(map_input_error)?;
        }
        ComputerAction::Keypress => {
            press_keys(&mut enigo, args.keys.as_deref().unwrap_or(&[]))?;
        }
        ComputerAction::Screenshot | ComputerAction::Wait => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct InputMapping {
    monitor_x: i32,
    monitor_y: i32,
    scale_x: f64,
    scale_y: f64,
}

impl InputMapping {
    fn from_screenshot(shot: Option<&ScreenshotPayload>) -> Self {
        match shot {
            Some(shot) => Self {
                monitor_x: shot.monitor_x,
                monitor_y: shot.monitor_y,
                // 模型坐标相对已缩放图；先还原到截图像素，再映射到显示器逻辑坐标。
                scale_x: f64::from(shot.monitor_width) / f64::from(shot.width.max(1)),
                scale_y: f64::from(shot.monitor_height) / f64::from(shot.height.max(1)),
            },
            None => Self {
                monitor_x: 0,
                monitor_y: 0,
                scale_x: 1.0,
                scale_y: 1.0,
            },
        }
    }

    fn to_abs(self, x: f64, y: f64) -> (i32, i32) {
        let (dx, dy) = scale_model_point(x, y, self.scale_x, self.scale_y);
        (
            self.monitor_x.saturating_add(dx),
            self.monitor_y.saturating_add(dy),
        )
    }
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

    let mut mapped = Vec::new();
    for raw in keys {
        for part in raw.split(['+', '-']) {
            let part = part.trim();
            if !part.is_empty() {
                mapped.push(map_key(part)?);
            }
        }
    }
    if mapped.is_empty() {
        return Err("按键动作需要 keys".to_string());
    }
    let last = mapped.len() - 1;
    for key in &mapped[..last] {
        enigo.key(*key, Direction::Press).map_err(map_input_error)?;
    }
    enigo
        .key(mapped[last], Direction::Click)
        .map_err(map_input_error)?;
    for key in mapped[..last].iter().rev() {
        enigo
            .key(*key, Direction::Release)
            .map_err(map_input_error)?;
    }
    Ok(())
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

fn linux_session_type() -> Option<String> {
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
                detail: "截屏需要系统设置 → 隐私与安全性 → 屏幕录制。".to_string(),
            },
            input: ComputerPermissionFlag {
                granted: Some(input),
                label: if input {
                    "已授权辅助功能".to_string()
                } else {
                    "未授权辅助功能".to_string()
                },
                detail: "键鼠注入需要系统设置 → 隐私与安全性 → 辅助功能。".to_string(),
            },
            can_open_settings: true,
            hint: "模型能看到屏幕并操作键鼠。请只在信任当前渠道与工作区时开启。".to_string(),
        }
    }
    #[cfg(target_os = "windows")]
    {
        ComputerPermissionStatus {
            platform: "windows".to_string(),
            session_type: None,
            screenshot: ComputerPermissionFlag {
                granted: None,
                label: "按显示器 DPI 映射坐标".to_string(),
                detail: "截屏通常无需额外授权；UAC 安全桌面无法注入。".to_string(),
            },
            input: ComputerPermissionFlag {
                granted: None,
                label: "键鼠注入可用".to_string(),
                detail: "遇到 UAC 或其他安全桌面时会明确失败，不会静默点偏。".to_string(),
            },
            can_open_settings: true,
            hint: "模型能看到屏幕并操作键鼠。请只在信任当前渠道与工作区时开启。".to_string(),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let session = linux_session_type();
        let wayland = session.as_deref() == Some("wayland");
        ComputerPermissionStatus {
            platform: "linux".to_string(),
            session_type: session.clone(),
            screenshot: ComputerPermissionFlag {
                granted: None,
                label: if wayland {
                    "Wayland 截屏走 portal".to_string()
                } else {
                    "X11 截屏".to_string()
                },
                detail: if wayland {
                    "Wayland 需要授权屏幕共享 portal；失败时请改用 X11。".to_string()
                } else {
                    "X11 会话可直接截取屏幕。".to_string()
                },
            },
            input: ComputerPermissionFlag {
                granted: None,
                label: if wayland {
                    "Wayland 注入需远程桌面 portal".to_string()
                } else {
                    "X11 键鼠注入".to_string()
                },
                detail: if wayland {
                    "注入失败时请授权 Remote Desktop portal，或使用 X11。".to_string()
                } else {
                    "X11 可注入键鼠；不会在失败时静默点偏。".to_string()
                },
            },
            can_open_settings: false,
            hint: "模型能看到屏幕并操作键鼠。请只在信任当前渠道与工作区时开启。".to_string(),
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
            hint: "模型能看到屏幕并操作键鼠。请只在信任当前渠道与工作区时开启。".to_string(),
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
    use crate::native::tools::dispatch::ToolCtx;
    use crate::native::tools::local::LocalWorkspace;

    #[test]
    fn parse_click_and_scale_back_to_physical() {
        let args =
            parse_computer_args(r#"{"action":"click","x":100,"y":50,"button":"left","display":0}"#)
                .expect("parse");
        assert_eq!(args.action, ComputerAction::Click);
        assert_eq!(args.x, Some(100.0));
        assert_eq!(args.y, Some(50.0));
        let (w, h) = fit_long_edge(2560, 1440, MAX_IMAGE_EDGE);
        assert_eq!((w, h), (1280, 720));
        let scale_x = 2560.0 / f64::from(w);
        let scale_y = 1440.0 / f64::from(h);
        assert_eq!(scale_model_point(100.0, 50.0, scale_x, scale_y), (200, 100));
    }

    #[test]
    fn parse_rejects_incomplete_actions() {
        assert!(parse_computer_args(r#"{"action":"click"}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"drag","path":[{"x":1,"y":1}]}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"scroll"}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"type","text":""}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"keypress","keys":[]}"#).is_err());
        assert!(parse_computer_args(r#"{"action":"screenshot"}"#).is_ok());
        assert!(parse_computer_args(r#"{"action":"wait","duration_ms":200}"#).is_ok());
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
    fn risk_summary_includes_action() {
        let summary = risk_summary(r#"{"action":"click","x":12,"y":8}"#);
        assert!(summary.contains("电脑控制"));
        assert!(summary.contains("点击"));
        assert!(summary.contains("12"));
    }
}
