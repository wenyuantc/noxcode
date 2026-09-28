use tauri::{Window, WindowEvent};

fn should_hide_on_close(label: &str) -> bool {
    label == "main"
}

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if !should_hide_on_close(window.label()) {
            return;
        }
        api.prevent_close();
        let _ = window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_browser_window_is_not_hidden_on_close() {
        assert!(should_hide_on_close("main"));
        assert!(!should_hide_on_close("browser"));
    }
}
