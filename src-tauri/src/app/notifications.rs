use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

const MAX_NOTIFICATION_BODY_CHARS: usize = 120;

fn should_notify(enabled: bool, focused: bool) -> bool {
    enabled && !focused
}

fn truncate_body(body: &str) -> String {
    if body.chars().count() <= MAX_NOTIFICATION_BODY_CHARS {
        return body.to_string();
    }

    let mut truncated = body
        .chars()
        .take(MAX_NOTIFICATION_BODY_CHARS - 1)
        .collect::<String>();
    truncated.push('…');
    truncated
}

pub(crate) fn notify_if_unfocused<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    let enabled = match crate::native::settings::load_native_settings(app) {
        Ok(settings) => settings.desktop_notifications,
        Err(error) => {
            eprintln!("读取桌面通知设置失败: {error}");
            return;
        }
    };
    let focused = app
        .get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false);
    if !should_notify(enabled, focused) {
        return;
    }

    if let Err(error) = app
        .notification()
        .builder()
        .title(title)
        .body(truncate_body(body))
        .show()
    {
        eprintln!("发送桌面通知失败: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{should_notify, truncate_body, MAX_NOTIFICATION_BODY_CHARS};

    #[test]
    fn notifies_only_when_enabled_and_unfocused() {
        assert!(should_notify(true, false));
        assert!(!should_notify(false, false));
        assert!(!should_notify(true, true));
        assert!(!should_notify(false, true));
    }

    #[test]
    fn truncates_notification_body_by_character_count() {
        let exact = "中".repeat(MAX_NOTIFICATION_BODY_CHARS);
        assert_eq!(truncate_body(&exact), exact);

        let long = "中".repeat(MAX_NOTIFICATION_BODY_CHARS + 10);
        let truncated = truncate_body(&long);
        assert_eq!(truncated.chars().count(), MAX_NOTIFICATION_BODY_CHARS);
        assert!(truncated.ends_with('…'));
    }
}
