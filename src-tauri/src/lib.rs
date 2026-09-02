mod git;
mod process_spawn;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    git::preflight::run_startup_check();

    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Ready = event {
                git::preflight::show_fatal_dialog_if_needed(app);
            }
        });
}
