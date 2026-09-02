mod app;
mod db;
mod engine;
mod git;
mod native;
mod process_spawn;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::app::ssh::{HostTrustBroker, HostTrustEvent, SshPool};
use crate::native::manager::NativeAgentManager;

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
            app.manage(Arc::new(Mutex::new(NativeAgentManager::new())));
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
            git::get_git_repo_info,
            git::get_git_status,
            git::get_git_file_diff,
            git::get_git_numstat,
            git::stage_git_paths,
            git::unstage_git_paths,
            git::restore_git_paths,
            git::commit_git_changes,
            git::push_git_branch,
            git::list_git_branches,
            git::create_git_branch,
            git::create_git_checkpoint,
            git::list_git_checkpoints,
            git::preview_git_checkpoint_restore,
            git::restore_git_checkpoint,
            git::clear_git_checkpoints,
            app::network_settings::get_network_settings,
            app::network_settings::update_network_settings,
            native::channels::list_ai_channels,
            native::channels::create_ai_channel,
            native::channels::update_ai_channel,
            native::channels::delete_ai_channel,
            native::channels::test_ai_channel,
            native::channels::list_ai_channel_models,
            native::model_catalog::list_model_catalog,
            native::session::start_native_session,
            native::session::stop_native_session,
            native::session::stop_native,
            native::session::restart_native_session,
            native::session::resume_native_session,
            native::session::send_native_input,
            native::session::finish_native_input,
            native::session::resolve_native_tool_permission,
            native::session::answer_native_plan_question,
            native::settings::get_native_settings,
            native::settings::update_native_settings,
            native::skills::list_native_global_skills,
            native::skills::open_native_skills_dir,
            native::subagents::list_native_subagents,
            native::subagents::create_native_subagent,
            native::subagents::update_native_subagent,
            native::subagents::delete_native_subagent,
            native::api_logs::list_native_api_call_logs,
            native::api_logs::get_native_api_call_log,
            native::mcp_servers::get_mcp_servers,
            native::mcp_servers::update_mcp_servers,
            native::mcp_servers::reset_mcp_servers,
            app::profiles::list_agent_profiles,
            app::profiles::create_agent_profile,
            app::profiles::update_agent_profile,
            app::profiles::delete_agent_profile,
            app::workspaces::list_workspaces,
            app::workspaces::create_workspace,
            app::workspaces::update_workspace,
            app::workspaces::delete_workspace,
            app::workspaces::check_workspace_health,
            app::sessions::list_agent_sessions,
            app::sessions::get_agent_session_log_lines,
            app::sessions::prepare_agent_session_resume,
            app::sessions::delete_agent_session,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            tauri::RunEvent::Ready => {
                git::preflight::show_fatal_dialog_if_needed(app);
            }
            tauri::RunEvent::Exit => {
                if let Some(manager) = app.try_state::<Arc<Mutex<NativeAgentManager>>>() {
                    tauri::async_runtime::block_on(async {
                        manager.lock().await.cancel_all();
                    });
                }
                if let Some(pool) = app.try_state::<SshPool>() {
                    tauri::async_runtime::block_on(pool.shutdown());
                }
            }
            _ => {}
        });
}
