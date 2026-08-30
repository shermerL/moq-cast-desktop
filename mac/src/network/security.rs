//! Request-path construction and constant-time listener credential checks.

use crate::contract::CLUSTER_PATH_PREFIX;

pub(crate) fn authorized(path: &str, expected: &str) -> bool {
    let Some(presented) = path.strip_prefix(CLUSTER_PATH_PREFIX) else {
        return false;
    };

    !presented.is_empty() && !presented.contains('/') && moq_tokio::mdns::ct_eq(expected, presented)
}

#[cfg(test)]
mod tests {
    use super::authorized;

    #[test]
    fn accepts_only_the_exact_cluster_path_and_credential() {
        assert!(authorized("/.cluster/expected", "expected"));
        assert!(!authorized("/.cluster/wrong", "expected"));
        assert!(!authorized("/.cluster", "expected"));
        assert!(!authorized("/.cluster/", "expected"));
        assert!(!authorized("/.cluster/expected/extra", "expected"));
        assert!(!authorized("/.clusterish/expected", "expected"));
    }
}
