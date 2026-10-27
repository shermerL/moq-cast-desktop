//! MoQCast Desktop application state, UI, and runtime ownership.

#[path = "../../shared/task-cleanup.rs"]
mod cleanup;

#[path = "../../shared/connection-bandwidth.rs"]
mod bandwidth;

pub mod app;
mod network;
mod publish;
pub mod runtime;
mod screen_path;
