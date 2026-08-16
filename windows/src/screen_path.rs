//! Canonical MoQCast screen broadcast paths.

const PREFIX: &str = "moqcast.screen/";

pub(crate) fn for_peer(peer_id: &str) -> String {
    format!("{PREFIX}{peer_id}")
}

#[cfg(test)]
mod tests {
    use super::for_peer;

    #[test]
    fn builds_the_android_and_linux_compatible_screen_path() {
        assert_eq!(for_peer("peer-a"), "moqcast.screen/peer-a");
    }
}
