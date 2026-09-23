//! 电脑控制的应用目标：枚举、解析、按窗口截图与状态快照。

use std::collections::BTreeMap;
use std::io::Cursor;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::{DynamicImage, ImageFormat};

use crate::native::model::types::NativeImage;

/// 送给模型的截图最长边（像素）。
pub const MAX_IMAGE_EDGE: u32 = 1280;
/// 送给模型的无障碍树节点上限。
pub const MAX_TREE_NODES: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowBounds {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width as i32)
            && y < self.y.saturating_add(self.height as i32)
    }

    pub fn relative_to(self, origin: WindowBounds) -> WindowBounds {
        Self {
            x: self.x.saturating_sub(origin.x),
            y: self.y.saturating_sub(origin.y),
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    pub name: String,
    pub identifier: String,
    pub pid: u32,
    pub window_count: u32,
    pub focused: bool,
}

impl AppInfo {
    pub fn matches_query(&self, query: &str) -> bool {
        identity_matches(query, &self.name) || identity_matches(query, &self.identifier)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppTarget {
    pub name: String,
    pub identifier: String,
    pub pid: u32,
    pub window_id: u32,
    pub window_title: String,
    pub bounds: WindowBounds,
    pub class_name: Option<String>,
    pub focused: bool,
}

impl AppTarget {
    pub fn matches_query(&self, query: &str) -> bool {
        identity_matches(query, &self.name)
            || identity_matches(query, &self.identifier)
            || identity_matches(query, &self.window_title)
    }

    pub fn identity_candidates(&self) -> Vec<String> {
        let mut values = vec![self.identifier.clone(), self.name.clone()];
        if let Some(class_name) = &self.class_name {
            if !class_name.is_empty() {
                values.push(class_name.clone());
            }
        }
        values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppElement {
    pub index: u32,
    pub role: String,
    pub title: String,
    pub value: String,
    pub bounds: WindowBounds,
    pub actions: Vec<String>,
}

impl AppElement {
    pub fn summary_line(&self) -> String {
        let actions = if self.actions.is_empty() {
            "-".to_string()
        } else {
            self.actions.join(",")
        };
        format!(
            "[{}] {} \"{}\" value=\"{}\" bounds=({},{},{},{}) actions={}",
            self.index,
            self.role,
            truncate_label(&self.title, 48),
            truncate_label(&self.value, 32),
            self.bounds.x,
            self.bounds.y,
            self.bounds.width,
            self.bounds.height,
            actions
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerAppState {
    pub target: AppTarget,
    pub elements: Vec<AppElement>,
    pub width: u32,
    pub height: u32,
    pub scale_x: f64,
    pub scale_y: f64,
    pub notes: Vec<String>,
    pub image: Option<WindowImage>,
}

impl ComputerAppState {
    pub fn element(&self, index: u32) -> Result<&AppElement, String> {
        self.elements
            .iter()
            .find(|element| element.index == index)
            .ok_or_else(|| format!("元素下标 {index} 无效，请先调用 get_app_state 刷新本轮树"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowImage {
    pub image: NativeImage,
    pub width: u32,
    pub height: u32,
    pub scale_x: f64,
    pub scale_y: f64,
    pub capture_width: u32,
    pub capture_height: u32,
}

pub fn identity_matches(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.trim();
    let candidate = candidate.trim();
    if pattern.is_empty() || candidate.is_empty() {
        return false;
    }
    identity_tokens(pattern).iter().any(|left| {
        identity_tokens(candidate)
            .iter()
            .any(|right| left.eq_ignore_ascii_case(right))
    })
}

pub fn identity_tokens(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }
    let mut tokens = vec![value.to_string()];
    let base = value.rsplit(['/', '\\']).next().unwrap_or(value).trim();
    if !base.is_empty() && base != value {
        tokens.push(base.to_string());
    }
    let lower = base.to_ascii_lowercase();
    for ext in [".exe", ".app", ".bin"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            tokens.push(stem.to_string());
        }
    }
    if looks_like_bundle_id(base) {
        if let Some(last) = base.rsplit('.').next() {
            if !last.is_empty() && last != base {
                tokens.push(last.to_string());
            }
        }
    }
    tokens
}

fn looks_like_bundle_id(value: &str) -> bool {
    !value.contains(['/', '\\'])
        && value.contains('.')
        && !value.to_ascii_lowercase().ends_with(".exe")
        && !value.to_ascii_lowercase().ends_with(".app")
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let taken: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{taken}…")
    } else {
        taken
    }
}

pub fn list_apps() -> Result<Vec<AppInfo>, String> {
    let windows = visible_windows()?;
    let mut grouped: BTreeMap<(u32, String), AppInfo> = BTreeMap::new();
    for window in windows {
        let key = (window.pid, window.identifier.clone());
        grouped
            .entry(key)
            .and_modify(|info| {
                info.window_count += 1;
                info.focused |= window.focused;
                if info.name.is_empty() {
                    info.name = window.name.clone();
                }
            })
            .or_insert(AppInfo {
                name: window.name,
                identifier: window.identifier,
                pid: window.pid,
                window_count: 1,
                focused: window.focused,
            });
    }
    let mut apps: Vec<AppInfo> = grouped.into_values().collect();
    apps.sort_by(|left, right| {
        right.focused.cmp(&left.focused).then(
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase()),
        )
    });
    Ok(apps)
}

pub fn resolve_app(query: &str) -> Result<AppTarget, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("请指定 app（名称、bundle id 或可执行文件路径）".to_string());
    }
    let windows = visible_windows()?;
    let matches: Vec<AppTarget> = windows
        .into_iter()
        .filter(|window| window.matches_query(query))
        .collect();
    if matches.is_empty() {
        return Err(format!("未找到应用：{query}"));
    }
    if let Some(exact) = matches.iter().find(|window| {
        window.identifier.eq_ignore_ascii_case(query) || window.name.eq_ignore_ascii_case(query)
    }) {
        return Ok(pick_preferred_window(
            matches
                .iter()
                .filter(|window| window.pid == exact.pid && window.identifier == exact.identifier)
                .cloned()
                .collect(),
        ));
    }
    let identifiers: Vec<String> = matches
        .iter()
        .map(|window| window.identifier.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if identifiers.len() > 1 {
        return Err(format!(
            "应用 {query} 不唯一（{} 个匹配：{}），请使用 bundle id 或可执行文件路径",
            identifiers.len(),
            identifiers.join(" / ")
        ));
    }
    Ok(pick_preferred_window(matches))
}

fn pick_preferred_window(mut windows: Vec<AppTarget>) -> AppTarget {
    windows.sort_by(|left, right| {
        right.focused.cmp(&left.focused).then_with(|| {
            (right.bounds.width.saturating_mul(right.bounds.height))
                .cmp(&(left.bounds.width.saturating_mul(left.bounds.height)))
        })
    });
    windows
        .into_iter()
        .next()
        .expect("pick_preferred_window 需要非空窗口列表")
}

pub fn capture_window(target: &AppTarget) -> Result<WindowImage, String> {
    let window = find_xcap_window(target)?;
    let rgba = window
        .capture_image()
        .map_err(|error| format!("无法截取应用窗口 {}：{error}", target.name))?;
    encode_window_image(
        rgba,
        &format!("app-{}-{}.png", target.pid, target.window_id),
    )
}

pub fn encode_window_image(rgba: image::RgbaImage, name: &str) -> Result<WindowImage, String> {
    let capture_width = rgba.width();
    let capture_height = rgba.height();
    if capture_width == 0 || capture_height == 0 {
        return Err("窗口截图为空（窗口可能在其他 Space / 工作区，或尚未绘制）".to_string());
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
        .map_err(|error| format!("编码窗口截图失败: {error}"))?;
    Ok(WindowImage {
        image: NativeImage {
            name: name.to_string(),
            mime_type: "image/png".to_string(),
            data_base64: BASE64.encode(png.into_inner()),
            attachment_id: String::new(),
            page: None,
            time_range: None,
        },
        width,
        height,
        scale_x: f64::from(capture_width) / f64::from(width.max(1)),
        scale_y: f64::from(capture_height) / f64::from(height.max(1)),
        capture_width,
        capture_height,
    })
}

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

pub fn scale_model_point(x: f64, y: f64, scale_x: f64, scale_y: f64) -> (i32, i32) {
    ((x * scale_x).round() as i32, (y * scale_y).round() as i32)
}

pub fn format_app_list(apps: &[AppInfo]) -> String {
    if apps.is_empty() {
        return "未找到可控制的已打开应用".to_string();
    }
    let mut lines = Vec::new();
    for app in apps {
        let focus = if app.focused { " focused" } else { "" };
        lines.push(format!(
            "- {}  {}  pid={}  windows={}{focus}",
            app.name, app.identifier, app.pid, app.window_count
        ));
    }
    lines.join("\n")
}

pub fn format_app_tree(state: &ComputerAppState) -> String {
    let mut lines = vec![format!(
        "app={} pid={} window={} title=\"{}\" width={} height={} scale_x={:.4} scale_y={:.4}",
        state.target.identifier,
        state.target.pid,
        state.target.window_id,
        truncate_label(&state.target.window_title, 64),
        state.width,
        state.height,
        state.scale_x,
        state.scale_y
    )];
    for note in &state.notes {
        lines.push(format!("note: {note}"));
    }
    for element in &state.elements {
        lines.push(element.summary_line());
    }
    lines.join("\n")
}

fn visible_windows() -> Result<Vec<AppTarget>, String> {
    let windows = xcap::Window::all().map_err(|error| format!("无法枚举本机窗口：{error}"))?;
    let mut targets = Vec::new();
    for window in windows {
        if window.is_minimized().unwrap_or(false) {
            continue;
        }
        let width = window.width().unwrap_or(0);
        let height = window.height().unwrap_or(0);
        if width == 0 || height == 0 {
            continue;
        }
        let pid = window.pid().unwrap_or(0);
        if pid == 0 {
            continue;
        }
        let app_name = window.app_name().unwrap_or_default();
        let title = window.title().unwrap_or_default();
        if app_name.trim().is_empty() && title.trim().is_empty() {
            continue;
        }
        let identifier = process_identifier(pid).unwrap_or_else(|| {
            if app_name.trim().is_empty() {
                format!("pid:{pid}")
            } else {
                app_name.clone()
            }
        });
        let name = if app_name.trim().is_empty() {
            identifier_display_name(&identifier)
        } else {
            app_name
        };
        targets.push(AppTarget {
            name,
            identifier,
            pid,
            window_id: window.id().unwrap_or(0),
            window_title: title,
            bounds: WindowBounds {
                x: window.x().unwrap_or(0),
                y: window.y().unwrap_or(0),
                width,
                height,
            },
            class_name: None,
            focused: window.is_focused().unwrap_or(false),
        });
    }
    Ok(targets)
}

fn find_xcap_window(target: &AppTarget) -> Result<xcap::Window, String> {
    let windows = xcap::Window::all().map_err(|error| format!("无法枚举本机窗口：{error}"))?;
    windows
        .into_iter()
        .find(|window| {
            window.id().ok() == Some(target.window_id) || (window.pid().ok() == Some(target.pid))
        })
        .ok_or_else(|| format!("应用窗口已消失：{}", target.name))
}

fn identifier_display_name(identifier: &str) -> String {
    identity_tokens(identifier)
        .into_iter()
        .next_back()
        .unwrap_or_else(|| identifier.to_string())
}

fn process_identifier(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    }
    #[cfg(target_os = "macos")]
    {
        macos_process_identifier(pid)
    }
    #[cfg(target_os = "windows")]
    {
        windows_process_identifier(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_process_identifier(pid: u32) -> Option<String> {
    let mut buffer = vec![0u8; 4096];
    let len = unsafe { proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if len <= 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buffer[..len as usize]).into_owned())
}

#[cfg(target_os = "macos")]
extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, buffersize: u32) -> i32;
}

#[cfg(target_os = "windows")]
fn windows_process_identifier(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 512];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_tokens_cover_bundle_and_exe() {
        let safari = identity_tokens("com.apple.Safari");
        assert!(safari.iter().any(|item| item == "Safari"));
        let firefox = identity_tokens("/usr/bin/firefox");
        assert!(firefox.iter().any(|item| item == "firefox"));
        let exe = identity_tokens(r"C:\Program Files\App\code.exe");
        assert!(exe.iter().any(|item| item == "code"));
    }

    #[test]
    fn identity_matches_display_name_and_bundle() {
        assert!(identity_matches("Safari", "com.apple.Safari"));
        assert!(identity_matches("com.apple.Safari", "Safari"));
        assert!(identity_matches("firefox", "/usr/bin/firefox"));
        assert!(!identity_matches("Safari", "Notes"));
    }

    #[test]
    fn formats_empty_app_list() {
        assert!(format_app_list(&[]).contains("未找到"));
    }
}
