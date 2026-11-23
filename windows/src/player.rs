//! Aspect-fit remote screen player for the Windows desktop UI.

use eframe::egui::{self, Color32, Rect, Sense, TextureHandle, ViewportCommand};
use moqcast_ui::{PlayerAudioState, PlayerFrameSample, PlayerInfoLocale, PlayerInfoSnapshot};

use moqcast_ui::v2;

use crate::{
    app::Locale,
    playback::{PlaybackFrameIdentity, ViewAudioPhase, ViewPhase, ViewSnapshot},
};

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayerLayout {
    surface: egui::Vec2,
    image: egui::Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayerLayoutSignature {
    view_generation: u64,
    fullscreen: bool,
    texture_ready: bool,
    source: Option<egui::Vec2>,
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
    let surface = v2::player_rects(Rect::from_min_size(egui::Pos2::ZERO, available), fullscreen)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerAction {
    Stop,
    SetVolume { generation: u64, percent: u8 },
}

#[derive(Default)]
struct FullscreenState {
    active: bool,
}

impl FullscreenState {
    fn sync(&mut self, actual: bool, viewing: bool) {
        self.active = actual && viewing;
    }

    fn active(&self) -> bool {
        self.active
    }

    fn should_exit(actual: bool, viewing: bool) -> bool {
        actual && !viewing
    }
}

pub(crate) struct LivePlayer {
    fullscreen: FullscreenState,
    layout_signature: Option<PlayerLayoutSignature>,
    chrome: v2::WatchChrome,
}

impl Default for LivePlayer {
    fn default() -> Self {
        Self {
            fullscreen: FullscreenState::default(),
            layout_signature: None,
            chrome: Default::default(),
        }
    }
}

impl LivePlayer {
    fn update_layout_signature(&mut self, signature: PlayerLayoutSignature) -> bool {
        if self.layout_signature == Some(signature) {
            return false;
        }
        self.layout_signature = Some(signature);
        true
    }

    pub(crate) fn reconcile_fullscreen(&mut self, context: &egui::Context, viewing: bool) -> bool {
        let actual = context.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if !actual || !viewing {
            self.chrome.leave_fullscreen();
        }
        self.fullscreen.sync(actual, viewing);
        if FullscreenState::should_exit(actual, viewing) {
            context.set_cursor_icon(egui::CursorIcon::Default);
            context.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        actual
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        locale: Locale,
        view: &ViewSnapshot,
        source_label: Option<&str>,
        texture: Option<&TextureHandle>,
        frame_identity: Option<PlaybackFrameIdentity>,
    ) -> Option<PlayerAction> {
        let fullscreen = self.fullscreen.active();
        let available = ui.available_size();
        let source = view
            .width
            .zip(view.height)
            .map(|(width, height)| egui::vec2(width as f32, height as f32));
        let layout = player_layout(source, available, fullscreen);
        let signature = PlayerLayoutSignature {
            view_generation: view.generation,
            fullscreen,
            texture_ready: texture.is_some(),
            source,
            surface: layout.surface,
            image: layout.image,
        };
        if self.update_layout_signature(signature) {
            tracing::info!(
                view_generation = signature.view_generation,
                fullscreen = signature.fullscreen,
                texture_ready = signature.texture_ready,
                source_width = %signature.source.map_or(0.0, |size| size.x),
                source_height = %signature.source.map_or(0.0, |size| size.y),
                surface_width = %signature.surface.x,
                surface_height = %signature.surface.y,
                image_width = %signature.image.x,
                image_height = %signature.image.y,
                "remote player layout changed"
            );
        }
        let available = ui.available_rect_before_wrap();
        let rects = v2::player_rects(available, fullscreen);
        ui.allocate_rect(available, Sense::hover());
        ui.painter().rect_filled(rects.stage, 0.0, Color32::BLACK);
        paint_surface(ui, rects.stage, layout.image, texture);
        if texture.is_none() {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_center_size(
                    rects.stage.center(),
                    egui::vec2(28.0, 28.0),
                )),
                |ui| {
                    let response = ui.add(egui::Spinner::new().size(28.0));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::ProgressIndicator,
                            true,
                            waiting_for_first_frame(locale),
                        )
                    });
                },
            );
        }
        let phase = presentation(view.phase, texture.is_some());
        let mut snapshot = player_info_snapshot(view, frame_identity, false);
        snapshot.source = source_label;
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
                v2::WatchCommand::Stop => PlayerAction::Stop,
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

fn paint_surface(
    ui: &mut egui::Ui,
    surface: Rect,
    image_size: egui::Vec2,
    texture: Option<&TextureHandle>,
) {
    let Some(texture) = texture else {
        return;
    };
    let image = Rect::from_center_size(surface.center(), image_size);
    ui.painter().image(
        texture.id(),
        image,
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn player_info_snapshot(
    view: &ViewSnapshot,
    frame_identity: Option<PlaybackFrameIdentity>,
    muted: bool,
) -> PlayerInfoSnapshot<'_> {
    PlayerInfoSnapshot {
        generation: view.generation,
        source: None,
        frame: frame_identity.map(|identity| PlayerFrameSample {
            view_generation: identity.view_generation,
            decoder_generation: identity.decoder_generation,
            sequence: identity.sequence,
        }),
        resolution: view.width.zip(view.height),
        video_codec: view.video_codec.as_deref(),
        decoder: view.decoder.as_deref(),
        audio_state: player_audio_state(view.audio.phase, muted),
        audio_codec: view.audio.codec.as_deref(),
        audio_sample_rate: view.audio.sample_rate,
        audio_channels: view.audio.channels,
    }
}

fn player_info_locale(locale: Locale) -> PlayerInfoLocale {
    match locale {
        Locale::Chinese => PlayerInfoLocale::Chinese,
        Locale::English => PlayerInfoLocale::English,
    }
}

fn player_audio_state(phase: ViewAudioPhase, muted: bool) -> PlayerAudioState {
    match phase {
        ViewAudioPhase::Idle | ViewAudioPhase::Pending => PlayerAudioState::Pending,
        ViewAudioPhase::NotPublished => PlayerAudioState::NoAudio,
        ViewAudioPhase::Failed => PlayerAudioState::Failed,
        ViewAudioPhase::TrackSelected
        | ViewAudioPhase::Decoded
        | ViewAudioPhase::Writing
        | ViewAudioPhase::CallbackConsumed => {
            if muted {
                PlayerAudioState::Muted
            } else {
                PlayerAudioState::Ready
            }
        }
    }
}

fn presentation(phase: ViewPhase, texture_ready: bool) -> v2::WatchPhase {
    match phase {
        ViewPhase::Viewing if texture_ready => v2::WatchPhase::Live,
        ViewPhase::Stopping => v2::WatchPhase::Stopping,
        _ => v2::WatchPhase::Waiting,
    }
}

fn waiting_for_first_frame(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "等待首帧",
        Locale::English => "Waiting for first frame",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_size(actual: egui::Vec2, expected: egui::Vec2) {
        assert!((actual.x - expected.x).abs() < 0.001);
        assert!((actual.y - expected.y).abs() < 0.001);
    }

    #[test]
    fn landscape_and_portrait_frames_remain_inside_the_player() {
        let landscape = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(1000.0, 752.0),
            false,
        );
        assert_size(landscape.surface, egui::vec2(1000.0, 700.0));
        assert_size(landscape.image, egui::vec2(1000.0, 562.5));

        let portrait = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1000.0, 752.0),
            false,
        );
        assert_size(portrait.surface, egui::vec2(1000.0, 700.0));
        assert_size(portrait.image, egui::vec2(393.75, 700.0));
    }

    #[test]
    fn fullscreen_fills_the_surface_and_letterboxes_the_image() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1200.0, 800.0),
            true,
        );
        assert_size(layout.surface, egui::vec2(1200.0, 800.0));
        assert_size(layout.image, egui::vec2(450.0, 800.0));
    }

    #[test]
    fn invalid_source_uses_a_stable_fallback() {
        assert_eq!(
            player_layout(None, egui::vec2(800.0, 652.0), false),
            player_layout(
                Some(egui::vec2(f32::NAN, 0.0)),
                egui::vec2(800.0, 652.0),
                false,
            )
        );
    }

    #[test]
    fn layout_log_signature_changes_only_with_observable_player_layout() {
        let signature = PlayerLayoutSignature {
            view_generation: 3,
            fullscreen: false,
            texture_ready: true,
            source: Some(egui::vec2(1080.0, 1920.0)),
            surface: egui::vec2(1000.0, 700.0),
            image: egui::vec2(393.75, 700.0),
        };
        let mut player = LivePlayer::default();

        assert!(player.update_layout_signature(signature));
        assert!(!player.update_layout_signature(signature));
        assert!(player.update_layout_signature(PlayerLayoutSignature {
            fullscreen: true,
            ..signature
        }));
        assert!(player.update_layout_signature(PlayerLayoutSignature {
            view_generation: 4,
            surface: egui::vec2(1200.0, 800.0),
            image: egui::vec2(450.0, 800.0),
            ..signature
        }));
    }

    #[test]
    fn normal_player_reserves_a_fixed_toolbar_below_the_stage() {
        let layout = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(1000.0, 752.0),
            false,
        );
        assert_size(layout.surface, egui::vec2(1000.0, 700.0));
        assert_size(layout.image, egui::vec2(1000.0, 562.5));
        assert_eq!(layout.surface.y + 52.0, 752.0);
    }

    #[test]
    fn player_presentation_uses_texture_readiness_without_claiming_audio_output() {
        assert_eq!(
            presentation(ViewPhase::Preparing, false),
            v2::WatchPhase::Waiting
        );
        assert_eq!(
            presentation(ViewPhase::Viewing, false),
            v2::WatchPhase::Waiting
        );
        assert_eq!(presentation(ViewPhase::Viewing, true), v2::WatchPhase::Live);
        assert_eq!(
            player_audio_state(ViewAudioPhase::CallbackConsumed, false),
            PlayerAudioState::Ready
        );
    }

    #[test]
    fn volume_is_enabled_only_after_a_playable_audio_track_is_selected() {
        assert_ne!(
            player_audio_state(ViewAudioPhase::Pending, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(ViewAudioPhase::TrackSelected, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(ViewAudioPhase::CallbackConsumed, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(ViewAudioPhase::NotPublished, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(ViewAudioPhase::Failed, false),
            PlayerAudioState::Ready
        );
    }

    #[test]
    fn playback_information_distinguishes_audio_availability_from_local_mute() {
        assert_eq!(
            player_audio_state(ViewAudioPhase::Pending, false),
            PlayerAudioState::Pending
        );
        assert_eq!(
            player_audio_state(ViewAudioPhase::NotPublished, false),
            PlayerAudioState::NoAudio
        );
        assert_eq!(
            player_audio_state(ViewAudioPhase::CallbackConsumed, true),
            PlayerAudioState::Muted
        );
        assert_eq!(
            player_audio_state(ViewAudioPhase::CallbackConsumed, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(ViewAudioPhase::Failed, false),
            PlayerAudioState::Failed
        );
    }

    #[test]
    fn leaving_viewing_exits_fullscreen_state() {
        let mut state = FullscreenState::default();
        state.sync(true, true);
        assert!(state.active());
        assert!(FullscreenState::should_exit(true, false));

        state.sync(true, false);
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
        let view = ViewSnapshot {
            phase: ViewPhase::Viewing,
            generation: 1,
            width: Some(2),
            height: Some(2),
            ..Default::default()
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
                            &view,
                            Some("Source"),
                            ready.then_some(&texture),
                            None
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
