//! Linux remote-audio decode policy.

use std::time::Duration;

pub(super) const REMOTE_AUDIO_LIVE_EDGE_BUDGET: Duration = Duration::from_millis(80);

pub(super) fn remote_video_max_age(has_playable_audio: bool) -> Duration {
    if has_playable_audio {
        REMOTE_AUDIO_LIVE_EDGE_BUDGET
    } else {
        Duration::ZERO
    }
}

#[cfg(target_os = "linux")]
pub(super) fn remote_audio_decode_config() -> moq_audio::decode::Options {
    let mut options = moq_audio::decode::Options::new();
    options.output.format = moq_audio::Format::F32;
    options.max_age = REMOTE_AUDIO_LIVE_EDGE_BUDGET;
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_video_share_80ms_and_video_only_skips_stale_groups() {
        assert_eq!(REMOTE_AUDIO_LIVE_EDGE_BUDGET, Duration::from_millis(80));
        assert_eq!(remote_video_max_age(true), REMOTE_AUDIO_LIVE_EDGE_BUDGET);
        assert_eq!(remote_video_max_age(false), Duration::ZERO);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn remote_audio_decode_config_uses_f32_and_live_edge_budget() {
        let config = remote_audio_decode_config();

        assert_eq!(config.output.format, moq_audio::Format::F32);
        assert_eq!(config.max_age, REMOTE_AUDIO_LIVE_EDGE_BUDGET);
    }
}
