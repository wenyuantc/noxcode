use tauri::{Window, WindowEvent};

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if let Err(error) = crate::window_state::save_window_size(window) {
            eprintln!("保存窗口尺寸失败: {error}");
        }
        api.prevent_close();
        let _ = window.hide();
    }
}
