//! Runtime-owned diagnostics and frame sampling for a single accepted watch.
use moqcast_diagnostics::media;
use moqcast_playback_stats::PlaybackStats;

pub(crate) struct WatchSession {
    pub(crate) media: media::Session,
    pub(crate) stats: moqcast_playback_stats::WatchSession,
}

impl WatchSession {
    pub(crate) fn new(generation: u64, stats: &PlaybackStats) -> Self {
        Self {
            media: media::Session::watch(generation),
            stats: stats.begin(generation),
        }
    }

    /// Close both records before waiting to deliver the terminal UI event.
    pub(crate) fn finish(self, result: &Result<(), String>, stop_requested: bool) {
        if stop_requested && result.is_ok() {
            self.media.stopped();
        } else {
            self.media.finish(result.is_err());
        }
    }
}
