//! Cross-platform MoQCast protocol and product contract.

#[cfg(test)]
pub(crate) const SERVICE_TYPE: &str = "_moq._udp.local.";
pub(crate) const CLUSTER_PATH_PREFIX: &str = "/.cluster/";
#[cfg(test)]
pub(crate) const SCREEN_PATH_PREFIX: &str = "moqcast.screen/";

pub(crate) fn cluster_path(credential: &str) -> String {
    format!("{CLUSTER_PATH_PREFIX}{credential}")
}

#[cfg(test)]
pub(crate) fn screen_path(peer_id: &str) -> String {
    format!("{SCREEN_PATH_PREFIX}{peer_id}")
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
}
