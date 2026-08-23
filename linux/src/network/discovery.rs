//! Discovered-peer records and lifecycle reduction.

use std::collections::HashMap;
use std::net::SocketAddr;

use moq_tokio::mdns;
use url::Url;

/// Connection details retained for one discovered peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerRecord {
    pub(crate) id: String,
    pub(crate) addrs: Vec<SocketAddr>,
    pub(crate) urls: Vec<Url>,
    pub(crate) fingerprint: Option<String>,
    pub(crate) has_node: bool,
    pub(crate) credential: String,
}

impl PeerRecord {
    pub(crate) fn from_mdns(peer: mdns::Peer) -> Self {
        let urls = peer.urls();
        Self {
            id: peer.id,
            addrs: peer.addrs,
            urls,
            fingerprint: peer.fingerprint,
            has_node: peer.node.is_some(),
            credential: peer.credential,
        }
    }

    pub(crate) fn endpoint_labels(&self) -> Vec<String> {
        self.addrs.iter().map(ToString::to_string).collect()
    }

    #[cfg(test)]
    pub(crate) fn for_test(id: &str, addr: SocketAddr, credential: &str) -> Self {
        Self {
            id: id.to_owned(),
            addrs: vec![addr],
            urls: vec![format!("moqt://{addr}").parse().unwrap()],
            fingerprint: Some("test-fingerprint".into()),
            has_node: false,
            credential: credential.to_owned(),
        }
    }

    fn same_advertisement(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
            && self.has_node == other.has_node
            && self.credential == other.credential
    }

    fn merge_candidates(&mut self, other: Self) -> bool {
        let mut changed = false;
        for addr in other.addrs {
            if !self.addrs.contains(&addr) {
                self.addrs.push(addr);
                changed = true;
            }
        }
        for url in other.urls {
            if !self.urls.contains(&url) {
                self.urls.push(url);
                changed = true;
            }
        }
        changed
    }
}

/// Current mDNS records keyed by the stable peer id.
pub(crate) struct PeerRegistry {
    local_id: String,
    peers: HashMap<String, PeerRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeerUpdate {
    Unchanged,
    Added,
    CandidatesMerged,
    IdentityReplaced,
}

impl PeerUpdate {
    pub(crate) fn changed(self) -> bool {
        self != Self::Unchanged
    }
}

impl PeerRegistry {
    pub(crate) fn new(local_id: impl Into<String>) -> Self {
        Self {
            local_id: local_id.into(),
            peers: HashMap::new(),
        }
    }

    pub(crate) fn found(&mut self, peer: PeerRecord) -> PeerUpdate {
        if peer.id == self.local_id {
            return PeerUpdate::Unchanged;
        }

        match self.peers.get_mut(&peer.id) {
            Some(current) if current.same_advertisement(&peer) => {
                if current.merge_candidates(peer) {
                    PeerUpdate::CandidatesMerged
                } else {
                    PeerUpdate::Unchanged
                }
            }
            Some(_) => {
                self.peers.insert(peer.id.clone(), peer);
                PeerUpdate::IdentityReplaced
            }
            None => {
                self.peers.insert(peer.id.clone(), peer);
                PeerUpdate::Added
            }
        }
    }

    pub(crate) fn lost(&mut self, id: &str) -> bool {
        self.peers.remove(id).is_some()
    }

    pub(crate) fn get(&self, id: &str) -> Option<&PeerRecord> {
        self.peers.get(id)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.peers.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::{PeerRecord, PeerRegistry, PeerUpdate};

    fn record(id: &str, octet: u8, credential: &str) -> PeerRecord {
        PeerRecord::for_test(
            id,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, octet)), 4443),
            credential,
        )
    }

    #[test]
    fn ignores_self_and_merges_candidates_from_one_advertisement() {
        let mut peers = PeerRegistry::new("linux-self");

        assert_eq!(
            peers.found(record("linux-self", 1, "self-proof")),
            PeerUpdate::Unchanged
        );
        assert_eq!(
            peers.found(record("android", 2, "same-proof")),
            PeerUpdate::Added
        );
        assert_eq!(
            peers.found(record("android", 3, "same-proof")),
            PeerUpdate::CandidatesMerged
        );

        let peer = peers.get("android").unwrap();
        assert_eq!(peer.addrs.len(), 2);
        assert_eq!(peers.len(), 1);
    }

    #[test]
    fn replaces_candidates_when_security_identity_rotates_and_handles_lost() {
        let mut peers = PeerRegistry::new("linux-self");
        assert_eq!(
            peers.found(record("android", 2, "old-proof")),
            PeerUpdate::Added
        );
        assert_eq!(
            peers.found(record("android", 3, "new-proof")),
            PeerUpdate::IdentityReplaced
        );

        let peer = peers.get("android").unwrap();
        assert_eq!(peer.addrs.len(), 1);
        assert_eq!(peer.credential, "new-proof");

        assert!(peers.lost("android"));
        assert!(peers.is_empty());
        assert!(!peers.lost("android"));
    }
}
