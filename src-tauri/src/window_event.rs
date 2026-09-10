use tauri::{Manager, Window, WindowEvent};

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::Moved(position) => {
            crate::window_state::remember_moved(window, *position);
        }
        WindowEvent::Resized(size) => {
            crate::window_state::remember_resized(window, *size);
        }
        WindowEvent::CloseRequested { api, .. } => {
            if !crate::app::lifecycle::is_stopping(window.app_handle()) {
                if let Err(error) = crate::window_state::persist_window(window) {
                    eprintln!("保存窗口状态失败: {error}");
                }
            }
            api.prevent_close();
            let _ = window.hide();
        }
        _ => {}
    }
}
