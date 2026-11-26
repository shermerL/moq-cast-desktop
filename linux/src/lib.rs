//! MoQCast Desktop application state, UI, and runtime ownership.

#[path = "../../shared/task-cleanup.rs"]
mod cleanup;

#[path = "../../shared/relay/desktop.rs"]
mod relay;

#[path = "../../shared/connection-bandwidth.rs"]
mod bandwidth;

#[path = "../../shared/watch-session.rs"]
mod watch_session;

pub mod app;
mod network;
mod publish;
pub mod runtime;
mod screen_path;
