//! Generation-guarded application lifecycle owned outside the UI thread.

use std::sync::Arc;
use std::thread;

use thiserror::Error;
use tokio::sync::{mpsc, watch};

const COMMAND_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Generation(u64);

impl Generation {
    fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("lifecycle generation counter exhausted"),
        )
    }

    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lifecycle<P> {
    generation: Generation,
    phase: P,
}

impl<P: Copy> Lifecycle<P> {
    pub(crate) fn new(phase: P) -> Self {
        Self {
            generation: Generation::default(),
            phase,
        }
    }

    pub(crate) fn begin(&mut self, phase: P) -> Generation {
        self.generation = self.generation.next();
        self.phase = phase;
        self.generation
    }

    pub(crate) fn apply(&mut self, generation: Generation, phase: P) -> bool {
        if self.generation != generation {
            return false;
        }
        self.phase = phase;
        true
    }

    pub(crate) fn generation(&self) -> Generation {
        self.generation
    }

    pub(crate) fn phase(&self) -> P {
        self.phase
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RuntimePhase {
    #[default]
    Starting,
    Ready,
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CapabilityPhase {
    #[default]
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppSnapshot {
    pub(crate) runtime: Lifecycle<RuntimePhase>,
    pub(crate) discovery: Lifecycle<CapabilityPhase>,
    pub(crate) session: Lifecycle<CapabilityPhase>,
    pub(crate) capture: Lifecycle<CapabilityPhase>,
    pub(crate) decoder: Lifecycle<CapabilityPhase>,
    pub(crate) event_revision: u64,
    pub(crate) last_event: String,
    pub(crate) last_error: Option<String>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            runtime: Lifecycle::new(RuntimePhase::Starting),
            discovery: Lifecycle::new(CapabilityPhase::Unavailable),
            session: Lifecycle::new(CapabilityPhase::Unavailable),
            capture: Lifecycle::new(CapabilityPhase::Unavailable),
            decoder: Lifecycle::new(CapabilityPhase::Unavailable),
            event_revision: 0,
            last_event: "runtime owner starting".to_owned(),
            last_error: None,
        }
    }
}

impl AppSnapshot {
    fn record(&mut self, event: impl Into<String>) {
        self.event_revision = self.event_revision.saturating_add(1);
        self.last_event = event.into();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCommand {
    Shutdown,
}

#[derive(Debug, Error)]
pub(crate) enum RuntimeStartError {
    #[error("failed to create the async runtime: {0}")]
    AsyncRuntime(#[source] std::io::Error),
    #[error("failed to create the runtime owner thread: {0}")]
    OwnerThread(#[source] std::io::Error),
}

pub(crate) struct RuntimeOwner {
    commands: mpsc::Sender<RuntimeCommand>,
    snapshot: watch::Receiver<Arc<AppSnapshot>>,
    owner: Option<thread::JoinHandle<()>>,
}

impl RuntimeOwner {
    pub(crate) fn start() -> Result<Self, RuntimeStartError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moqcast-macos-async")
            .enable_all()
            .build()
            .map_err(RuntimeStartError::AsyncRuntime)?;
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshot) = watch::channel(Arc::new(AppSnapshot::default()));
        let owner = thread::Builder::new()
            .name("moqcast-macos-runtime".to_owned())
            .spawn(move || runtime.block_on(run(command_rx, snapshot_tx)))
            .map_err(RuntimeStartError::OwnerThread)?;

        Ok(Self {
            commands,
            snapshot,
            owner: Some(owner),
        })
    }

    pub(crate) fn snapshot(&self) -> Arc<AppSnapshot> {
        self.snapshot.borrow().clone()
    }

    fn shutdown(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        let _ = self.commands.blocking_send(RuntimeCommand::Shutdown);
        if owner.join().is_err() {
            tracing::error!(stage = "shutdown", "runtime owner thread panicked");
        }
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run(
    mut commands: mpsc::Receiver<RuntimeCommand>,
    snapshot_tx: watch::Sender<Arc<AppSnapshot>>,
) {
    let mut snapshot = AppSnapshot::default();
    let generation = snapshot.runtime.begin(RuntimePhase::Starting);
    assert!(snapshot.runtime.apply(generation, RuntimePhase::Ready));
    snapshot.record("runtime owner ready; LAN and media capabilities are not implemented in M1");
    snapshot_tx.send_replace(Arc::new(snapshot.clone()));
    tracing::info!(
        stage = "runtime",
        generation = generation.value(),
        "macOS runtime owner ready"
    );

    if let Some(RuntimeCommand::Shutdown) = commands.recv().await {
        let generation = snapshot.runtime.begin(RuntimePhase::Stopped);
        snapshot.record("runtime owner stopped");
        snapshot_tx.send_replace(Arc::new(snapshot));
        tracing::info!(
            stage = "shutdown",
            generation = generation.value(),
            "macOS runtime owner stopped"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Phase {
        Idle,
        Starting,
        Ready,
        Failed,
    }

    #[test]
    fn beginning_a_new_operation_advances_generation() {
        let mut lifecycle = Lifecycle::new(Phase::Idle);
        let first = lifecycle.begin(Phase::Starting);
        let second = lifecycle.begin(Phase::Starting);

        assert_ne!(first, second);
    }

    #[test]
    fn stale_events_cannot_override_the_current_phase() {
        let mut lifecycle = Lifecycle::new(Phase::Idle);
        let stale = lifecycle.begin(Phase::Starting);
        let current = lifecycle.begin(Phase::Starting);

        assert!(!lifecycle.apply(stale, Phase::Failed));
        assert!(lifecycle.apply(current, Phase::Ready));
        assert_eq!(lifecycle.phase(), Phase::Ready);
    }

    #[test]
    fn startup_publishes_ready_with_future_capabilities_unavailable() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(async {
                let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
                let (snapshot_tx, mut snapshot_rx) =
                    watch::channel(Arc::new(AppSnapshot::default()));
                let owner = tokio::spawn(run(command_rx, snapshot_tx));

                snapshot_rx.changed().await.expect("startup snapshot");
                let snapshot = snapshot_rx.borrow().clone();
                assert_eq!(snapshot.runtime.phase(), RuntimePhase::Ready);
                assert_eq!(snapshot.runtime.generation().value(), 1);
                assert_eq!(snapshot.discovery.phase(), CapabilityPhase::Unavailable);
                assert_eq!(snapshot.session.phase(), CapabilityPhase::Unavailable);
                assert_eq!(snapshot.capture.phase(), CapabilityPhase::Unavailable);
                assert_eq!(snapshot.decoder.phase(), CapabilityPhase::Unavailable);

                commands
                    .send(RuntimeCommand::Shutdown)
                    .await
                    .expect("shutdown command");
                owner.await.expect("runtime owner task");
            });
    }

    #[test]
    fn shutdown_joins_the_owner_and_publishes_stopped() {
        let mut runtime = RuntimeOwner::start().expect("runtime starts");
        runtime.shutdown();

        assert_eq!(runtime.snapshot().runtime.phase(), RuntimePhase::Stopped);
    }
}
