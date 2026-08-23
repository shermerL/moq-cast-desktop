//! Strict LAN peer request-path construction and authorization.

const CLUSTER_PATH: &str = "/.cluster";

pub(crate) fn peer_path(credential: &str) -> String {
    format!("{CLUSTER_PATH}/{credential}")
}

pub(crate) fn authorized(path: &str, expected: &str) -> bool {
    let Some(presented) = path
        .strip_prefix(CLUSTER_PATH)
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
    fn accepts_only_the_exact_cluster_path_and_credential() {
        assert_eq!(peer_path("expected"), "/.cluster/expected");
        assert!(authorized("/.cluster/expected", "expected"));
        assert!(!authorized("/.cluster/wrong", "expected"));
        assert!(!authorized("/.cluster", "expected"));
        assert!(!authorized("/.cluster/", "expected"));
        assert!(!authorized("/.cluster/expected/extra", "expected"));
        assert!(!authorized("/.clusterish/expected", "expected"));
    }
}
