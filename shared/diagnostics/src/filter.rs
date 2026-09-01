use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::Level;

const DETAILED_TARGETS: &[&str] = &[
    "moq_cast_desktop",
    "moqcast_macos",
    "moqcast_windows",
    "moqcast_diagnostics",
    "moq_audio",
    "moq_video",
    "moq_mux",
    "hang",
];
const WARN_CAPPED_TARGETS: &[&str] = &["moq_tokio", "mdns_sd"];

#[derive(Clone)]
pub(crate) struct FilterPolicy {
    detailed: Arc<AtomicBool>,
    private_targets_only: bool,
}

impl FilterPolicy {
    pub(crate) fn new(detailed: bool, private_targets_only: bool) -> Self {
        Self {
            detailed: Arc::new(AtomicBool::new(detailed)),
            private_targets_only,
        }
    }

    pub(crate) fn detailed(&self) -> bool {
        self.detailed.load(Ordering::Relaxed)
    }

    pub(crate) fn set_detailed(&self, detailed: bool) {
        if self.detailed.swap(detailed, Ordering::Relaxed) != detailed {
            tracing::callsite::rebuild_interest_cache();
        }
    }

    pub(crate) fn allows(&self, level: &Level, target: &str) -> bool {
        if self.private_targets_only
            && !matches_target(target, DETAILED_TARGETS)
            && !matches_target(target, WARN_CAPPED_TARGETS)
        {
            return false;
        }
        if matches_target(target, WARN_CAPPED_TARGETS) {
            return severity(level) <= severity(&Level::WARN);
        }
        if severity(level) <= severity(&Level::INFO) {
            return true;
        }
        self.detailed()
            && severity(level) <= severity(&Level::DEBUG)
            && matches_target(target, DETAILED_TARGETS)
    }

    pub(crate) fn description(&self) -> String {
        format!(
            "base=info; detailed={}; moq_tokio=warn; mdns_sd=warn",
            if self.detailed() { "on" } else { "off" }
        )
    }
}

fn matches_target(target: &str, allowed: &[&str]) -> bool {
    allowed.iter().any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with("::"))
    })
}

fn severity(level: &Level) -> u8 {
    match *level {
        Level::ERROR => 0,
        Level::WARN => 1,
        Level::INFO => 2,
        Level::DEBUG => 3,
        Level::TRACE => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_toggle_only_raises_allowed_targets() {
        let policy = FilterPolicy::new(false, false);
        assert!(policy.allows(&Level::INFO, "moq_cast_desktop::runtime"));
        assert!(!policy.allows(&Level::DEBUG, "moq_cast_desktop::runtime"));
        assert!(!policy.allows(&Level::DEBUG, "unrelated_dependency"));

        policy.set_detailed(true);
        assert!(policy.allows(&Level::DEBUG, "moq_cast_desktop::runtime"));
        assert!(!policy.allows(&Level::DEBUG, "unrelated_dependency"));
    }

    #[test]
    fn windows_application_debug_requires_detailed_mode() {
        let policy = FilterPolicy::new(false, false);
        assert!(!policy.allows(&Level::DEBUG, "moqcast_windows::playback"));

        policy.set_detailed(true);
        assert!(policy.allows(&Level::DEBUG, "moqcast_windows::playback"));
    }

    #[test]
    fn macos_application_debug_requires_detailed_mode() {
        let policy = FilterPolicy::new(false, false);
        assert!(!policy.allows(&Level::DEBUG, "moqcast_macos::playback"));

        policy.set_detailed(true);
        assert!(policy.allows(&Level::DEBUG, "moqcast_macos::playback"));
    }

    #[test]
    fn sensitive_targets_remain_warn_capped() {
        let policy = FilterPolicy::new(true, false);
        for target in ["moq_tokio", "moq_tokio::connect", "mdns_sd::service"] {
            assert!(policy.allows(&Level::WARN, target));
            assert!(!policy.allows(&Level::INFO, target));
            assert!(!policy.allows(&Level::DEBUG, target));
        }
    }

    #[test]
    fn private_target_policy_excludes_unknown_dependencies() {
        let policy = FilterPolicy::new(true, true);
        assert!(policy.allows(&Level::INFO, "moqcast_macos::runtime"));
        assert!(policy.allows(&Level::DEBUG, "moq_video::decode"));
        assert!(policy.allows(&Level::WARN, "moq_tokio::connect"));
        assert!(!policy.allows(&Level::ERROR, "unknown_dependency"));
    }
}
