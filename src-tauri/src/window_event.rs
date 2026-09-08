use tauri::{Manager, Window, WindowEvent};

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if !crate::app::lifecycle::is_stopping(window.app_handle()) {
            if let Err(error) = crate::window_state::save_window_size(window) {
                eprintln!("保存窗口尺寸失败: {error}");
            }
        }
        api.prevent_close();
        let _ = window.hide();
    }
}
