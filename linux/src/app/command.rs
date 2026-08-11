//! Commands sent from the UI to the background runtime.

/// A user request handled by the runtime resource owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserCommand {
    /// Begin looking for LAN peers.
    StartDiscovery,
    /// Stop looking for LAN peers.
    StopDiscovery,
    /// Connect to a discovered peer by stable id.
    ConnectPeer { peer_id: String },
    /// Disconnect the selected peer.
    Disconnect,
    /// Open the system picker and begin screen publishing.
    StartScreenShare,
    /// Stop the current screen publication while keeping the peer connected.
    StopScreenShare,
    /// Stop every runtime-owned task and exit.
    Shutdown,
}
