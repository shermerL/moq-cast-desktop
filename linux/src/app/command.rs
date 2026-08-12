//! Commands sent from the UI to the background runtime.

/// A user request handled by the runtime resource owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserCommand {
    /// Begin looking for LAN peers.
    StartDiscovery,
    /// Stop looking for LAN peers.
    StopDiscovery,
    /// Restart LAN discovery and its listener after a visible failure.
    RetryDiscovery,
    /// Open the system picker and begin screen publishing.
    StartScreenShare,
    /// Stop the current screen publication while keeping the peer connected.
    StopScreenShare,
    /// Begin viewing one announced remote screen.
    StartWatching { path: String },
    /// Stop the current remote screen playback while keeping the mesh connected.
    StopWatching,
    /// Stop every runtime-owned task and exit.
    Shutdown,
}
