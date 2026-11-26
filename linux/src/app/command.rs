//! Commands sent from the UI to the background runtime.

use super::CaptureSource;

/// A user request handled by the runtime resource owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserCommand {
    /// Manually control the independent relay.
    Relay(crate::relay::Command),
    /// Publish the selected native source to the current relay connection.
    StartRelayShare {
        connection: crate::relay::Generation,
        prefix: crate::relay::PublishPrefix,
        system_audio: bool,
        source: CaptureSource,
    },
    /// Begin looking for LAN peers.
    StartDiscovery,
    /// Stop looking for LAN peers.
    StopDiscovery,
    /// Restart LAN discovery and its listener after a visible failure.
    RetryDiscovery,
    /// Refresh available capture sources without opening the system picker.
    RefreshCaptureSources,
    /// Begin publishing the selected source.
    StartScreenShare {
        system_audio: bool,
        source: CaptureSource,
    },
    /// Stop the current screen publication while keeping the peer connected.
    StopScreenShare,
    /// Begin viewing one announced remote screen.
    StartWatching { path: String },
    /// Set the volume of the matching active remote playback session.
    SetPlaybackVolume { generation: u64, percent: u8 },
    /// Stop the current remote screen playback while keeping the mesh connected.
    StopWatching,
    /// Stop every runtime-owned task and exit.
    Shutdown,
}
