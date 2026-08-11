//! Background runtime and UI communication handles.

mod supervisor;

use std::thread;

use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::app::{AppSnapshot, UserCommand};

const COMMAND_CAPACITY: usize = 32;

/// Failure to start the background runtime thread.
#[derive(Debug, Error)]
pub enum RuntimeStartError {
    /// Tokio could not create its worker threads.
    #[error("failed to create the async runtime: {0}")]
    AsyncRuntime(#[source] std::io::Error),
    /// The operating system could not create the owner thread.
    #[error("failed to create the runtime owner thread: {0}")]
    OwnerThread(#[source] std::io::Error),
}

/// Failure to enqueue a non-blocking UI command.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RuntimeSendError {
    /// The bounded queue is full and the user should retry after state advances.
    #[error("the background runtime is busy")]
    Busy,
    /// The runtime has already stopped.
    #[error("the background runtime is no longer available")]
    Closed,
}

/// UI-side handle for the runtime owner thread.
pub struct RuntimeHandle {
    commands: mpsc::Sender<UserCommand>,
    snapshot: watch::Receiver<AppSnapshot>,
    owner: Option<thread::JoinHandle<()>>,
}

impl RuntimeHandle {
    /// Start a bounded command channel and its Tokio owner thread.
    pub fn start() -> Result<Self, RuntimeStartError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moqcast-async")
            .enable_all()
            .build()
            .map_err(RuntimeStartError::AsyncRuntime)?;
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshot) = watch::channel(AppSnapshot::default());
        let owner = thread::Builder::new()
            .name("moqcast-runtime".into())
            .spawn(move || runtime.block_on(supervisor::run(command_rx, snapshot_tx)))
            .map_err(RuntimeStartError::OwnerThread)?;

        Ok(Self {
            commands,
            snapshot,
            owner: Some(owner),
        })
    }

    /// Enqueue one command without blocking the UI event loop.
    pub fn try_send(&self, command: UserCommand) -> Result<(), RuntimeSendError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RuntimeSendError::Busy,
                mpsc::error::TrySendError::Closed(_) => RuntimeSendError::Closed,
            })
    }

    /// Clone the newest runtime snapshot without waiting.
    pub fn snapshot(&self) -> AppSnapshot {
        self.snapshot.borrow().clone()
    }

    fn shutdown(&mut self) {
        if let Some(owner) = self.owner.take() {
            let _ = self.commands.blocking_send(UserCommand::Shutdown);
            if owner.join().is_err() {
                tracing::error!("runtime owner thread panicked during shutdown");
            }
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
