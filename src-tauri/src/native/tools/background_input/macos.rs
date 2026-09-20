//! macOS：AX 树优先，坐标走 CGEventPostToPid + 窗口绑定；截窗用 CGWindowListCreateImage。

use std::ffi::{c_void, CStr, CString};
use std::ptr;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use foreign_types::ForeignType;
use core_graphics::display::{
    kCGWindowImageBoundsIgnoreFraming, kCGWindowListOptionIncludingWindow, CGWindowListCreateImage,
};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use image::RgbaImage;

use super::{window_root_element, BackgroundError, ResolvedAction};
use crate::native::tools::app_target::{
    encode_window_image, AppElement, AppTarget, WindowBounds, WindowImage, MAX_TREE_NODES,
};
use crate::native::tools::desktop::{ComputerAction, ComputerButton};

type AXUIElementRef = *mut c_void;
type AXError = i32;
type AXValueRef = *mut c_void;
const AX_OK: AXError = 0;
const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
const WINDOW_UNDER_MOUSE: u32 = 91;
const WINDOW_CAN_HANDLE_EVENT: u32 = 92;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFTypeRef) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXValueGetValue(value: AXValueRef, typ: u32, ptr: *mut c_void) -> bool;
}

extern "C" {
    fn CGEventPostToPid(pid: i32, event: core_graphics::sys::CGEventRef);
    fn CGEventSetIntegerValueField(event: core_graphics::sys::CGEventRef, field: u32, value: i64);
}

pub fn collect_tree(target: &AppTarget) -> (Vec<AppElement>, Vec<String>) {
    match walk_ax(target) {
        Ok(elements) if !elements.is_empty() => (elements, Vec::new()),
        Ok(_) => (
            vec![window_root_element(target)],
            vec!["辅助功能树为空，仅返回窗口节点".to_string()],
        ),
        Err(error) => (
            vec![window_root_element(target)],
            vec![format!("无法读取辅助功能树：{error}")],
        ),
    }
}

pub fn capture_window_image(target: &AppTarget) -> Result<WindowImage, String> {
    let rect = CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(
            f64::from(target.bounds.width),
            f64::from(target.bounds.height),
        ),
    );
    let image = unsafe {
        CGWindowListCreateImage(
            rect,
            kCGWindowListOptionIncludingWindow,
            target.window_id,
            kCGWindowImageBoundsIgnoreFraming,
        )
    };
    if image.is_null() {
        return Err("窗口在其他 Space 上或尚未绘制，CGWindowListCreateImage 没有像素".to_string());
    }
    let rgba = cgimage_to_rgba(image)?;
    encode_window_image(
        rgba,
        &format!("app-{}-{}.png", target.pid, target.window_id),
    )
}

pub fn apply_background(
    target: &AppTarget,
    resolved: &ResolvedAction,
) -> Result<(), BackgroundError> {
    if let Some(element) = resolved.element.as_ref() {
        if try_ax_action(target, element, resolved).is_ok() {
            return Ok(());
        }
    }
    match resolved.action {
        ComputerAction::SetValue => {
            if let Some(element) = resolved.element.as_ref() {
                return ax_set_value(target, element, &resolved.text);
            }
            Err(BackgroundError::unavailable(
                "后台 set_value 需要可用的辅助功能元素".to_string(),
            ))
        }
        ComputerAction::Click
        | ComputerAction::Scroll
        | ComputerAction::Drag
        | ComputerAction::TypeText
        | ComputerAction::PressKey => post_to_pid(target, resolved),
        ComputerAction::ListApps | ComputerAction::GetAppState | ComputerAction::Wait => Ok(()),
    }
}

fn walk_ax(target: &AppTarget) -> Result<Vec<AppElement>, String> {
    unsafe {
        let app = AXUIElementCreateApplication(target.pid as i32);
        if app.is_null() {
            return Err("无法创建应用辅助功能元素".to_string());
        }
        let mut elements = Vec::new();
        collect_ax(app, target, &mut elements, 0);
        CFRelease(app as CFTypeRef);
        Ok(elements)
    }
}

fn collect_ax(
    element: AXUIElementRef,
    target: &AppTarget,
    out: &mut Vec<AppElement>,
    depth: usize,
) {
    if out.len() >= MAX_TREE_NODES || depth > 8 || element.is_null() {
        return;
    }
    let role = ax_string(element, "AXRole").unwrap_or_else(|| "unknown".to_string());
    let title = ax_string(element, "AXTitle")
        .or_else(|| ax_string(element, "AXDescription"))
        .unwrap_or_default();
    let value = ax_string(element, "AXValue").unwrap_or_default();
    let bounds = ax_bounds(element, target);
    let actions = ax_actions(element);
    out.push(AppElement {
        index: out.len() as u32,
        role,
        title,
        value,
        bounds,
        actions,
    });
    if let Some(children) = ax_children(element) {
        for child in children {
            collect_ax(child, target, out, depth + 1);
            unsafe { CFRelease(child as CFTypeRef) };
        }
    }
}

fn ax_string(element: AXUIElementRef, attribute: &str) -> Option<String> {
    unsafe {
        let attr = CFString::new(attribute);
        let mut value: CFTypeRef = ptr::null();
        if AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) != AX_OK
            || value.is_null()
        {
            return None;
        }
        let text = cf_to_string(value);
        CFRelease(value);
        text
    }
}

fn ax_children(element: AXUIElementRef) -> Option<Vec<AXUIElementRef>> {
    unsafe {
        let attr = CFString::new("AXChildren");
        let mut value: CFTypeRef = ptr::null();
        if AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) != AX_OK
            || value.is_null()
        {
            return None;
        }
        let array = value as core_foundation::array::CFArrayRef;
        let count = core_foundation::array::CFArrayGetCount(array);
        let mut children = Vec::new();
        for index in 0..count {
            let child =
                core_foundation::array::CFArrayGetValueAtIndex(array, index) as AXUIElementRef;
            if !child.is_null() {
                core_foundation::base::CFRetain(child as CFTypeRef);
                children.push(child);
            }
        }
        CFRelease(value);
        Some(children)
    }
}

fn ax_bounds(element: AXUIElementRef, target: &AppTarget) -> WindowBounds {
    unsafe {
        let mut position_ref: CFTypeRef = ptr::null();
        let mut size_ref: CFTypeRef = ptr::null();
        let pos_attr = CFString::new("AXPosition");
        let size_attr = CFString::new("AXSize");
        let mut point = CGPoint::new(0.0, 0.0);
        let mut size = CGSize::new(0.0, 0.0);
        if AXUIElementCopyAttributeValue(element, pos_attr.as_concrete_TypeRef(), &mut position_ref)
            == AX_OK
            && !position_ref.is_null()
        {
            AXValueGetValue(
                position_ref as AXValueRef,
                AX_VALUE_CGPOINT,
                &mut point as *mut _ as *mut c_void,
            );
            CFRelease(position_ref);
        }
        if AXUIElementCopyAttributeValue(element, size_attr.as_concrete_TypeRef(), &mut size_ref)
            == AX_OK
            && !size_ref.is_null()
        {
            AXValueGetValue(
                size_ref as AXValueRef,
                AX_VALUE_CGSIZE,
                &mut size as *mut _ as *mut c_void,
            );
            CFRelease(size_ref);
        }
        WindowBounds {
            x: (point.x as i32).saturating_sub(target.bounds.x),
            y: (point.y as i32).saturating_sub(target.bounds.y),
            width: size.width.max(0.0) as u32,
            height: size.height.max(0.0) as u32,
        }
    }
}

fn ax_actions(element: AXUIElementRef) -> Vec<String> {
    unsafe {
        let mut names: CFTypeRef = ptr::null();
        if AXUIElementCopyActionNames(element, &mut names) != AX_OK || names.is_null() {
            return Vec::new();
        }
        let array = names as core_foundation::array::CFArrayRef;
        let count = core_foundation::array::CFArrayGetCount(array);
        let mut actions = Vec::new();
        for index in 0..count {
            let item = core_foundation::array::CFArrayGetValueAtIndex(array, index);
            if let Some(text) = cf_to_string(item) {
                actions.push(text.trim_start_matches("AX").to_ascii_lowercase());
            }
        }
        CFRelease(names);
        actions
    }
}

fn cf_to_string(value: CFTypeRef) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let string = unsafe { CFString::wrap_under_get_rule(value as CFStringRef) };
    Some(string.to_string())
}

fn ax_element_at(target: &AppTarget, index: u32) -> Result<AXUIElementRef, BackgroundError> {
    let (elements, _) = collect_tree(target);
    if elements.iter().all(|element| element.index != index) {
        return Err(BackgroundError::failed(format!(
            "辅助功能树没有元素 {index}"
        )));
    }
    unsafe {
        let app = AXUIElementCreateApplication(target.pid as i32);
        if app.is_null() {
            return Err(BackgroundError::failed(
                "无法创建应用辅助功能元素".to_string(),
            ));
        }
        let found = find_ax_index(app, index, &mut 0);
        CFRelease(app as CFTypeRef);
        found.ok_or_else(|| BackgroundError::failed(format!("无法定位元素 {index}")))
    }
}

fn find_ax_index(
    element: AXUIElementRef,
    wanted: u32,
    current: &mut u32,
) -> Option<AXUIElementRef> {
    if element.is_null() || *current > MAX_TREE_NODES as u32 {
        return None;
    }
    if *current == wanted {
        unsafe { core_foundation::base::CFRetain(element as CFTypeRef) };
        return Some(element);
    }
    *current += 1;
    if let Some(children) = ax_children(element) {
        for child in children {
            if let Some(found) = find_ax_index(child, wanted, current) {
                unsafe { CFRelease(child as CFTypeRef) };
                return Some(found);
            }
            unsafe { CFRelease(child as CFTypeRef) };
        }
    }
    None
}

fn try_ax_action(
    target: &AppTarget,
    element: &AppElement,
    resolved: &ResolvedAction,
) -> Result<(), BackgroundError> {
    match resolved.action {
        ComputerAction::Click => {
            let ax = ax_element_at(target, element.index)?;
            let result = unsafe {
                let action = CFString::new("AXPress");
                AXUIElementPerformAction(ax, action.as_concrete_TypeRef())
            };
            unsafe { CFRelease(ax as CFTypeRef) };
            if result == AX_OK {
                Ok(())
            } else {
                Err(BackgroundError::failed(format!("AXPress 失败：{result}")))
            }
        }
        ComputerAction::SetValue => ax_set_value(target, element, &resolved.text),
        _ => Err(BackgroundError::failed(
            "该动作没有对应 AX action".to_string(),
        )),
    }
}

fn ax_set_value(
    target: &AppTarget,
    element: &AppElement,
    text: &str,
) -> Result<(), BackgroundError> {
    let ax = ax_element_at(target, element.index)?;
    let value = CFString::new(text);
    let result = unsafe {
        let attr = CFString::new("AXValue");
        AXUIElementSetAttributeValue(ax, attr.as_concrete_TypeRef(), value.as_CFTypeRef())
    };
    unsafe { CFRelease(ax as CFTypeRef) };
    if result == AX_OK {
        Ok(())
    } else {
        Err(BackgroundError::failed(format!(
            "AXSetValue 失败：{result}"
        )))
    }
}

fn post_to_pid(target: &AppTarget, resolved: &ResolvedAction) -> Result<(), BackgroundError> {
    match resolved.action {
        ComputerAction::Click => {
            let (x, y) = resolved.point.unwrap_or((0, 0));
            post_mouse(target, x, y, resolved.button, true)?;
            post_mouse(target, x, y, resolved.button, false)?;
            Ok(())
        }
        ComputerAction::Scroll => {
            let (x, y) = resolved.point.unwrap_or((0, 0));
            post_scroll(target, x, y, resolved.scroll_x, resolved.scroll_y)
        }
        ComputerAction::Drag => post_drag(target, resolved),
        ComputerAction::TypeText => post_text(target, &resolved.text),
        ComputerAction::PressKey => post_keys(target, &resolved.keys),
        _ => Ok(()),
    }
}

fn post_mouse(
    target: &AppTarget,
    x: i32,
    y: i32,
    button: ComputerButton,
    press: bool,
) -> Result<(), BackgroundError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| BackgroundError::failed("无法创建 CGEventSource".to_string()))?;
    let point = CGPoint::new(
        f64::from(target.bounds.x.saturating_add(x)),
        f64::from(target.bounds.y.saturating_add(y)),
    );
    let (down, up, cg_button) = match button {
        ComputerButton::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGMouseButton::Left,
        ),
        ComputerButton::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGMouseButton::Right,
        ),
        ComputerButton::Middle => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            CGMouseButton::Center,
        ),
    };
    let event_type = if press { down } else { up };
    let event = CGEvent::new_mouse_event(source, event_type, point, cg_button)
        .map_err(|_| BackgroundError::failed("无法创建鼠标事件".to_string()))?;
    bind_window(&event, target.window_id);
    unsafe { CGEventPostToPid(target.pid as i32, event.as_ptr()) };
    let _ = CGEventTapLocation::HID;
    Ok(())
}

fn post_scroll(
    target: &AppTarget,
    x: i32,
    y: i32,
    scroll_x: i32,
    scroll_y: i32,
) -> Result<(), BackgroundError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| BackgroundError::failed("无法创建 CGEventSource".to_string()))?;
    let event = CGEvent::new_scroll_event(source, 0, 2, scroll_y, scroll_x, 0).or_else(|_| {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| BackgroundError::failed("无法创建 CGEventSource".to_string()))?;
        CGEvent::new_mouse_event(
            source,
            CGEventType::ScrollWheel,
            CGPoint::new(
                f64::from(target.bounds.x.saturating_add(x)),
                f64::from(target.bounds.y.saturating_add(y)),
            ),
            CGMouseButton::Left,
        )
        .map_err(|_| BackgroundError::failed("无法创建滚动事件".to_string()))
    })?;
    bind_window(&event, target.window_id);
    unsafe { CGEventPostToPid(target.pid as i32, event.as_ptr()) };
    Ok(())
}

fn post_drag(target: &AppTarget, resolved: &ResolvedAction) -> Result<(), BackgroundError> {
    if resolved.path.len() < 2 {
        return Err(BackgroundError::failed("拖拽需要至少两个点".to_string()));
    }
    let start = (
        resolved.path[0].0 - target.bounds.x,
        resolved.path[0].1 - target.bounds.y,
    );
    post_mouse(target, start.0, start.1, resolved.button, true)?;
    for (abs_x, abs_y) in resolved.path.iter().skip(1) {
        post_mouse(
            target,
            abs_x - target.bounds.x,
            abs_y - target.bounds.y,
            resolved.button,
            true,
        )?;
    }
    let last = resolved.path.last().copied().unwrap_or((0, 0));
    post_mouse(
        target,
        last.0 - target.bounds.x,
        last.1 - target.bounds.y,
        resolved.button,
        false,
    )
}

fn post_text(target: &AppTarget, text: &str) -> Result<(), BackgroundError> {
    for ch in text.chars() {
        post_unicode(target, ch)?;
    }
    Ok(())
}

fn post_keys(target: &AppTarget, keys: &[String]) -> Result<(), BackgroundError> {
    for key in keys {
        let mapped =
            crate::native::tools::desktop::map_key(key).map_err(BackgroundError::failed)?;
        let unicode = match mapped {
            enigo::Key::Unicode(ch) => ch,
            enigo::Key::Return => '\n',
            enigo::Key::Tab => '\t',
            enigo::Key::Space => ' ',
            _ => {
                return Err(BackgroundError::unavailable(format!(
                    "后台无法投递按键 {key}，且不会改用全局 enigo"
                )));
            }
        };
        post_unicode(target, unicode)?;
    }
    Ok(())
}

fn post_unicode(target: &AppTarget, ch: char) -> Result<(), BackgroundError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| BackgroundError::failed("无法创建 CGEventSource".to_string()))?;
    let event = CGEvent::new_keyboard_event(source, 0, true)
        .map_err(|_| BackgroundError::failed("无法创建键盘事件".to_string()))?;
    event.set_string(&ch.to_string());
    event.set_flags(CGEventFlags::CGEventFlagNonCoalesced);
    bind_window(&event, target.window_id);
    unsafe { CGEventPostToPid(target.pid as i32, event.as_ptr()) };
    Ok(())
}

fn bind_window(event: &CGEvent, window_id: u32) {
    unsafe {
        CGEventSetIntegerValueField(event.as_ptr(), WINDOW_UNDER_MOUSE, i64::from(window_id));
        CGEventSetIntegerValueField(
            event.as_ptr(),
            WINDOW_CAN_HANDLE_EVENT,
            i64::from(window_id),
        );
    }
}

fn cgimage_to_rgba(image: core_graphics::sys::CGImageRef) -> Result<RgbaImage, String> {
    use core_graphics::color_space::CGColorSpace;
    use core_graphics::context::CGContext;
    use core_graphics::image::CGImage;

    let image = unsafe { CGImage::from_ptr(image) };
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err("窗口在其他 Space 上，没有像素".to_string());
    }
    let color_space = CGColorSpace::create_device_rgb();
    let mut context = CGContext::create_bitmap_context(
        None,
        width,
        height,
        8,
        0,
        &color_space,
        core_graphics::base::kCGImageAlphaPremultipliedLast,
    );
    context.draw_image(
        CGRect::new(
            &CGPoint::new(0.0, 0.0),
            &CGSize::new(width as f64, height as f64),
        ),
        &image,
    );
    let data = context.data().to_vec();
    RgbaImage::from_raw(width as u32, height as u32, data)
        .ok_or_else(|| "无法把窗口像素转成 RGBA".to_string())
}

#[allow(dead_code)]
fn ns_event_mouse(window_id: u32, x: f64, y: f64) -> Option<()> {
    // 尽力绑定 NSEvent.windowNumber；失败时仍使用上面的 CGEvent 字段。
    let _ = (
        window_id,
        x,
        y,
        CString::new("NSEvent"),
        c"",
    );
    None
}
