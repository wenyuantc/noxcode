mod app;
mod db;
mod engine;
mod git;
mod process_spawn;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::app::ssh::{HostTrustBroker, HostTrustEvent, SshPool};

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
            let trust = Arc::new(HostTrustBroker::new(Duration::from_secs(120)));
            let handle = app.handle().clone();
            trust.set_emitter(move |event| match event {
                HostTrustEvent::Request(prompt) => {
                    let _ = handle.emit("ssh-host-trust-request", &prompt);
                }
                HostTrustEvent::KeyChanged(info) => {
                    let _ = handle.emit("ssh-host-key-changed", &info);
                }
            });
            let pool = SshPool::new(trust, Duration::from_secs(600));
            app.manage(pool.clone());
            pool.start_idle_reaper(Duration::from_secs(60));

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
            app::ssh::list_ssh_configs,
            app::ssh::get_ssh_config,
            app::ssh::create_ssh_config,
            app::ssh::update_ssh_config,
            app::ssh::delete_ssh_config,
            app::ssh::probe_ssh_password_auth,
            app::ssh::test_ssh_connection,
            app::ssh::list_ssh_config_file_hosts,
            app::ssh::import_ssh_config_file_host,
            app::ssh::resolve_ssh_host_trust,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            tauri::RunEvent::Ready => {
                git::preflight::show_fatal_dialog_if_needed(app);
            }
            tauri::RunEvent::Exit => {
                if let Some(pool) = app.try_state::<SshPool>() {
                    tauri::async_runtime::block_on(pool.shutdown());
                }
            }
            _ => {}
        });
}
