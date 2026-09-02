mod app;
mod db;
mod git;
mod process_spawn;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    git::preflight::run_startup_check();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(app::shared::DB_URL, db::migrations::get_all_migrations())
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    app::database::log_database_startup_status(&app_handle).await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::database::health_check,
            app::database::backup_database,
            app::database::restore_database,
            app::database::open_database_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Ready = event {
                git::preflight::show_fatal_dialog_if_needed(app);
            }
        });
}
