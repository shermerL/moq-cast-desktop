//! Cross-platform MoQCast protocol and product contract.

#[cfg(test)]
pub(crate) const SERVICE_TYPE: &str = "_moq._udp.local.";
pub(crate) const CLUSTER_PATH_PREFIX: &str = "/.cluster/";
pub(crate) const SCREEN_PATH_PREFIX: &str = "moqcast.screen/";

pub(crate) fn cluster_path(credential: &str) -> String {
    format!("{CLUSTER_PATH_PREFIX}{credential}")
}

pub(crate) fn screen_path(peer_id: &str) -> String {
    format!("{SCREEN_PATH_PREFIX}{peer_id}")
}

pub(crate) fn screen_peer_id(path: &str) -> Option<&str> {
    let peer_id = path.strip_prefix(SCREEN_PATH_PREFIX)?;
    (!peer_id.is_empty() && !peer_id.contains('/')).then_some(peer_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_the_cross_platform_contract() {
        assert_eq!(SERVICE_TYPE, "_moq._udp.local.");
        assert_eq!(CLUSTER_PATH_PREFIX, "/.cluster/");
        assert_eq!(SCREEN_PATH_PREFIX, "moqcast.screen/");
        assert_eq!(cluster_path("temporary"), "/.cluster/temporary");
        assert_eq!(screen_path("peer-7"), "moqcast.screen/peer-7");
    }

    #[test]
    fn screen_peer_id_accepts_only_the_canonical_broadcast_path() {
        assert_eq!(screen_peer_id("moqcast.screen/peer-7"), Some("peer-7"));
        assert_eq!(screen_peer_id("moqcast.screen/"), None);
        assert_eq!(screen_peer_id("moqcast.screen/peer-7/extra"), None);
        assert_eq!(screen_peer_id("other/peer-7"), None);
    }
}
