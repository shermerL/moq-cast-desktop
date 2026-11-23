//! Shared-system remote screen player surface and controls.

use eframe::egui::{self, Color32, Rect, Sense, ViewportCommand};
use moqcast_ui::{PlayerAudioState, PlayerFrameSample, PlayerInfoLocale, PlayerInfoSnapshot};

use super::{Locale, video::Texture};
use crate::playback::{AudioPhase, AudioSnapshot};
use crate::runtime::MediaPhase;
use moqcast_ui::v2;

const FALLBACK_ASPECT: egui::Vec2 = egui::vec2(16.0, 9.0);

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
        .unwrap_or(FALLBACK_ASPECT);
    let surface =
        moqcast_ui::v2::player_rects(Rect::from_min_size(egui::Pos2::ZERO, available), fullscreen)
            .stage
            .size();
    let scale = (surface.x / source.x).min(surface.y / source.y);
    PlayerLayout {
        surface,
        image: source * scale,
    }
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

pub(super) enum PlayerAction {
    Stop,
    SetVolume { generation: u64, percent: u8 },
}

pub(super) struct PlayerView<'a> {
    pub(super) locale: Locale,
    pub(super) generation: u64,
    pub(super) phase: MediaPhase,
    pub(super) frame: Option<PlayerFrameSample>,
    pub(super) video_codec: Option<&'a str>,
    pub(super) decoder: Option<&'a str>,
    pub(super) audio: &'a AudioSnapshot,
    pub(super) device_name: &'a str,
    pub(super) texture: Option<Texture>,
}

#[derive(Default)]
pub(super) struct Player {
    fullscreen: bool,
    chrome: v2::WatchChrome,
}

impl Player {
    pub(super) fn reconcile_fullscreen(&mut self, context: &egui::Context, active: bool) -> bool {
        self.fullscreen = context.input(|input| input.viewport().fullscreen.unwrap_or(false));
        if self.fullscreen && !active {
            self.fullscreen = false;
            context.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        if !self.fullscreen {
            self.chrome.leave_fullscreen();
        }
        self.fullscreen
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, view: PlayerView<'_>) -> Option<PlayerAction> {
        let source = view
            .texture
            .map(|texture| egui::vec2(texture.display.0 as f32, texture.display.1 as f32));
        let available = ui.available_rect_before_wrap();
        let available = Rect::from_min_size(
            available.min,
            egui::vec2(
                valid_extent(available.width()),
                valid_extent(available.height()),
            ),
        );
        let rects = moqcast_ui::v2::player_rects(available, self.fullscreen);
        let layout = player_layout(source, available.size(), self.fullscreen);
        let occupied = if self.fullscreen {
            rects.stage
        } else {
            rects.stage.union(rects.toolbar)
        };
        ui.allocate_rect(occupied, Sense::hover());
        ui.painter().rect_filled(rects.stage, 0, Color32::BLACK);
        paint_surface(ui, rects.stage, layout.image, view.texture);
        paint_status(
            ui,
            rects.stage,
            view.locale,
            if view.phase == MediaPhase::Watching && view.texture.is_none() {
                MediaPhase::PreparingWatch
            } else {
                view.phase
            },
        );
        let phase = match view.phase {
            MediaPhase::Stopping => v2::WatchPhase::Stopping,
            MediaPhase::Watching if view.texture.is_some() => v2::WatchPhase::Live,
            _ => v2::WatchPhase::Waiting,
        };
        self.chrome
            .show(
                ui,
                available,
                self.fullscreen,
                phase,
                player_info_locale(view.locale),
                player_info_snapshot(&view, false),
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
    texture: Option<Texture>,
) {
    let Some(texture) = texture else {
        return;
    };
    let image = Rect::from_center_size(surface.center(), image_size);
    ui.painter().add(texture.mesh(image));
}

fn paint_status(ui: &mut egui::Ui, surface: Rect, locale: Locale, phase: MediaPhase) {
    if phase == MediaPhase::Watching {
        return;
    }
    if phase == MediaPhase::PreparingWatch {
        let rect = Rect::from_center_size(surface.center(), egui::vec2(28.0, 28.0));
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
                match locale {
                    Locale::Chinese => "等待画面",
                    Locale::English => "Waiting for video",
                },
            )
        });
    } else {
        ui.painter().text(
            surface.center(),
            egui::Align2::CENTER_CENTER,
            match locale {
                Locale::Chinese => "正在停止观看",
                Locale::English => "Stopping playback",
            },
            egui::FontId::proportional(16.0),
            Color32::from_gray(190),
        );
    }
}

fn player_info_snapshot<'a>(view: &'a PlayerView<'a>, muted: bool) -> PlayerInfoSnapshot<'a> {
    PlayerInfoSnapshot {
        generation: view.generation,
        source: Some(view.device_name),
        frame: view.frame,
        resolution: view.texture.map(|texture| texture.display),
        video_codec: view.video_codec,
        decoder: view.decoder,
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

fn player_audio_state(phase: AudioPhase, muted: bool) -> PlayerAudioState {
    match phase {
        AudioPhase::Idle | AudioPhase::Pending => PlayerAudioState::Pending,
        AudioPhase::NoAudio => PlayerAudioState::NoAudio,
        AudioPhase::Failed => PlayerAudioState::Failed,
        AudioPhase::TrackSelected | AudioPhase::PcmDecoded | AudioPhase::PcmSubmitted => {
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
    use super::*;

    #[test]
    fn playback_status_text_matches_the_centered_content_block() {
        {
            let (phase, label, text_offset) = (MediaPhase::Stopping, "Stopping playback", 0.0);
            let ctx = egui::Context::default();
            moqcast_ui::v2::install(&ctx);
            let surface = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(600.0, 300.0));
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 700.0),
                    )),
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default()
                        .show(root, |ui| paint_status(ui, surface, Locale::English, phase));
                },
            );
            output.textures_delta.clear();
            let rect = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .expect("playback status label is drawn");
            assert!(
                (rect.center().x - surface.center().x).abs() < 2.0
                    && (rect.center().y - surface.center().y - text_offset).abs() < 2.0,
                "message {rect:?} must match centered {phase:?} block in {surface:?}"
            );
        }
    }

    fn assert_size(actual: egui::Vec2, expected: egui::Vec2) {
        assert!((actual.x - expected.x).abs() < 0.001);
        assert!((actual.y - expected.y).abs() < 0.001);
    }

    #[test]
    fn windowed_surface_reserves_the_attached_toolbar() {
        let layout = player_layout(None, egui::vec2(900.0, 700.0), false);
        assert_size(layout.surface, egui::vec2(900.0, 648.0));
    }

    #[test]
    fn portrait_video_is_contained_in_the_stable_surface() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(900.0, 700.0),
            false,
        );
        assert_size(layout.surface, egui::vec2(900.0, 648.0));
        assert_size(layout.image, egui::vec2(364.5, 648.0));
    }

    #[test]
    fn constrained_height_keeps_stage_and_toolbar_inside_the_page() {
        let layout = player_layout(None, egui::vec2(632.0, 356.0), false);
        assert_size(layout.surface, egui::vec2(632.0, 304.0));
        assert_eq!(
            layout.surface.y + moqcast_ui::v2::PLAYER_TOOLBAR_HEIGHT,
            356.0
        );
    }

    #[test]
    fn preparing_first_frame_and_watching_keep_identical_player_rects() {
        let available = Rect::from_min_size(egui::pos2(24.0, 32.0), egui::vec2(900.0, 700.0));
        let preparing = moqcast_ui::v2::player_rects(available, false);
        let first_frame = moqcast_ui::v2::player_rects(available, false);
        let watching = moqcast_ui::v2::player_rects(available, false);
        assert_eq!(preparing, first_frame);
        assert_eq!(first_frame, watching);
        assert_eq!(preparing.toolbar.top(), preparing.stage.bottom());

        let fullscreen_preparing = moqcast_ui::v2::player_rects(available, true);
        let fullscreen_watching = moqcast_ui::v2::player_rects(available, true);
        assert_eq!(fullscreen_preparing, fullscreen_watching);
        assert_eq!(
            fullscreen_preparing.toolbar.bottom(),
            fullscreen_preparing.stage.bottom() - 16.0
        );
    }

    #[test]
    fn fullscreen_surface_fills_the_available_viewport() {
        let layout = player_layout(None, egui::vec2(1440.0, 900.0), true);
        assert_size(layout.surface, egui::vec2(1440.0, 900.0));
    }

    #[test]
    fn volume_is_available_only_after_selecting_a_playable_audio_track() {
        assert_ne!(
            player_audio_state(AudioPhase::Idle, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(AudioPhase::Pending, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(AudioPhase::NoAudio, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(AudioPhase::TrackSelected, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(AudioPhase::PcmDecoded, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(AudioPhase::PcmSubmitted, false),
            PlayerAudioState::Ready
        );
        assert_ne!(
            player_audio_state(AudioPhase::Failed, false),
            PlayerAudioState::Ready
        );
    }

    #[test]
    fn playback_information_distinguishes_audio_availability_from_local_mute() {
        assert_eq!(
            player_audio_state(AudioPhase::Pending, false),
            PlayerAudioState::Pending
        );
        assert_eq!(
            player_audio_state(AudioPhase::NoAudio, false),
            PlayerAudioState::NoAudio
        );
        assert_eq!(
            player_audio_state(AudioPhase::PcmSubmitted, true),
            PlayerAudioState::Muted
        );
        assert_eq!(
            player_audio_state(AudioPhase::PcmSubmitted, false),
            PlayerAudioState::Ready
        );
        assert_eq!(
            player_audio_state(AudioPhase::Failed, false),
            PlayerAudioState::Failed
        );
    }
}
