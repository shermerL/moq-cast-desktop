//! Real-time screen player layout and local interaction state.

use eframe::egui::{self, Color32, Rect, Sense, TextureHandle, ViewportCommand};
use moqcast_ui::v2;
use moqcast_ui::{PlayerAudioState, PlayerFrameSample, PlayerInfoLocale, PlayerInfoSnapshot};

use super::{Locale, RemoteAudioPhase, RemoteAudioSnapshot, RemoteVideoSnapshot};

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayerLayout {
    surface: egui::Vec2,
    image: egui::Vec2,
}

fn player_layout(
    source: Option<egui::Vec2>,
    available: egui::Vec2,
    fullscreen: bool,
) -> PlayerLayout {
    let available = egui::vec2(valid_extent(available.x), valid_extent(available.y));
    let source = source
        .filter(|size| valid_size(*size))
        .unwrap_or(FALLBACK_SOURCE);
    let surface =
        moqcast_ui::v2::player_rects(Rect::from_min_size(egui::Pos2::ZERO, available), fullscreen)
            .stage
            .size();
    let scale = (surface.x / source.x).min(surface.y / source.y);
    let image = source * scale;

    PlayerLayout { surface, image }
}

fn valid_extent(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn valid_size(size: egui::Vec2) -> bool {
    size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0
}

#[derive(Default)]
struct FullscreenState {
    actual: bool,
}

impl FullscreenState {
    fn sync(&mut self, actual: bool) {
        self.actual = actual;
    }

    fn active(&self) -> bool {
        self.actual
    }

    fn exit_if_inactive(&self, viewing: bool) -> Option<bool> {
        (self.actual && !viewing).then_some(false)
    }
}

pub(super) enum PlayerMode<'a> {
    Preparing {
        device: &'a str,
        audio: &'a RemoteAudioSnapshot,
    },
    Viewing {
        device: &'a str,
        stopping: bool,
        audio: &'a RemoteAudioSnapshot,
    },
}

#[derive(Clone, Copy)]
pub(super) struct PlayerPlayback<'a> {
    pub(super) generation: u64,
    pub(super) frame: Option<PlayerFrameSample>,
    pub(super) video: &'a RemoteVideoSnapshot,
}

impl PlayerMode<'_> {
    fn device(&self) -> &str {
        match self {
            Self::Preparing { device, .. } | Self::Viewing { device, .. } => device,
        }
    }

    fn audio(&self) -> &RemoteAudioSnapshot {
        match self {
            Self::Preparing { audio, .. } | Self::Viewing { audio, .. } => audio,
        }
    }

    fn viewing(&self) -> bool {
        matches!(self, Self::Viewing { .. })
    }

    fn stopping(&self) -> bool {
        matches!(self, Self::Viewing { stopping: true, .. })
    }
}

pub(super) enum PlayerAction {
    StopWatching,
    SetVolume { generation: u64, percent: u8 },
}

pub(super) struct LivePlayer {
    fullscreen: FullscreenState,
    chrome: v2::WatchChrome,
}

impl Default for LivePlayer {
    fn default() -> Self {
        Self {
            fullscreen: FullscreenState::default(),
            chrome: Default::default(),
        }
    }
}

impl LivePlayer {
    pub(super) fn reconcile_fullscreen(&mut self, context: &egui::Context, viewing: bool) -> bool {
        let actual = context.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if !actual || !viewing {
            self.chrome.leave_fullscreen();
        }
        self.fullscreen.sync(actual);
        if let Some(command) = self.fullscreen.exit_if_inactive(viewing) {
            context.send_viewport_cmd(ViewportCommand::Fullscreen(command));
        }
        actual
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        locale: Locale,
        playback: PlayerPlayback<'_>,
        mode: PlayerMode<'_>,
        texture: Option<&TextureHandle>,
    ) -> Option<PlayerAction> {
        let fullscreen = self.fullscreen.active();
        let available = ui.available_rect_before_wrap();
        let rects = moqcast_ui::v2::player_rects(available, fullscreen);
        let layout = player_layout(
            texture.map(TextureHandle::size_vec2),
            available.size(),
            fullscreen,
        );
        let occupied = if fullscreen {
            rects.stage
        } else {
            rects.stage.union(rects.toolbar)
        };
        ui.allocate_rect(occupied, Sense::hover());
        ui.painter().rect_filled(rects.stage, 0, Color32::BLACK);
        if let Some(texture) = texture {
            let image = Rect::from_center_size(rects.stage.center(), layout.image);
            ui.painter().image(
                texture.id(),
                image,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            let rect = Rect::from_center_size(rects.stage.center(), egui::vec2(28.0, 28.0));
            ui.put(
                rect,
                egui::Spinner::new()
                    .size(28.0)
                    .color(Color32::from_gray(165)),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::ProgressIndicator,
                    true,
                    locale.waiting_for_first_frame(),
                )
            });
        }
        let phase = if mode.stopping() {
            v2::WatchPhase::Stopping
        } else if mode.viewing() && texture.is_some() {
            v2::WatchPhase::Live
        } else {
            v2::WatchPhase::Waiting
        };
        let snapshot = player_info_snapshot(playback, &mode, texture, false);

        self.chrome
            .show(
                ui,
                available,
                fullscreen,
                phase,
                player_info_locale(locale),
                snapshot,
            )
            .map(|cmd| match cmd {
                v2::WatchCommand::Stop => PlayerAction::StopWatching,
                v2::WatchCommand::Volume {
                    generation,
                    percent,
                } => PlayerAction::SetVolume {
                    generation,
                    percent,
                },
            })
    }
}

fn player_info_snapshot<'a>(
    playback: PlayerPlayback<'a>,
    mode: &'a PlayerMode<'a>,
    texture: Option<&TextureHandle>,
    muted: bool,
) -> PlayerInfoSnapshot<'a> {
    PlayerInfoSnapshot {
        generation: playback.generation,
        source: Some(mode.device()),
        frame: playback.frame,
        resolution: texture.map(|texture| {
            let [width, height] = texture.size();
            (width as u32, height as u32)
        }),
        video_codec: playback.video.codec.as_deref(),
        decoder: playback.video.decoder.as_deref(),
        audio_state: player_audio_state(mode.audio().phase, muted),
        audio_codec: mode.audio().codec.as_deref(),
        audio_sample_rate: mode.audio().sample_rate,
        audio_channels: mode.audio().channels,
    }
}

fn player_info_locale(locale: Locale) -> PlayerInfoLocale {
    match locale {
        Locale::Chinese => PlayerInfoLocale::Chinese,
        Locale::English => PlayerInfoLocale::English,
    }
}

fn player_audio_state(phase: RemoteAudioPhase, muted: bool) -> PlayerAudioState {
    match phase {
        RemoteAudioPhase::Idle | RemoteAudioPhase::Pending => PlayerAudioState::Pending,
        RemoteAudioPhase::NoAudio => PlayerAudioState::NoAudio,
        RemoteAudioPhase::Failed => PlayerAudioState::Failed,
        RemoteAudioPhase::TrackSelected
        | RemoteAudioPhase::PcmDecoded
        | RemoteAudioPhase::PcmSubmitted => {
            if muted {
                PlayerAudioState::Muted
            } else {
                PlayerAudioState::Ready
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use super::*;

    fn assert_size(actual: egui::Vec2, expected: egui::Vec2) {
        assert!((actual.x - expected.x).abs() < 0.001);
        assert!((actual.y - expected.y).abs() < 0.001);
    }

    #[test]
    fn landscape_frame_uses_source_aspect_ratio() {
        let layout = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(1000.0, 700.0),
            false,
        );

        assert_size(layout.surface, egui::vec2(1000.0, 648.0));
        assert_size(layout.image, egui::vec2(1000.0, 562.5));
    }

    #[test]
    fn portrait_frame_uses_side_bars_inside_the_stable_stage() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1000.0, 700.0),
            false,
        );

        assert_size(layout.surface, egui::vec2(1000.0, 648.0));
        assert_size(layout.image, egui::vec2(364.5, 648.0));
    }

    #[test]
    fn image_never_exceeds_the_available_region() {
        let layout = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(500.0, 200.0),
            false,
        );

        assert_eq!(layout.surface.x, 500.0);
        assert_eq!(layout.surface.y, 148.0);
        assert_size(layout.image, egui::vec2(263.111_1, 148.0));
    }

    #[test]
    fn missing_or_invalid_source_uses_stable_fallback() {
        let missing = player_layout(None, egui::vec2(800.0, 600.0), false);
        let invalid = player_layout(
            Some(egui::vec2(f32::NAN, 0.0)),
            egui::vec2(800.0, 600.0),
            false,
        );

        assert_eq!(missing, invalid);
        assert_size(missing.surface, egui::vec2(800.0, 548.0));
    }

    #[test]
    fn narrow_window_keeps_a_full_width_stable_stage() {
        let landscape = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(420.0, 400.0),
            false,
        );
        let portrait = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(420.0, 400.0),
            false,
        );

        assert_size(landscape.surface, egui::vec2(420.0, 348.0));
        assert_size(portrait.surface, landscape.surface);
        assert_size(portrait.image, egui::vec2(195.75, 348.0));
    }

    #[test]
    fn volume_is_enabled_only_after_a_playable_audio_track_is_selected() {
        assert_ne!(
            player_audio_state(RemoteAudioPhase::Pending, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(RemoteAudioPhase::TrackSelected, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(RemoteAudioPhase::PcmSubmitted, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(RemoteAudioPhase::NoAudio, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(RemoteAudioPhase::Failed, false),
            PlayerAudioState::Ready
        );
    }

    #[test]
    fn playback_information_distinguishes_audio_availability_from_local_mute() {
        assert_eq!(
            player_audio_state(RemoteAudioPhase::Pending, false),
            PlayerAudioState::Pending
        );
        assert_eq!(
            player_audio_state(RemoteAudioPhase::NoAudio, false),
            PlayerAudioState::NoAudio
        );
        assert_eq!(
            player_audio_state(RemoteAudioPhase::PcmSubmitted, true),
            PlayerAudioState::Muted
        );
        assert_eq!(
            player_audio_state(RemoteAudioPhase::PcmSubmitted, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(RemoteAudioPhase::Failed, false),
            PlayerAudioState::Failed
        );
    }

    #[test]
    fn fullscreen_surface_fills_available_space_and_contains_video() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1200.0, 800.0),
            true,
        );

        assert_size(layout.surface, egui::vec2(1200.0, 800.0));
        assert_size(layout.image, egui::vec2(450.0, 800.0));
    }

    #[test]
    fn preparing_first_frame_and_watching_share_one_player_geometry() {
        let available = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(880.0, 600.0));
        let preparing = moqcast_ui::v2::player_rects(available, false);
        let first_frame = moqcast_ui::v2::player_rects(available, false);
        let watching = moqcast_ui::v2::player_rects(available, false);

        assert_eq!(preparing, first_frame);
        assert_eq!(first_frame, watching);
        assert_eq!(watching.toolbar.top(), watching.stage.bottom());
    }

    #[test]
    fn fullscreen_state_follows_viewport_and_exits_when_viewing_stops() {
        let mut state = FullscreenState::default();

        state.sync(true);
        assert!(state.active());
        assert_eq!(state.exit_if_inactive(true), None);
        assert_eq!(state.exit_if_inactive(false), Some(false));

        state.sync(false);
        assert!(!state.active());
    }
    #[test]
    fn fullscreen_without_frames_stays_visible_then_live_hides_and_wakes() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "test frame",
            egui::ColorImage::filled([2, 2], Color32::BLACK),
            Default::default(),
        );
        let mut player = LivePlayer::default();
        let audio = RemoteAudioSnapshot::default();
        let video = RemoteVideoSnapshot::default();
        let playback = PlayerPlayback {
            generation: 1,
            frame: None,
            video: &video,
        };

        let mut frame = |time, ready: bool, events| {
            let mut input = egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .fullscreen = Some(true);
            let output = ctx.run_ui(input, |ui| {
                assert!(player.reconcile_fullscreen(ui.ctx(), true));
                assert!(
                    player
                        .show(
                            ui,
                            Locale::English,
                            playback,
                            PlayerMode::Viewing {
                                device: "Source",
                                stopping: false,
                                audio: &audio
                            },
                            ready.then_some(&texture)
                        )
                        .is_none()
                );
            });
            let cursor = output.platform_output.cursor_icon;
            output.drop_without_applying_deltas();
            cursor
        };
        assert_ne!(frame(0.0, false, vec![]), egui::CursorIcon::None);
        assert_ne!(frame(9.0, false, vec![]), egui::CursorIcon::None);
        assert_ne!(frame(10.0, true, vec![]), egui::CursorIcon::None);
        assert_ne!(frame(12.49, true, vec![]), egui::CursorIcon::None);
        assert_eq!(frame(12.50, true, vec![]), egui::CursorIcon::None);
        assert_ne!(
            frame(
                13.0,
                true,
                vec![egui::Event::PointerMoved(egui::pos2(50.0, 90.0))]
            ),
            egui::CursorIcon::None
        );
    }
}
