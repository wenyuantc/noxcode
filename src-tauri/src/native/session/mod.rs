#![allow(unused_imports)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};

use crate::app::network_settings::{load_network_settings, proxy_env_vars};
use crate::app::sessions::{
    lock_agent_session_operation, persist_context_usage_with, require_unarchived_session_with,
};
use crate::app::shared::{new_id, now_sqlite, sqlite_pool};
use crate::app::ssh::configs::fetch_ssh_config_record_by_id;
use crate::app::ssh::validate_password_execution;
use crate::db::models::{
    AgentSessionExit, AgentSessionOutput, AgentSessionRecord, AgentSessionStarted,
    NativeContextUsage, NativePlanModeChanged, NativeSessionConfigurationEvent,
    NativeSessionRuntime, NativeTextDelta, NativeToolEvent, NativeToolImage, SshConfigRecord,
    StartNativeSessionInput, UpdateNativeSessionConfigurationInput,
};
use crate::engine::context::{resolve_workspace_execution_context_with_pool, ExecutionContext};
use crate::engine::UsageDelta;
use crate::git::create_checkpoint;
use crate::native::agent::compact::{BudgetSnapshot, CompactTrigger, ContextWindow};
use crate::native::agent::r#loop::AgentDiagnosticsSnapshot;
use crate::native::agent::r#loop::{AgentRunner, NativeEvent, TranscriptCheckpoint};
use crate::native::api_logs::sqlite_call_log_sink;
use crate::native::channels::{fetch_channel_record, require_channel_api_key};
use crate::native::history::{commit_model_context, HistoryWrite};
use crate::native::input_queue::{NativeInputQueue, NativeInputQueueSnapshot};
use crate::native::live_model::{write_live_model, LiveModelSnapshot, SharedLiveModel};
use crate::native::manager::{
    take_latest_configuration, NativeAgentManager, NativeCompactionRequest,
    NativeConfigurationRequest, NativeFollowup, NativeLiveSession, NativeSessionInfo,
    PendingPermission, PendingPlanApproval, PendingPlanQuestion, PermissionRequest,
    PlanApprovalRequest, PlanQuestionRequest,
};
use crate::native::mcp_servers::resolve_session_mcp_servers;
use crate::native::model::call_log::{
    CallLogContext, CALL_KIND_CHAT, CALL_KIND_ONE_SHOT, CALL_KIND_PLAN,
};
use crate::native::model::types::StreamDelta;
use crate::native::model::{ModelClient, ModelClientConfig, ResponsesContinuationMode};
use crate::native::model_catalog::{
    apply_catalog_defaults, fill_from_catalog, resolve_runtime_reasoning_effort,
};
use crate::native::plans::{
    self, ApprovedPlanSnapshot, PendingPlanSnapshot, PlanAuthorization, PlanSaveStatus,
};
use crate::native::protocol::record_to_channel;
use crate::native::tools::dispatch::PlanApprovalAnswer;
use crate::native::tools::permission::{
    NativePermissionDecision, NativeToolRiskKind, PermissionRuleSuggestion,
};
use crate::native::tools::question::PlanQuestionAnswer;
use crate::native::tools::{
    connect_mcp_servers, local::LocalWorkspace, ssh::SshToolRuntime, SharedMcp,
};
use crate::native::transcript::{
    load_transcript, save_transcript, transcript_fingerprint, NativeTranscriptMeta,
};

const ENGINE_LABEL: &str = "内置 Agent";

mod events;

mod run_settings;

mod lifecycle;

mod startup;

mod run_loop;

mod permissions;

mod history;

mod memory;

mod steer;

mod background;

mod commands;

#[cfg(test)]
mod tests;

use events::{
    announce_isolation_restore, apply_bound_subagent, approve_workspace_hooks, attach_memory,
    attach_skills_and_hooks, attach_subagent_runtime, attach_transcript_checkpoint,
    emit_native_line, emit_native_output, emit_plan_mode, emit_turn_state, extra_headers_map,
    finish_memory, forward_native_events, hook_agent_handler, insert_session_event,
    native_images_for_output, permission_event, persist_native_transcript,
    persist_runner_transcript, persist_stdout_message, persist_steer_receipt, plan_approval_event,
    question_event, session_kind, user_turn_count, NativeDeltaEmitter,
    NativePermissionRequestEvent, NativePlanApprovalEvent, NativePlanQuestionEvent,
    DELTA_FLUSH_BYTES, DELTA_FLUSH_INTERVAL, DELTA_SEGMENT_REASONING, DELTA_SEGMENT_TEXT,
};

pub(crate) use events::{next_loop_step, NativeLoopAction, NativeLoopEvent};

use run_settings::{
    configure_runner_limits, format_native_diagnostics, is_cancelled_run_error,
    is_mcp_error_status, live_snapshot_from_run, load_native_client,
    load_native_client_from_channel, native_one_shot_text, native_startup_banner,
    one_shot_reasoning_usable, publish_live_run, resolve_run_model_config,
    run_native_one_shot_with_run, should_announce_session_startup, sync_run_from_live,
    NativeRunSettings,
};

pub(crate) use run_settings::{run_native_one_shot, NativeOneShotArgs};

pub use run_settings::NativeOneShotResult;

use lifecycle::{
    apply_session_usage, attach_mutation_checkpoint, configure_local_tool_runtime,
    enqueue_live_input, insert_agent_session, load_session_ssh_config,
    maybe_isolate_session_worktree, reactivate_agent_session, resolve_insert_title,
    update_agent_session_status,
};

pub(crate) use lifecycle::session_title;

pub use lifecycle::restore_session_worktree;

pub use lifecycle::{
    __cmd__restore_session_worktree, __tauri_command_name_restore_session_worktree,
};

use startup::{
    apply_run_settings_to_runner, apply_session_configuration, bind_run_to_session,
    clear_pending_plan, clear_pending_plan_with, dispatch_session_configuration, save_pending_plan,
    save_pending_plan_with, start_native_session_locked, update_agent_session_channel,
    validate_live_configuration,
};

pub(crate) use startup::{recv_idle_wait, start_native_with_manager, NativeIdleWait};

pub use startup::start_native_session;

pub use startup::{__cmd__start_native_session, __tauri_command_name_start_native_session};

use run_loop::{run_native_loop, stop_native_process};

use permissions::{files_rollback_enabled, live_files_for_session};

pub use permissions::{
    apply_native_file_rollback, fork_native_session, preview_native_file_rollback,
    resolve_native_tool_permission, NativeFileRollbackInput, NativeFileRollbackPreviewInput,
};

pub use permissions::{
    __cmd__apply_native_file_rollback, __cmd__fork_native_session,
    __cmd__preview_native_file_rollback, __cmd__resolve_native_tool_permission,
    __tauri_command_name_apply_native_file_rollback, __tauri_command_name_fork_native_session,
    __tauri_command_name_preview_native_file_rollback,
    __tauri_command_name_resolve_native_tool_permission,
};

use history::{
    boundary_request_session, ensure_idle_for_boundary, history_revision, record_branch_display,
};

pub use history::{apply_native_history_boundary, NativeBoundaryInput};

pub use history::{
    __cmd__apply_native_history_boundary, __tauri_command_name_apply_native_history_boundary,
};

use memory::{
    apply_plan_implementation_model, clear_pending_plan_request, plan_implementation_unchanged,
    resolve_plan_implementation_effort,
};

pub(crate) use memory::invalidate_native_plan_authorization;

pub use memory::{
    answer_native_plan_question, compact_native_session, dream_native_memory,
    resolve_native_plan_approval, stop_native, stop_native_session,
};

pub use memory::{
    __cmd__answer_native_plan_question, __cmd__compact_native_session, __cmd__dream_native_memory,
    __cmd__resolve_native_plan_approval, __cmd__stop_native, __cmd__stop_native_session,
    __tauri_command_name_answer_native_plan_question, __tauri_command_name_compact_native_session,
    __tauri_command_name_dream_native_memory, __tauri_command_name_resolve_native_plan_approval,
    __tauri_command_name_stop_native, __tauri_command_name_stop_native_session,
};

use steer::{
    emit_request_resolved, find_steer_receipt, load_steer_receipts, load_steer_submission_images,
    submit_native_steer_with, SteerSubmission,
};

pub use steer::{
    get_native_steer_snapshot, list_native_queued_inputs, remove_native_queued_input,
    send_native_input, submit_native_steer, update_native_queued_input,
};

pub use steer::{
    __cmd__get_native_steer_snapshot, __cmd__list_native_queued_inputs,
    __cmd__remove_native_queued_input, __cmd__send_native_input, __cmd__submit_native_steer,
    __cmd__update_native_queued_input, __tauri_command_name_get_native_steer_snapshot,
    __tauri_command_name_list_native_queued_inputs,
    __tauri_command_name_remove_native_queued_input, __tauri_command_name_send_native_input,
    __tauri_command_name_submit_native_steer, __tauri_command_name_update_native_queued_input,
};

use background::background_registry;

pub use background::{
    list_native_background_processes, list_native_background_tasks, send_native_background_message,
    stop_native_background_process, stop_native_background_task,
};

pub use background::{
    __cmd__list_native_background_processes, __cmd__list_native_background_tasks,
    __cmd__send_native_background_message, __cmd__stop_native_background_process,
    __cmd__stop_native_background_task, __tauri_command_name_list_native_background_processes,
    __tauri_command_name_list_native_background_tasks,
    __tauri_command_name_send_native_background_message,
    __tauri_command_name_stop_native_background_process,
    __tauri_command_name_stop_native_background_task,
};

use commands::{extract_event_line, finish_live_input};

pub use commands::{
    extract_session_subagents, finish_native_input, get_session_subagents, restart_native_session,
    resume_native_session, update_native_session_configuration, SessionSubagentInfo,
};

pub use commands::{
    __cmd__finish_native_input, __cmd__get_session_subagents, __cmd__restart_native_session,
    __cmd__resume_native_session, __cmd__update_native_session_configuration,
    __tauri_command_name_finish_native_input, __tauri_command_name_get_session_subagents,
    __tauri_command_name_restart_native_session, __tauri_command_name_resume_native_session,
    __tauri_command_name_update_native_session_configuration,
};
