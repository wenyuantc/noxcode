//! Linux：AT-SPI 树 + 按窗口截图；后台输入走 AT-SPI / XSendEvent，不用全局 XTEST。

use std::time::Duration;

use dbus::arg::{RefArg, Variant};
use dbus::blocking::{Connection, Proxy};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    ButtonPressEvent, ButtonReleaseEvent, ConnectionExt, EventMask, KeyButMask, Motion,
    MotionNotifyEvent, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, MOTION_NOTIFY_EVENT,
};

use super::{window_root_element, BackgroundError, ResolvedAction};
use crate::native::tools::app_target::{AppElement, AppTarget, WindowBounds, MAX_TREE_NODES};
use crate::native::tools::desktop::{ComputerAction, ComputerButton};

const AT_SPI_TIMEOUT: Duration = Duration::from_millis(400);

pub fn collect_tree(target: &AppTarget) -> (Vec<AppElement>, Vec<String>) {
    match walk_atspi(target) {
        Ok(elements) if !elements.is_empty() => (elements, Vec::new()),
        Ok(_) => (
            vec![window_root_element(target)],
            vec!["AT-SPI 未返回子节点，仅包含窗口".to_string()],
        ),
        Err(error) => (
            vec![window_root_element(target)],
            vec![format!("AT-SPI 不可用，仅返回窗口节点：{error}")],
        ),
    }
}

pub fn apply_background(
    target: &AppTarget,
    resolved: &ResolvedAction,
) -> Result<(), BackgroundError> {
    if super::linux_session_type().as_deref() == Some("wayland")
        && matches!(
            resolved.action,
            ComputerAction::Click | ComputerAction::Scroll | ComputerAction::Drag
        )
        && resolved.element.is_none()
    {
        if try_atspi_action(target, resolved).is_ok() {
            return Ok(());
        }
        return Err(BackgroundError::unavailable(
            "Wayland 无法按窗口投递坐标事件，且不会改用全局 XTEST / enigo".to_string(),
        ));
    }
    if try_atspi_action(target, resolved).is_ok() {
        return Ok(());
    }
    match resolved.action {
        ComputerAction::Click | ComputerAction::Scroll | ComputerAction::Drag => {
            send_x11_pointer(target, resolved)
        }
        ComputerAction::SetValue | ComputerAction::TypeText | ComputerAction::PressKey => {
            Err(BackgroundError::unavailable(
                "后台键盘 / 设值需要 AT-SPI，当前无法投递，且不会改用全局 enigo".to_string(),
            ))
        }
        ComputerAction::ListApps | ComputerAction::GetAppState | ComputerAction::Wait => Ok(()),
    }
}

fn try_atspi_action(target: &AppTarget, resolved: &ResolvedAction) -> Result<(), BackgroundError> {
    let Some(element) = resolved.element.as_ref() else {
        return Err(BackgroundError::failed(
            "AT-SPI 动作需要 element_index".to_string(),
        ));
    };
    let (conn, dest, path) = atspi_node_for(target, element.index)?;
    match resolved.action {
        ComputerAction::Click => {
            do_atspi_action(&conn, &dest, &path, &["click", "press", "activate"])
        }
        ComputerAction::SetValue | ComputerAction::TypeText => {
            set_atspi_text(&conn, &dest, &path, &resolved.text)
        }
        ComputerAction::PressKey => Err(BackgroundError::failed(
            "AT-SPI 不直接支持组合键".to_string(),
        )),
        ComputerAction::Scroll => {
            do_atspi_action(&conn, &dest, &path, &["scroll", "increment", "decrement"])
        }
        ComputerAction::Drag => Err(BackgroundError::failed("AT-SPI 不支持拖拽".to_string())),
        _ => Ok(()),
    }
}

fn walk_atspi(target: &AppTarget) -> Result<Vec<AppElement>, String> {
    let (conn, dest, path) = open_application(target)?;
    let mut elements = Vec::new();
    walk_node(&conn, &dest, &path, target, &mut elements, 0)?;
    Ok(elements)
}

fn open_application(target: &AppTarget) -> Result<(Connection, String, String), String> {
    let session =
        Connection::new_session().map_err(|error| format!("无法连接会话总线：{error}"))?;
    let proxy = session.with_proxy("org.a11y.Bus", "/org/a11y/bus", AT_SPI_TIMEOUT);
    let (address,): (String,) = proxy
        .method_call("org.a11y.Bus", "GetAddress", ())
        .map_err(|error| format!("无法获取 AT-SPI 总线：{error}"))?;
    let conn = Connection::new_address(&address)
        .map_err(|error| format!("无法连接 AT-SPI 总线：{error}"))?;
    let root = conn.with_proxy(
        "org.a11y.atspi.Registry",
        "/org/a11y/atspi/accessible/root",
        AT_SPI_TIMEOUT,
    );
    let children = get_children(&root)?;
    for (dest, path) in children {
        if application_matches(&conn, &dest, &path, target) {
            return Ok((conn, dest, path));
        }
    }
    Err(format!("AT-SPI 未注册应用 {}", target.name))
}

fn application_matches(conn: &Connection, dest: &str, path: &str, target: &AppTarget) -> bool {
    let proxy = conn.with_proxy(dest, path, AT_SPI_TIMEOUT);
    let name = accessible_name(&proxy).unwrap_or_default();
    let attrs = accessible_attributes(&proxy);
    crate::native::tools::app_target::identity_matches(&target.name, &name)
        || crate::native::tools::app_target::identity_matches(&target.identifier, &name)
        || attrs.iter().any(|value| {
            value.parse::<u32>().ok() == Some(target.pid)
                || crate::native::tools::app_target::identity_matches(&target.identifier, value)
                || crate::native::tools::app_target::identity_matches(&target.name, value)
        })
}

fn walk_node(
    conn: &Connection,
    dest: &str,
    path: &str,
    target: &AppTarget,
    elements: &mut Vec<AppElement>,
    depth: usize,
) -> Result<(), String> {
    if elements.len() >= MAX_TREE_NODES || depth > 8 {
        return Ok(());
    }
    let proxy = conn.with_proxy(dest, path, AT_SPI_TIMEOUT);
    let role = accessible_role(&proxy).unwrap_or_else(|_| "unknown".to_string());
    let title = accessible_name(&proxy).unwrap_or_default();
    let value = accessible_text(&proxy).unwrap_or_default();
    let bounds = accessible_bounds(&proxy, target).unwrap_or(WindowBounds {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    });
    let actions = accessible_actions(&proxy);
    elements.push(AppElement {
        index: elements.len() as u32,
        role,
        title,
        value,
        bounds,
        actions,
    });
    for (child_dest, child_path) in get_children(&proxy).unwrap_or_default() {
        walk_node(conn, &child_dest, &child_path, target, elements, depth + 1)?;
    }
    Ok(())
}

fn get_children(proxy: &Proxy<'_, &Connection>) -> Result<Vec<(String, String)>, String> {
    let (children,): (Vec<(String, dbus::Path<'static>)>,) = proxy
        .method_call("org.a11y.atspi.Accessible", "GetChildren", ())
        .map_err(|error| format!("GetChildren 失败：{error}"))?;
    Ok(children
        .into_iter()
        .map(|(dest, path)| (dest, path.to_string()))
        .collect())
}

fn accessible_name(proxy: &Proxy<'_, &Connection>) -> Result<String, String> {
    let (value,): (Variant<Box<dyn RefArg>>,) = proxy
        .method_call(
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.a11y.atspi.Accessible", "Name"),
        )
        .map_err(|error| error.to_string())?;
    Ok(value.0.as_str().unwrap_or_default().to_string())
}

fn accessible_role(proxy: &Proxy<'_, &Connection>) -> Result<String, String> {
    let (role,): (String,) = proxy
        .method_call("org.a11y.atspi.Accessible", "GetRoleName", ())
        .map_err(|error| error.to_string())?;
    Ok(role)
}

fn accessible_text(proxy: &Proxy<'_, &Connection>) -> Result<String, String> {
    if let Ok((text,)) =
        proxy.method_call::<(String,), _, _, _>("org.a11y.atspi.Text", "GetText", (0_i32, -1_i32))
    {
        return Ok(text);
    }
    let (value,): (Variant<Box<dyn RefArg>>,) = proxy
        .method_call(
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.a11y.atspi.Value", "CurrentValue"),
        )
        .map_err(|error| error.to_string())?;
    Ok(value
        .0
        .as_f64()
        .map(|item| item.to_string())
        .unwrap_or_default())
}

fn accessible_bounds(
    proxy: &Proxy<'_, &Connection>,
    target: &AppTarget,
) -> Result<WindowBounds, String> {
    let (x, y, width, height): (i32, i32, i32, i32) = proxy
        .method_call("org.a11y.atspi.Component", "GetExtents", (0_u32,))
        .map_err(|error| error.to_string())?;
    Ok(WindowBounds {
        x: x.saturating_sub(target.bounds.x),
        y: y.saturating_sub(target.bounds.y),
        width: width.max(0) as u32,
        height: height.max(0) as u32,
    })
}

fn accessible_actions(proxy: &Proxy<'_, &Connection>) -> Vec<String> {
    let Ok((count,)) =
        proxy.method_call::<(i32,), _, _, _>("org.a11y.atspi.Action", "GetNActions", ())
    else {
        return Vec::new();
    };
    (0..count.max(0))
        .filter_map(|index| {
            proxy
                .method_call("org.a11y.atspi.Action", "GetName", (index,))
                .ok()
                .map(|(name,): (String,)| name)
        })
        .collect()
}

fn accessible_attributes(proxy: &Proxy<'_, &Connection>) -> Vec<String> {
    let Ok((attrs,)) = proxy.method_call::<(std::collections::HashMap<String, String>,), _, _, _>(
        "org.a11y.atspi.Accessible",
        "GetAttributes",
        (),
    ) else {
        return Vec::new();
    };
    attrs
        .into_iter()
        .flat_map(|(key, value)| [key, value])
        .collect()
}

fn atspi_node_for(
    target: &AppTarget,
    index: u32,
) -> Result<(Connection, String, String), BackgroundError> {
    let (conn, dest, path) = open_application(target).map_err(BackgroundError::failed)?;
    let mut elements = Vec::new();
    let mut paths = Vec::new();
    collect_paths(&conn, &dest, &path, &mut elements, &mut paths, 0)
        .map_err(BackgroundError::failed)?;
    paths
        .into_iter()
        .find(|(item_index, _, _)| *item_index == index)
        .map(|(_, dest, path)| (conn, dest, path))
        .ok_or_else(|| BackgroundError::failed(format!("AT-SPI 找不到元素 {index}")))
}

fn collect_paths(
    conn: &Connection,
    dest: &str,
    path: &str,
    elements: &mut Vec<u32>,
    paths: &mut Vec<(u32, String, String)>,
    depth: usize,
) -> Result<(), String> {
    if elements.len() >= MAX_TREE_NODES || depth > 8 {
        return Ok(());
    }
    let index = elements.len() as u32;
    elements.push(index);
    paths.push((index, dest.to_string(), path.to_string()));
    let proxy = conn.with_proxy(dest, path, AT_SPI_TIMEOUT);
    for (child_dest, child_path) in get_children(&proxy).unwrap_or_default() {
        collect_paths(conn, &child_dest, &child_path, elements, paths, depth + 1)?;
    }
    Ok(())
}

fn do_atspi_action(
    conn: &Connection,
    dest: &str,
    path: &str,
    wanted: &[&str],
) -> Result<(), BackgroundError> {
    let proxy = conn.with_proxy(dest, path, AT_SPI_TIMEOUT);
    let actions = accessible_actions(&proxy);
    let Some(index) = actions.iter().position(|action| {
        let lower = action.to_ascii_lowercase();
        wanted.iter().any(|item| lower.contains(item))
    }) else {
        return Err(BackgroundError::failed(format!(
            "元素没有可用动作（{}）",
            actions.join(",")
        )));
    };
    let _: () = proxy
        .method_call("org.a11y.atspi.Action", "DoAction", (index as i32,))
        .map_err(|error| BackgroundError::failed(format!("AT-SPI DoAction 失败：{error}")))?;
    Ok(())
}

fn set_atspi_text(
    conn: &Connection,
    dest: &str,
    path: &str,
    text: &str,
) -> Result<(), BackgroundError> {
    let proxy = conn.with_proxy(dest, path, AT_SPI_TIMEOUT);
    if proxy
        .method_call::<(), _, _, _>("org.a11y.atspi.EditableText", "SetTextContents", (text,))
        .is_ok()
    {
        return Ok(());
    }
    let value = Variant(text.to_string());
    proxy
        .method_call(
            "org.freedesktop.DBus.Properties",
            "Set",
            ("org.a11y.atspi.Value", "CurrentValue", value),
        )
        .map_err(|error| BackgroundError::failed(format!("AT-SPI 设值失败：{error}")))
}

fn send_x11_pointer(target: &AppTarget, resolved: &ResolvedAction) -> Result<(), BackgroundError> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|error| {
        BackgroundError::unavailable(format!(
            "无法连接 X11 显示（Wayland 可能无窗口 backing store）：{error}"
        ))
    })?;
    let screen = &conn.setup().roots[screen_num];
    let window = target.window_id;
    if window == 0 {
        return Err(BackgroundError::unavailable(
            "缺少 X11 window id，无法按窗口投递".to_string(),
        ));
    }
    match resolved.action {
        ComputerAction::Click => {
            let (x, y) = resolved.point.unwrap_or((0, 0));
            send_button(&conn, screen.root, window, x, y, resolved.button, true)?;
            send_button(&conn, screen.root, window, x, y, resolved.button, false)?;
        }
        ComputerAction::Scroll => {
            let (x, y) = resolved.point.unwrap_or((0, 0));
            let button = if resolved.scroll_y < 0 {
                4
            } else if resolved.scroll_y > 0 {
                5
            } else if resolved.scroll_x < 0 {
                6
            } else {
                7
            };
            send_button_code(&conn, screen.root, window, x, y, button, true)?;
            send_button_code(&conn, screen.root, window, x, y, button, false)?;
        }
        ComputerAction::Drag => {
            if resolved.path.len() < 2 {
                return Err(BackgroundError::failed("拖拽需要至少两个点".to_string()));
            }
            let (x0, y0) = (
                resolved.path[0].0 - target.bounds.x,
                resolved.path[0].1 - target.bounds.y,
            );
            send_button(&conn, screen.root, window, x0, y0, resolved.button, true)?;
            for (abs_x, abs_y) in resolved.path.iter().skip(1) {
                let x = abs_x - target.bounds.x;
                let y = abs_y - target.bounds.y;
                send_motion(&conn, window, x, y)?;
            }
            let last = resolved.path.last().copied().unwrap_or((0, 0));
            send_button(
                &conn,
                screen.root,
                window,
                last.0 - target.bounds.x,
                last.1 - target.bounds.y,
                resolved.button,
                false,
            )?;
        }
        _ => {}
    }
    conn.flush()
        .map_err(|error| BackgroundError::failed(format!("XSendEvent 刷新失败：{error}")))?;
    Ok(())
}

fn send_button(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    window: u32,
    x: i32,
    y: i32,
    button: ComputerButton,
    press: bool,
) -> Result<(), BackgroundError> {
    let code = match button {
        ComputerButton::Left => 1,
        ComputerButton::Middle => 2,
        ComputerButton::Right => 3,
    };
    send_button_code(conn, root, window, x, y, code, press)
}

fn send_button_code(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    window: u32,
    x: i32,
    y: i32,
    button: u8,
    press: bool,
) -> Result<(), BackgroundError> {
    let event_x = i16::try_from(x).unwrap_or(0);
    let event_y = i16::try_from(y).unwrap_or(0);
    if press {
        let event = ButtonPressEvent {
            response_type: BUTTON_PRESS_EVENT,
            detail: button,
            sequence: 0,
            time: 0,
            root,
            event: window,
            child: x11rb::NONE,
            root_x: event_x,
            root_y: event_y,
            event_x,
            event_y,
            state: 0u16.into(),
            same_screen: true,
        };
        conn.send_event(false, window, EventMask::BUTTON_PRESS, event)
            .map_err(|error| BackgroundError::failed(format!("XSendEvent 按下失败：{error}")))?;
    } else {
        let event = ButtonReleaseEvent {
            response_type: BUTTON_RELEASE_EVENT,
            detail: button,
            sequence: 0,
            time: 0,
            root,
            event: window,
            child: x11rb::NONE,
            root_x: event_x,
            root_y: event_y,
            event_x,
            event_y,
            state: 0u16.into(),
            same_screen: true,
        };
        conn.send_event(false, window, EventMask::BUTTON_RELEASE, event)
            .map_err(|error| BackgroundError::failed(format!("XSendEvent 松开失败：{error}")))?;
    }
    Ok(())
}

fn send_motion(
    conn: &x11rb::rust_connection::RustConnection,
    window: u32,
    x: i32,
    y: i32,
) -> Result<(), BackgroundError> {
    let event_x = i16::try_from(x).unwrap_or(0);
    let event_y = i16::try_from(y).unwrap_or(0);
    let event = MotionNotifyEvent {
        response_type: MOTION_NOTIFY_EVENT,
        detail: Motion::NORMAL,
        sequence: 0,
        time: 0,
        root: window,
        event: window,
        child: x11rb::NONE,
        root_x: event_x,
        root_y: event_y,
        event_x,
        event_y,
        state: KeyButMask::BUTTON1,
        same_screen: true,
    };
    conn.send_event(false, window, EventMask::POINTER_MOTION, event)
        .map_err(|error| BackgroundError::failed(format!("XSendEvent 移动失败：{error}")))?;
    Ok(())
}
