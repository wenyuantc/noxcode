//! Native model clients used by the in-process agent loop.
//!
//! P3 只落地 probe / list_models。P4.1 用完整实现覆盖本目录。
#![allow(dead_code)]

pub mod client;
pub mod retry;

#[allow(unused_imports)]
pub use client::{ListedModels, ModelClient, ModelClientConfig};
pub use retry::RetryConfig;
