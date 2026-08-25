//! Background runtime and UI communication handles.

#[cfg(target_os = "linux")]
mod playback;
#[cfg(any(target_os = "linux", test))]
mod playback_audio_continuity;
#[cfg(any(target_os = "linux", test))]
mod playback_sync;
mod supervisor;

use std::sync::Arc;
use std::thread;

use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::app::{AppSnapshot, UserCommand};

const COMMAND_CAPACITY: usize = 32;

/// The latest decoded remote screen frame in tightly packed RGBA.
#[derive(Clone)]
pub(crate) struct PlaybackFrame {
    pub(crate) identity: PlaybackFrameIdentity,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) rgba: Vec<u8>,
}

/// Identifies a decoded frame across playback sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaybackFrameIdentity {
    pub(crate) view_generation: u64,
    pub(crate) decoder_generation: u64,
    pub(crate) sequence: u64,
}

impl PlaybackFrame {
    #[cfg(target_os = "linux")]
    fn from_video(
        frame: moq_video::Frame,
        identity: PlaybackFrameIdentity,
    ) -> anyhow::Result<Self> {
        let width = frame.surface.width() as usize;
        let height = frame.surface.height() as usize;
        anyhow::ensure!(
            width > 0 && height > 0 && width.is_multiple_of(2) && height.is_multiple_of(2),
            "remote I420 frame dimensions must be non-zero and even"
        );
        let i420 = frame.surface.into_i420()?;
        let pixels = width
            .checked_mul(height)
            .ok_or_else(|| anyhow::anyhow!("remote frame dimensions overflow"))?;
        let i420_len = pixels
            .checked_mul(3)
            .map(|length| length / 2)
            .ok_or_else(|| anyhow::anyhow!("remote I420 frame length overflow"))?;
        anyhow::ensure!(
            i420.len() == i420_len,
            "remote I420 frame has an invalid byte length"
        );
        let u_offset = pixels;
        let v_offset = pixels + pixels / 4;
        let rgba_len = pixels
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("remote RGBA frame length overflow"))?;
        let mut rgba = Vec::with_capacity(rgba_len);
        for y in 0..height {
            for x in 0..width {
                let luma = i32::from(i420[y * width + x]) - 16;
                let chroma = (y / 2) * (width / 2) + x / 2;
                let u = i32::from(i420[u_offset + chroma]) - 128;
                let v = i32::from(i420[v_offset + chroma]) - 128;
                let r = (298 * luma + 409 * v + 128) >> 8;
                let g = (298 * luma - 100 * u - 208 * v + 128) >> 8;
                let b = (298 * luma + 516 * u + 128) >> 8;
                rgba.extend_from_slice(&[
                    r.clamp(0, 255) as u8,
                    g.clamp(0, 255) as u8,
                    b.clamp(0, 255) as u8,
                    255,
                ]);
            }
        }
        Ok(Self {
            identity,
            width,
            height,
            rgba,
        })
    }
}

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
    snapshot: watch::Receiver<Arc<AppSnapshot>>,
    playback: watch::Receiver<Option<Arc<PlaybackFrame>>>,
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
        let (snapshot_tx, snapshot) = watch::channel(Arc::new(AppSnapshot::default()));
        let (playback_tx, playback) = watch::channel(None);
        let owner = thread::Builder::new()
            .name("moqcast-runtime".into())
            .spawn(move || runtime.block_on(supervisor::run(command_rx, snapshot_tx, playback_tx)))
            .map_err(RuntimeStartError::OwnerThread)?;

        Ok(Self {
            commands,
            snapshot,
            playback,
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
    pub fn snapshot(&self) -> Arc<AppSnapshot> {
        self.snapshot.borrow().clone()
    }

    /// Clone the newest decoded remote frame without waiting.
    pub(crate) fn playback_frame(&self) -> Option<Arc<PlaybackFrame>> {
        self.playback.borrow().clone()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_sequences_from_different_view_generations_are_different_frames() {
        let previous = PlaybackFrameIdentity {
            view_generation: 1,
            decoder_generation: 1,
            sequence: 1,
        };
        let next = PlaybackFrameIdentity {
            view_generation: 2,
            decoder_generation: 1,
            sequence: 1,
        };

        assert_ne!(previous, next);
    }
}
