//! Shared application policy over discovery, attributed transport and content facts.
//! This crate owns no sockets, credentials, retries or platform resources.

pub mod bandwidth;

/// Only transport facts attributable to this particular peer belong here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    /// No attributed connection is known, including anonymous inbound sessions.
    #[default]
    Unknown,
    /// An attributed connection attempt is pending.
    Connecting,
    /// An attributed session is established.
    Connected,
    /// Authentication rejected the connection.
    Rejected,
    /// The attributed attempt or session failed.
    Failed,
    /// The attributed session ended.
    Disconnected,
}

/// Independent facts for one device row, not proof of a long-term identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    /// A current advertisement exists.
    pub discovered: bool,
    /// The state of a connection specifically associated with this peer.
    pub connection: Connection,
    /// The directory currently offers usable content for this peer's path.
    pub available: bool,
}

impl Peer {
    /// Keep discovered peers, pending/active attributed sessions, or usable content.
    pub fn visible(self) -> bool {
        self.discovered
            || self.available
            || matches!(
                self.connection,
                Connection::Connecting | Connection::Connected
            )
    }

    /// Project content and transport without treating advertisement loss as disconnection.
    pub fn status(self) -> Status {
        if self.available {
            return Status::Available;
        }
        match self.connection {
            Connection::Connected => Status::Connected,
            Connection::Connecting => Status::Connecting,
            Connection::Rejected => Status::Rejected,
            Connection::Failed => Status::Failed,
            Connection::Disconnected if self.discovered => Status::Disconnected,
            _ if self.discovered => Status::Found,
            _ => Status::NotDiscovered,
        }
    }
}

/// Product-level row status shared by all platform presentations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The advertisement is present without an attributed session.
    Found,
    /// Connection setup is in progress.
    Connecting,
    /// An attributed connection remains active.
    Connected,
    /// The directory offers watchable content.
    Available,
    /// Connection authentication was rejected.
    Rejected,
    /// Connection setup or transport failed.
    Failed,
    /// A known connection ended.
    Disconnected,
    /// No current advertisement or usable activity remains.
    NotDiscovered,
}
impl Status {
    /// Return the compact product label in the application's supported language.
    pub fn label(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::Found, true) => "已发现",
            (Self::Found, false) => "Found",
            (Self::Connecting, true) => "正在连接",
            (Self::Connecting, false) => "Connecting",
            (Self::Connected, true) => "已连接",
            (Self::Connected, false) => "Connected",
            (Self::Available, true) => "可观看",
            (Self::Available, false) => "Available",
            (Self::Rejected, true) => "无法验证连接",
            (Self::Rejected, false) => "Could not verify connection",
            (Self::Failed, true) => "连接失败",
            (Self::Failed, false) => "Connection failed",
            (Self::Disconnected, true) => "连接已结束",
            (Self::Disconnected, false) => "Connection ended",
            (Self::NotDiscovered, true) => "暂未发现",
            (Self::NotDiscovered, false) => "Not currently discovered",
        }
    }
}

/// Admission facts supplied by the LAN and media resource owners.
#[derive(Clone, Copy, Debug)]
pub struct Access {
    /// The LAN owner can accept new operations; false during start, stop or failed stop.
    pub ready: bool,
    /// Neither publication nor playback owns the media slot.
    pub media_idle: bool,
}
impl Access {
    /// Publishing may wait for viewers; it does not require a discovered peer.
    pub fn publish(self) -> bool {
        self.ready && self.media_idle
    }
    /// A usable route, not an unrelated global connection count, permits watching.
    pub fn watch(self, available: bool) -> bool {
        self.publish() && available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fact_combination_has_consistent_visibility_status_and_permissions() {
        for discovered in [false, true] {
            for connection in [
                Connection::Unknown,
                Connection::Connecting,
                Connection::Connected,
                Connection::Rejected,
                Connection::Failed,
                Connection::Disconnected,
            ] {
                for available in [false, true] {
                    let peer = Peer {
                        discovered,
                        connection,
                        available,
                    };
                    assert_eq!(
                        peer.visible(),
                        discovered
                            || available
                            || matches!(connection, Connection::Connecting | Connection::Connected)
                    );
                    if available {
                        assert_eq!(peer.status(), Status::Available);
                    }
                    for ready in [false, true] {
                        for media_idle in [false, true] {
                            let access = Access { ready, media_idle };
                            assert_eq!(access.watch(available), ready && media_idle && available);
                            assert_eq!(access.publish(), ready && media_idle);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn lost_does_not_disconnect_an_attributed_session_or_hide_content() {
        let mut peer = Peer {
            discovered: true,
            connection: Connection::Connected,
            available: true,
        };
        peer.discovered = false;
        assert!(peer.visible());
        assert_eq!(peer.status(), Status::Available);
        peer.available = false;
        assert!(peer.visible());
        assert_eq!(peer.status(), Status::Connected);
        peer.connection = Connection::Disconnected;
        assert!(!peer.visible());
    }

    #[test]
    fn found_lost_and_content_event_orders_converge() {
        for available_before_lost in [false, true] {
            let mut peer = Peer {
                discovered: true,
                ..Default::default()
            };
            if available_before_lost {
                peer.available = true;
            }
            peer.discovered = false;
            peer.available = true;
            assert!(peer.visible());
            assert_eq!(peer.connection, Connection::Unknown);
            peer.available = false;
            assert!(!peer.visible());
        }
    }

    #[test]
    fn unrelated_inbound_does_not_change_peer_facts() {
        // Anonymous inbound accounting intentionally has no input to this projection.
        let peer = Peer::default();
        assert!(!peer.visible());
        assert_eq!(peer.status(), Status::NotDiscovered);
    }

    #[test]
    fn stopping_or_off_blocks_new_media_even_with_retained_content() {
        assert!(
            !Access {
                ready: false,
                media_idle: true
            }
            .watch(true)
        );
        assert!(
            !Access {
                ready: false,
                media_idle: true
            }
            .publish()
        );
    }

    #[test]
    fn publication_and_playback_remain_exclusive() {
        let busy = Access {
            ready: true,
            media_idle: false,
        };
        assert!(!busy.publish());
        assert!(!busy.watch(true));
    }

    #[test]
    fn publication_does_not_depend_on_a_viewer_or_discovery_result() {
        assert!(
            Access {
                ready: true,
                media_idle: true
            }
            .publish()
        );
    }
}
