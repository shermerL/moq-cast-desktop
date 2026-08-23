//! Peer request-path construction and constant-time credential checks.

const PEER_PATH: &str = "/.cluster";

pub(crate) fn peer_path(credential: &str) -> String {
    format!("{PEER_PATH}/{credential}")
}

pub(crate) fn authorized(path: &str, expected: &str) -> bool {
    let Some(presented) = path
        .strip_prefix(PEER_PATH)
        .and_then(|path| path.strip_prefix('/'))
    else {
        return false;
    };
    !presented.is_empty() && !presented.contains('/') && moq_tokio::mdns::ct_eq(expected, presented)
}

#[cfg(test)]
mod tests {
    use super::{authorized, peer_path};

    #[test]
    fn accepts_only_the_exact_peer_path_and_credential() {
        assert_eq!(peer_path("abc123"), "/.cluster/abc123");
        assert!(authorized("/.cluster/abc123", "abc123"));
        assert!(!authorized("/.cluster/wrong", "abc123"));
        assert!(!authorized("/.cluster", "abc123"));
        assert!(!authorized("/.clusterish/abc123", "abc123"));
    }
}
