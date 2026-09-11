//! Tool runtime is consumed by the agent loop / engine child tasks.
#![allow(dead_code, unused_imports)]

pub mod cancel;
pub mod catalog;
pub mod command_path;
pub mod contract;
pub mod dispatch;
pub mod file_access;
#[cfg(test)]
mod file_access_tests;
pub mod glob;
pub mod hooks;
pub mod local;
pub mod lsp;
pub mod mcp;
pub mod patch;
pub mod paths;
pub mod permission;
pub mod processes;
pub mod question;
pub mod sandbox;
pub mod shell_snapshot;
pub mod sqlite;
pub mod ssh;
pub mod web;

pub use cancel::CancelFlag;
pub use catalog::{
    ask_question_spec, is_read_only_native_tool, read_only_tool_names,
    read_only_tool_names_with_bash, tool_contracts, tool_specs,
};
pub use contract::{
    builtin_contract, resolve_builtin_contract, ContractRegistry, PermissionCapability,
    PreviewDirection, ResultStrategy, RiskLevel, SideEffectScope, ToolContract,
};
pub use dispatch::{execute_tool, execute_tool_call, ToolCtx, ToolOutput};
pub use local::LocalWorkspace;
pub use mcp::{connect_mcp_servers, SharedMcp};
pub use question::PlanQuestion;
