//! Windows：UIA 树 + PrintWindow 截窗；后台输入走 UIA / PostMessage，不默认 SendInput。

use std::mem::size_of;

use image::RgbaImage;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationElementArray,
    IUIAutomationInvokePattern, IUIAutomationValuePattern, TreeScope_Descendants,
    UIA_InvokePatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, PostMessageW, PrintWindow, PW_RENDERFULLCONTENT, WM_CHAR,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use super::{window_root_element, BackgroundError, ResolvedAction};
use crate::native::tools::app_target::{
    encode_window_image, AppElement, AppTarget, WindowBounds, WindowImage, MAX_TREE_NODES,
};
use crate::native::tools::desktop::{ComputerAction, ComputerButton};

pub fn collect_tree(target: &AppTarget) -> (Vec<AppElement>, Vec<String>) {
    match walk_uia(target) {
        Ok(elements) if !elements.is_empty() => (elements, Vec::new()),
        Ok(_) => (
            vec![window_root_element(target)],
            vec!["UI Automation 未返回子节点，仅包含窗口".to_string()],
        ),
        Err(error) => (
            vec![window_root_element(target)],
            vec![format!("无法读取 UI Automation 树：{error}")],
        ),
    }
}

pub fn capture_window_image(target: &AppTarget) -> Result<WindowImage, String> {
    let hwnd = HWND(target.window_id as isize);
    let rect = extended_bounds(hwnd).map_err(|error| error.to_string())?;
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);
    unsafe {
        let window_dc = GetDC(hwnd);
        if window_dc.is_invalid() {
            return Err("PrintWindow 无法取得窗口 DC".to_string());
        }
        let mem_dc = CreateCompatibleDC(window_dc);
        let bitmap = CreateCompatibleBitmap(window_dc, width, height);
        let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));
        let printed = PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buffer = vec![0u8; (width * height * 4) as usize];
        let copied = GetDIBits(
            mem_dc,
            bitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(mem_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(hwnd, window_dc);
        if !printed.as_bool() || copied == 0 {
            return Err("PrintWindow 未能取得窗口像素".to_string());
        }
        // BGRA -> RGBA
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let rgba = RgbaImage::from_raw(width as u32, height as u32, buffer)
            .ok_or_else(|| "无法把 PrintWindow 像素转成 RGBA".to_string())?;
        encode_window_image(
            rgba,
            &format!("app-{}-{}.png", target.pid, target.window_id),
        )
    }
}

pub fn apply_background(
    target: &AppTarget,
    resolved: &ResolvedAction,
) -> Result<(), BackgroundError> {
    if would_drop_post_message(target) {
        return Err(BackgroundError::unavailable(
            "该窗口会丢弃后台 PostMessage（Chromium / UWP）。请改用 dispatch=foreground（会移动用户光标）"
                .to_string(),
        ));
    }
    if let Some(element) = resolved.element.as_ref() {
        if try_uia_action(target, element, resolved).is_ok() {
            return Ok(());
        }
    }
    match resolved.action {
        ComputerAction::Click | ComputerAction::Scroll | ComputerAction::Drag => {
            post_pointer(target, resolved)
        }
        ComputerAction::TypeText => post_chars(target, &resolved.text),
        ComputerAction::SetValue => {
            if let Some(element) = resolved.element.as_ref() {
                return try_uia_action(target, element, resolved);
            }
            Err(BackgroundError::unavailable(
                "后台 set_value 需要 UI Automation 元素".to_string(),
            ))
        }
        ComputerAction::PressKey => post_keys(target, &resolved.keys),
        ComputerAction::ListApps | ComputerAction::GetAppState | ComputerAction::Wait => Ok(()),
    }
}

fn would_drop_post_message(target: &AppTarget) -> bool {
    let class = window_class(HWND(target.window_id as isize)).unwrap_or_default();
    let hay = format!(
        "{} {} {}",
        class,
        target.identifier,
        target.class_name.clone().unwrap_or_default()
    )
    .to_ascii_lowercase();
    [
        "chrome_widgetwin",
        "windows.ui.core.corewindow",
        "applicationframewindow",
    ]
    .iter()
    .any(|needle| hay.contains(needle))
}

fn window_class(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if len <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

fn extended_bounds(hwnd: HWND) -> Result<RECT, BackgroundError> {
    unsafe {
        let mut rect = RECT::default();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
        .is_ok()
        {
            return Ok(rect);
        }
        GetWindowRect(hwnd, &mut rect)
            .map_err(|error| BackgroundError::failed(format!("无法读取窗口边框：{error}")))?;
        Ok(rect)
    }
}

fn walk_uia(target: &AppTarget) -> Result<Vec<AppElement>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("无法创建 UI Automation：{error}"))?;
        let root = automation
            .ElementFromHandle(HWND(target.window_id as isize))
            .map_err(|error| format!("无法取得窗口 UIA 元素：{error}"))?;
        let mut elements = vec![uia_to_element(&root, 0, target)?];
        if let Ok(array) = root.FindAll(TreeScope_Descendants, automation.CreateTrueCondition()?) {
            let count = array.Length().unwrap_or(0);
            for index in 0..count {
                if elements.len() >= MAX_TREE_NODES {
                    break;
                }
                if let Ok(child) = array.GetElement(index) {
                    elements.push(uia_to_element(&child, elements.len() as u32, target)?);
                }
            }
            let _ = array;
        }
        Ok(elements)
    }
}

fn uia_to_element(
    element: &IUIAutomationElement,
    index: u32,
    target: &AppTarget,
) -> Result<AppElement, String> {
    unsafe {
        let name = element
            .CurrentName()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let role = element
            .CurrentLocalizedControlType()
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let value = element
            .GetCurrentPropertyValue(UIA_ValuePatternId.0 as i32)
            .ok()
            .and_then(|item| item.to_string())
            .unwrap_or_default();
        let rect = element.CurrentBoundingRectangle().unwrap_or_default();
        let mut actions = Vec::new();
        if element.GetCurrentPattern(UIA_InvokePatternId).is_ok() {
            actions.push("invoke".to_string());
        }
        if element.GetCurrentPattern(UIA_ValuePatternId).is_ok() {
            actions.push("set_value".to_string());
        }
        Ok(AppElement {
            index,
            role,
            title: name,
            value,
            bounds: WindowBounds {
                x: rect.left.saturating_sub(target.bounds.x),
                y: rect.top.saturating_sub(target.bounds.y),
                width: (rect.right - rect.left).max(0) as u32,
                height: (rect.bottom - rect.top).max(0) as u32,
            },
            actions,
        })
    }
}

fn try_uia_action(
    target: &AppTarget,
    element: &AppElement,
    resolved: &ResolvedAction,
) -> Result<(), BackgroundError> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|error| {
                BackgroundError::failed(format!("无法创建 UI Automation：{error}"))
            })?;
        let root = automation
            .ElementFromHandle(HWND(target.window_id as isize))
            .map_err(|error| BackgroundError::failed(error.to_string()))?;
        let found = uia_element_at(&automation, &root, element.index)
            .ok_or_else(|| BackgroundError::failed(format!("找不到 UIA 元素 {}", element.index)))?;
        match resolved.action {
            ComputerAction::Click => {
                let pattern: IUIAutomationInvokePattern = found
                    .GetCurrentPattern(UIA_InvokePatternId)
                    .and_then(|unknown| unknown.cast())
                    .map_err(|error| {
                        BackgroundError::failed(format!("元素不支持 Invoke：{error}"))
                    })?;
                pattern
                    .Invoke()
                    .map_err(|error| BackgroundError::failed(format!("UIA Invoke 失败：{error}")))
            }
            ComputerAction::SetValue | ComputerAction::TypeText => {
                let pattern: IUIAutomationValuePattern = found
                    .GetCurrentPattern(UIA_ValuePatternId)
                    .and_then(|unknown| unknown.cast())
                    .map_err(|error| {
                        BackgroundError::failed(format!("元素不支持 Value：{error}"))
                    })?;
                pattern
                    .SetValue(&windows::core::BSTR::from(resolved.text.as_str()))
                    .map_err(|error| BackgroundError::failed(format!("UIA SetValue 失败：{error}")))
            }
            _ => Err(BackgroundError::failed(
                "该动作没有对应 UIA pattern".to_string(),
            )),
        }
    }
}

fn uia_element_at(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    index: u32,
) -> Option<IUIAutomationElement> {
    if index == 0 {
        return Some(root.clone());
    }
    unsafe {
        let array: IUIAutomationElementArray = root
            .FindAll(
                TreeScope_Descendants,
                automation.CreateTrueCondition().ok()?,
            )
            .ok()?;
        array.GetElement((index as i32) - 1).ok()
    }
}

fn post_pointer(target: &AppTarget, resolved: &ResolvedAction) -> Result<(), BackgroundError> {
    let hwnd = HWND(target.window_id as isize);
    match resolved.action {
        ComputerAction::Click => {
            let (x, y) = resolved.point.unwrap_or((0, 0));
            let (down, up) = button_messages(resolved.button);
            post(hwnd, down, 0, lparam(x, y))?;
            post(hwnd, up, 0, lparam(x, y))?;
        }
        ComputerAction::Scroll => {
            let (x, y) = resolved.point.unwrap_or((0, 0));
            let delta = if resolved.scroll_y != 0 {
                -resolved.scroll_y.saturating_mul(120)
            } else {
                resolved.scroll_x.saturating_mul(120)
            };
            post(
                hwnd,
                WM_MOUSEWHEEL,
                WPARAM((delta as u16 as u32) << 16),
                lparam(x, y),
            )?;
        }
        ComputerAction::Drag => {
            if resolved.path.len() < 2 {
                return Err(BackgroundError::failed("拖拽需要至少两个点".to_string()));
            }
            let start = (
                resolved.path[0].0 - target.bounds.x,
                resolved.path[0].1 - target.bounds.y,
            );
            let (down, up) = button_messages(resolved.button);
            post(hwnd, down, 0, lparam(start.0, start.1))?;
            for (abs_x, abs_y) in resolved.path.iter().skip(1) {
                post(
                    hwnd,
                    WM_LBUTTONDOWN,
                    0,
                    lparam(abs_x - target.bounds.x, abs_y - target.bounds.y),
                )?;
            }
            let last = resolved.path.last().copied().unwrap_or((0, 0));
            post(
                hwnd,
                up,
                0,
                lparam(last.0 - target.bounds.x, last.1 - target.bounds.y),
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn post_chars(target: &AppTarget, text: &str) -> Result<(), BackgroundError> {
    let hwnd = HWND(target.window_id as isize);
    for ch in text.encode_utf16() {
        post(hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0))?;
    }
    Ok(())
}

fn post_keys(target: &AppTarget, keys: &[String]) -> Result<(), BackgroundError> {
    let hwnd = HWND(target.window_id as isize);
    for key in keys {
        let vk = virtual_key(key).ok_or_else(|| {
            BackgroundError::unavailable(format!("后台无法投递按键 {key}，且不会改用 SendInput"))
        })?;
        post(hwnd, WM_KEYDOWN, WPARAM(vk as usize), LPARAM(0))?;
        post(hwnd, WM_KEYUP, WPARAM(vk as usize), LPARAM(0))?;
    }
    Ok(())
}

fn virtual_key(name: &str) -> Option<u16> {
    match name.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(0x0D),
        "esc" | "escape" => Some(0x1B),
        "tab" => Some(0x09),
        "space" => Some(0x20),
        "backspace" => Some(0x08),
        "delete" | "del" => Some(0x2E),
        "ctrl" | "control" => Some(0x11),
        "alt" => Some(0x12),
        "shift" => Some(0x10),
        other if other.chars().count() == 1 => other
            .chars()
            .next()
            .map(|ch| ch.to_ascii_uppercase() as u16),
        _ => None,
    }
}

fn button_messages(button: ComputerButton) -> (u32, u32) {
    match button {
        ComputerButton::Left => (WM_LBUTTONDOWN, WM_LBUTTONUP),
        ComputerButton::Right => (WM_RBUTTONDOWN, WM_RBUTTONUP),
        ComputerButton::Middle => (WM_MBUTTONDOWN, WM_MBUTTONUP),
    }
}

fn lparam(x: i32, y: i32) -> LPARAM {
    LPARAM(((y as u16 as u32) << 16 | (x as u16 as u32)) as isize)
}

fn post(
    hwnd: HWND,
    msg: u32,
    wparam: impl Into<WPARAM>,
    lparam: LPARAM,
) -> Result<(), BackgroundError> {
    let posted = unsafe { PostMessageW(hwnd, msg, wparam.into(), lparam) };
    posted.map_err(|error| BackgroundError::unavailable(format!("PostMessage 被丢弃：{error}")))
}

#[allow(dead_code)]
fn unused_pcwstr() -> PCWSTR {
    PCWSTR::null()
}

#[allow(dead_code)]
fn unused_bool() -> BOOL {
    BOOL(1)
}
