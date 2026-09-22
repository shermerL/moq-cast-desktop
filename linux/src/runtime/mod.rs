//! Background runtime and UI communication handles.

#[cfg(target_os = "linux")]
mod playback;
#[cfg(any(target_os = "linux", test))]
mod playback_audio_config;
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
        let image = frame
            .surface
            .to_rgba(&moq_video::convert::Config::default())?;
        Ok(Self {
            identity,
            width: image.width() as usize,
            height: image.height() as usize,
            rgba: image.into_data(),
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

    #[cfg(target_os = "linux")]
    #[test]
    fn decoded_i420_uses_the_vendor_rgba_conversion() {
        let size = moq_video::Size::new(2, 2);
        let pixels = moq_video::I420::new(size, vec![16, 16, 16, 16, 128, 128]).unwrap();
        let frame = moq_video::Frame::new(
            moq_video::Surface::I420(pixels),
            moq_tokio::moq_net::Timestamp::ZERO,
        );
        let identity = PlaybackFrameIdentity {
            view_generation: 1,
            decoder_generation: 1,
            sequence: 1,
        };

        let converted = PlaybackFrame::from_video(frame, identity).unwrap();
        assert_eq!((converted.width, converted.height), (2, 2));
        assert_eq!(converted.rgba.len(), 16);
        assert_eq!(
            converted
                .rgba
                .chunks_exact(4)
                .map(|pixel| pixel[3])
                .collect::<Vec<_>>(),
            vec![255; 4]
        );
    }

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
