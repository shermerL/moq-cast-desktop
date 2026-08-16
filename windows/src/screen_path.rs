//! Canonical MoQCast screen broadcast paths.

const PREFIX: &str = "moqcast.screen/";

pub(crate) fn for_peer(peer_id: &str) -> String {
    format!("{PREFIX}{peer_id}")
}

pub(crate) fn peer_id(path: &str) -> Option<&str> {
    let peer_id = path.strip_prefix(PREFIX)?;
    (!peer_id.is_empty() && !peer_id.contains('/')).then_some(peer_id)
}

#[cfg(test)]
mod tests {
    use super::{for_peer, peer_id};

    #[test]
    fn builds_the_android_and_linux_compatible_screen_path() {
        assert_eq!(for_peer("peer-a"), "moqcast.screen/peer-a");
    }

    #[test]
    fn accepts_only_one_canonical_peer_segment() {
        assert_eq!(peer_id("moqcast.screen/peer-a"), Some("peer-a"));
        assert_eq!(peer_id("moqcast.screen/peer/a"), None);
        assert_eq!(peer_id("moqcast.screen/"), None);
        assert_eq!(peer_id("other/peer-a"), None);
    }
}
