//! Linux remote-audio decode policy.

use std::time::Duration;

pub(super) const REMOTE_AUDIO_LIVE_EDGE_BUDGET: Duration = Duration::from_millis(100);

#[cfg(target_os = "linux")]
pub(super) fn remote_audio_decode_config() -> moq_audio::decode::Config {
    let mut config = moq_audio::decode::Config::new();
    config.format = moq_audio::Format::F32;
    config.max_age = REMOTE_AUDIO_LIVE_EDGE_BUDGET;
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_audio_live_edge_budget_is_100ms() {
        assert_eq!(REMOTE_AUDIO_LIVE_EDGE_BUDGET, Duration::from_millis(100));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn remote_audio_decode_config_uses_f32_and_live_edge_budget() {
        let config = remote_audio_decode_config();

        assert_eq!(config.format, moq_audio::Format::F32);
        assert_eq!(config.max_age, REMOTE_AUDIO_LIVE_EDGE_BUDGET);
    }
}
