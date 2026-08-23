//! Private peer registry built on the pinned moq-dev discovery contract.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::net::SocketAddr;

use moq_tokio::mdns;
use url::Url;

#[derive(Clone, PartialEq, Eq)]
struct Sensitive(String);

impl Sensitive {
    fn is_present(&self) -> bool {
        !self.0.is_empty()
    }
}

impl fmt::Debug for Sensitive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ObservedPeer {
    id: String,
    addrs: Vec<SocketAddr>,
    candidates: Vec<Url>,
    fingerprint: Option<Sensitive>,
    node: Option<Url>,
    credential: Sensitive,
    should_dial: bool,
}

impl fmt::Debug for ObservedPeer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservedPeer")
            .field("id", &sanitize_identity(&self.id))
            .field("addrs", &self.addrs)
            .field("candidates", &sanitize_candidates(&self.candidates))
            .field("fingerprint", &self.fingerprint)
            .field("node", &self.node.as_ref().map(sanitize_candidate))
            .field("credential", &self.credential)
            .field("should_dial", &self.should_dial)
            .finish()
    }
}

impl ObservedPeer {
    fn from_moq(peer: &mdns::Peer, should_dial: bool) -> Self {
        Self {
            id: peer.id.clone(),
            addrs: normalize_addrs(&peer.addrs),
            candidates: normalize_candidates(peer.urls()),
            fingerprint: peer.fingerprint.clone().map(Sensitive),
            node: peer.node.clone(),
            credential: Sensitive(peer.credential.clone()),
            should_dial,
        }
    }

    fn summary(&self, authenticated_discovery: bool) -> PeerSummary {
        PeerSummary {
            id: sanitize_identity(&self.id),
            candidates: sanitize_candidates(&self.candidates),
            should_dial: self.should_dial,
            authenticated_discovery,
            tls_pinned: self.fingerprint.as_ref().is_some_and(Sensitive::is_present),
        }
    }
}

fn normalize_addrs(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    let mut unique = HashSet::new();
    let mut normalized: Vec<_> = addrs
        .iter()
        .copied()
        .filter(|addr| unique.insert(*addr))
        .collect();
    normalized.sort_by_key(|addr| (addr.ip().is_loopback(), addr.is_ipv6(), *addr));
    normalized
}

fn normalize_candidates(candidates: Vec<Url>) -> Vec<Url> {
    let mut unique = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| unique.insert(candidate.to_string()))
        .collect()
}

pub(crate) fn sanitize_identity(id: &str) -> String {
    Url::parse(id).map_or_else(|_| id.to_string(), |url| sanitize_candidate(&url))
}

fn sanitize_candidates(candidates: &[Url]) -> Vec<String> {
    candidates.iter().map(sanitize_candidate).collect()
}

fn sanitize_candidate(candidate: &Url) -> String {
    let mut sanitized = candidate.clone();
    sanitized.set_username("").ok();
    sanitized.set_password(None).ok();
    sanitized.set_path("");
    sanitized.set_query(None);
    sanitized.set_fragment(None);
    sanitized.to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerSummary {
    pub(crate) id: String,
    pub(crate) candidates: Vec<String>,
    pub(crate) should_dial: bool,
    pub(crate) authenticated_discovery: bool,
    pub(crate) tls_pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RegistryChange {
    Added(PeerSummary),
    Updated(PeerSummary),
    Removed { id: String },
    Unchanged,
    IgnoredSelf,
}

pub(crate) struct PeerRegistry {
    local_id: String,
    authenticated_discovery: bool,
    peers: BTreeMap<String, ObservedPeer>,
}

impl PeerRegistry {
    pub(crate) fn new(local_id: impl Into<String>, authenticated_discovery: bool) -> Self {
        Self {
            local_id: local_id.into(),
            authenticated_discovery,
            peers: BTreeMap::new(),
        }
    }

    pub(crate) fn found(&mut self, peer: &mdns::Peer, should_dial: bool) -> RegistryChange {
        self.insert(ObservedPeer::from_moq(peer, should_dial))
    }

    fn insert(&mut self, peer: ObservedPeer) -> RegistryChange {
        if peer.id == self.local_id {
            return RegistryChange::IgnoredSelf;
        }

        let summary = peer.summary(self.authenticated_discovery);
        match self.peers.get(&peer.id) {
            Some(existing) if existing == &peer => RegistryChange::Unchanged,
            Some(_) => {
                self.peers.insert(peer.id.clone(), peer);
                RegistryChange::Updated(summary)
            }
            None => {
                self.peers.insert(peer.id.clone(), peer);
                RegistryChange::Added(summary)
            }
        }
    }

    pub(crate) fn lost(&mut self, id: &str) -> RegistryChange {
        match self.peers.remove(id) {
            Some(_) => RegistryChange::Removed {
                id: sanitize_identity(id),
            },
            None => RegistryChange::Unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn peer(id: &str) -> ObservedPeer {
        ObservedPeer {
            id: id.to_string(),
            addrs: vec![SocketAddr::from((Ipv4Addr::new(192, 168, 1, 8), 4443))],
            candidates: vec!["moqt://192.168.1.8:4443".parse().expect("candidate")],
            fingerprint: Some(Sensitive("fingerprint-secret-value".to_string())),
            node: None,
            credential: Sensitive("credential-secret-value".to_string()),
            should_dial: true,
        }
    }

    #[test]
    fn found_update_and_lost_are_distinct() {
        let mut registry = PeerRegistry::new("self", true);
        let original = peer("remote");
        assert!(matches!(
            registry.insert(original.clone()),
            RegistryChange::Added(_)
        ));
        assert_eq!(registry.insert(original.clone()), RegistryChange::Unchanged);

        let mut updated = original;
        updated.candidates = vec!["moqt://192.168.1.9:4443".parse().expect("candidate")];
        assert!(matches!(
            registry.insert(updated),
            RegistryChange::Updated(_)
        ));
        assert_eq!(
            registry.lost("remote"),
            RegistryChange::Removed {
                id: "remote".to_string()
            }
        );
        assert_eq!(registry.lost("remote"), RegistryChange::Unchanged);
    }

    #[test]
    fn self_record_is_ignored() {
        let mut registry = PeerRegistry::new("self", false);
        assert_eq!(registry.insert(peer("self")), RegistryChange::IgnoredSelf);
        assert!(registry.peers.is_empty());
    }

    #[test]
    fn ipv4_ipv6_and_loopback_addresses_are_normalized() {
        let lan_v6 = SocketAddr::from((Ipv6Addr::LOCALHOST.segments(), 4443));
        let lan_v4 = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 8), 4443));
        let loopback_v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, 4443));
        assert_eq!(
            normalize_addrs(&[lan_v6, loopback_v4, lan_v4, lan_v4]),
            vec![lan_v4, loopback_v4, lan_v6]
        );
    }

    #[test]
    fn candidate_order_is_preserved_while_duplicates_are_removed() {
        let ipv4: Url = "moqt://192.168.1.8:4443".parse().expect("IPv4");
        let ipv6: Url = "moqt://[2001:db8::8]:4443".parse().expect("IPv6");
        assert_eq!(
            normalize_candidates(vec![ipv4.clone(), ipv6.clone(), ipv4.clone()]),
            vec![ipv4, ipv6]
        );
    }

    #[test]
    fn sensitive_identity_fields_are_redacted() {
        let raw = "moqt://user:password@example.test:4443/path?credential=node-secret#fragment";
        let mut peer = peer(raw);
        peer.node = Some(raw.parse().expect("node"));
        peer.candidates = vec![peer.node.clone().expect("node")];
        let rendered = format!("{peer:?}");
        assert!(!rendered.contains("fingerprint-secret-value"));
        assert!(!rendered.contains("credential-secret-value"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("node-secret"));
        assert!(!rendered.contains("/path"));
        assert_eq!(rendered.matches("[redacted]").count(), 2);
    }

    #[test]
    fn summary_preserves_upstream_dial_decision_without_secrets() {
        let summary = peer("remote").summary(true);
        assert!(summary.should_dial);
        assert!(summary.authenticated_discovery);
        assert!(summary.tls_pinned);
        let rendered = format!("{summary:?}");
        assert!(!rendered.contains("fingerprint-secret-value"));
        assert!(!rendered.contains("credential-secret-value"));
    }

    #[test]
    fn summary_sanitizes_node_identity_and_candidate() {
        let raw = "moqt://user:password@example.test:4443/path?credential=node-secret#fragment";
        let mut observed = peer(raw);
        observed.node = Some(raw.parse().expect("node"));
        observed.candidates = vec![raw.parse().expect("candidate")];

        let summary = observed.summary(true);
        assert_eq!(summary.id, "moqt://example.test:4443");
        assert_eq!(summary.candidates, vec!["moqt://example.test:4443"]);
        let rendered = format!("{summary:?}");
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("node-secret"));
    }
}
