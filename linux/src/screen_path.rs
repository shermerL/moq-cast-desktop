//! Screen broadcast path construction and parsing.

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
    fn screen_paths_round_trip_one_peer_id() {
        let path = for_peer("peer-a");

        assert_eq!(path, "moqcast.screen/peer-a");
        assert_eq!(peer_id(&path), Some("peer-a"));
        assert_eq!(peer_id("moqcast.screen/peer/a"), None);
        assert_eq!(peer_id("moqcast.screen/"), None);
        assert_eq!(peer_id("other/peer-a"), None);
    }
}
