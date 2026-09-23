//! Shared-system remote screen player surface and controls.

use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, TextureHandle, ViewportCommand};
use moqcast_ui::{
    ButtonSpec, COLORS, ControlRole, IconButtonSpec, PLAYER_VOLUME_CONTROL_WIDTH, PlayerAudioState,
    PlayerFrameSample, PlayerInfoLocale, PlayerInfoSnapshot, PlayerInfoState, PlayerVolumeState,
    Size, TypographyRole, control_button, player_icon_button, player_info_control, player_rects,
    player_stage_at, player_toolbar_at, player_volume_control, typography,
};

use super::Locale;
use crate::playback::{AudioPhase, AudioSnapshot};
use crate::runtime::MediaPhase;

const FALLBACK_ASPECT: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROL_BUTTON_WIDTH: f32 = 108.0;
const COMPACT_CONTROL_BUTTON_WIDTH: f32 = 92.0;
const CONTROL_GAP: f32 = Size::PLAYER_TOOLBAR_ITEM_SPACING;

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayerLayout {
    surface: egui::Vec2,
    image: egui::Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ControlLayout {
    info_width: f32,
    actions_width: f32,
    button_width: f32,
}

fn control_layout(available_width: f32) -> ControlLayout {
    let available_width = valid_extent(available_width);
    let button_width = if available_width < 520.0 {
        COMPACT_CONTROL_BUTTON_WIDTH
    } else {
        CONTROL_BUTTON_WIDTH
    };
    let actions_width = button_width
        + CONTROL_GAP
        + Size::CONTROL
        + CONTROL_GAP
        + Size::CONTROL
        + CONTROL_GAP
        + PLAYER_VOLUME_CONTROL_WIDTH;
    ControlLayout {
        info_width: (available_width - actions_width - CONTROL_GAP).max(0.0),
        actions_width: actions_width.min(available_width),
        button_width,
    }
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
    let surface = player_rects(Rect::from_min_size(egui::Pos2::ZERO, available), fullscreen)
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
    pub(super) texture: Option<(&'a TextureHandle, (u32, u32))>,
}

#[derive(Default)]
pub(super) struct Player {
    fullscreen: bool,
    info: PlayerInfoState,
    volume: PlayerVolumeState,
}

impl Player {
    pub(super) fn reconcile_fullscreen(&mut self, context: &egui::Context, active: bool) -> bool {
        self.fullscreen = context.input(|input| input.viewport().fullscreen.unwrap_or(false));
        if self.fullscreen && !active {
            self.fullscreen = false;
            context.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        self.fullscreen
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, view: PlayerView<'_>) -> Option<PlayerAction> {
        self.info.sync_generation(view.generation);
        self.volume.sync_generation(view.generation);
        let now = ui.input(|input| input.time);
        if self.fullscreen && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Fullscreen(false));
            self.fullscreen = false;
        }
        let source = view
            .texture
            .map(|(_, (width, height))| egui::vec2(width as f32, height as f32));
        let available = ui.available_rect_before_wrap();
        let available = Rect::from_min_size(
            available.min,
            egui::vec2(
                valid_extent(available.width()),
                valid_extent(available.height()),
            ),
        );
        let rects = player_rects(available, self.fullscreen);
        let layout = player_layout(source, available.size(), self.fullscreen);
        let occupied = if self.fullscreen {
            rects.stage
        } else {
            rects.stage.union(rects.toolbar)
        };
        ui.allocate_rect(occupied, Sense::hover());
        player_stage_at(ui, rects.stage, |_| ());
        paint_surface(ui, rects.stage, layout.image, view.texture);
        paint_status(ui, rects.stage, view.locale, view.phase);
        show_toolbar(
            ui,
            rects.toolbar,
            &view,
            self.fullscreen,
            now,
            ToolbarState {
                info: &mut self.info,
                volume: &mut self.volume,
            },
        )
    }
}

fn paint_surface(
    ui: &mut egui::Ui,
    surface: Rect,
    image_size: egui::Vec2,
    texture: Option<(&TextureHandle, (u32, u32))>,
) {
    let Some((texture, _)) = texture else {
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

fn paint_status(ui: &mut egui::Ui, surface: Rect, locale: Locale, phase: MediaPhase) {
    if phase == MediaPhase::Watching {
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(surface));
    child.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            if phase == MediaPhase::PreparingWatch {
                ui.spinner();
            }
            ui.label(typography(
                match (locale, phase) {
                    (Locale::Chinese, MediaPhase::Stopping) => "正在停止观看",
                    (Locale::English, MediaPhase::Stopping) => "Stopping playback",
                    (Locale::Chinese, _) => "正在准备画面",
                    (Locale::English, _) => "Preparing video",
                },
                TypographyRole::Section,
                COLORS.player_text.into(),
            ));
            ui.label(typography(
                match locale {
                    Locale::Chinese => "附近连接保持可用。",
                    Locale::English => "The Nearby connection remains available.",
                },
                TypographyRole::Meta,
                COLORS.player_muted.into(),
            ));
        });
    });
}

fn show_toolbar(
    ui: &mut egui::Ui,
    toolbar: Rect,
    view: &PlayerView<'_>,
    fullscreen: bool,
    now: f64,
    state: ToolbarState<'_>,
) -> Option<PlayerAction> {
    let ToolbarState { info, volume } = state;
    let mut action = None;
    player_toolbar_at(ui, toolbar, |ui| {
        let row_layout = control_layout(ui.available_width());
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), Size::CONTROL),
            Sense::hover(),
        );
        let info_rect =
            Rect::from_min_size(row.min, egui::vec2(row_layout.info_width, row.height()));
        let actions = Rect::from_min_size(
            egui::pos2(row.right() - row_layout.actions_width, row.top()),
            egui::vec2(row_layout.actions_width, row.height()),
        );
        let mut info_ui = ui.new_child(egui::UiBuilder::new().max_rect(info_rect));
        info_ui.horizontal(|ui| {
            if view.phase == MediaPhase::Watching {
                live_badge(ui);
            } else {
                ui.label(typography(
                    match (view.locale, view.phase) {
                        (Locale::Chinese, MediaPhase::Stopping) => "正在停止",
                        (Locale::English, MediaPhase::Stopping) => "Stopping",
                        (Locale::Chinese, _) => "正在准备",
                        (Locale::English, _) => "Preparing",
                    },
                    TypographyRole::Meta,
                    COLORS.player_muted.into(),
                ));
            }
        });
        let mut actions_ui = ui.new_child(egui::UiBuilder::new().max_rect(actions));
        actions_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let enabled = view.phase != MediaPhase::Stopping;
            if player_icon_button(
                ui,
                IconButtonSpec::player(
                    "⛶",
                    match (view.locale, fullscreen) {
                        (Locale::Chinese, true) => "退出全屏",
                        (Locale::Chinese, false) => "全屏",
                        (Locale::English, true) => "Exit fullscreen",
                        (Locale::English, false) => "Fullscreen",
                    },
                )
                .enabled(enabled),
            )
            .clicked()
            {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
            }
            if control_button(
                ui,
                ButtonSpec::new(
                    match view.locale {
                        Locale::Chinese => "停止观看",
                        Locale::English => "Stop watching",
                    },
                    ControlRole::PlayerIcon,
                )
                .enabled(enabled)
                .min_width(row_layout.button_width),
            )
            .clicked()
            {
                if fullscreen {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                action = Some(PlayerAction::Stop);
            }
            let _ = player_info_control(
                ui,
                info,
                player_info_locale(view.locale),
                now,
                player_info_snapshot(view, volume.muted()),
                enabled,
            );
            let volume_response = player_volume_control(
                ui,
                volume,
                audio_playable(view.audio.phase) && enabled,
                match view.locale {
                    Locale::Chinese => "静音",
                    Locale::English => "Mute",
                },
                match view.locale {
                    Locale::Chinese => "取消静音",
                    Locale::English => "Unmute",
                },
                match view.locale {
                    Locale::Chinese => "播放音量",
                    Locale::English => "Playback volume",
                },
                match view.locale {
                    Locale::Chinese => "当前没有可播放音频",
                    Locale::English => "No playable audio",
                },
            );
            if let Some(percent) = volume_response.changed_percent() {
                action = Some(PlayerAction::SetVolume {
                    generation: view.generation,
                    percent,
                });
            }
        });
    });
    action
}

struct ToolbarState<'a> {
    info: &'a mut PlayerInfoState,
    volume: &'a mut PlayerVolumeState,
}

fn player_info_snapshot<'a>(view: &'a PlayerView<'a>, muted: bool) -> PlayerInfoSnapshot<'a> {
    PlayerInfoSnapshot {
        generation: view.generation,
        source: Some(view.device_name),
        frame: view.frame,
        resolution: view.texture.map(|(_, display)| display),
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

fn audio_playable(phase: AudioPhase) -> bool {
    matches!(
        phase,
        AudioPhase::TrackSelected | AudioPhase::PcmDecoded | AudioPhase::PcmSubmitted
    )
}

fn live_badge(ui: &mut egui::Ui) {
    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(dot.center(), 4.0, COLORS.live);
    ui.label(typography("LIVE", TypographyRole::Meta, COLORS.live.into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_size(actual: egui::Vec2, expected: egui::Vec2) {
        assert!((actual.x - expected.x).abs() < 0.001);
        assert!((actual.y - expected.y).abs() < 0.001);
    }

    #[test]
    fn windowed_surface_reserves_the_attached_toolbar() {
        let layout = player_layout(None, egui::vec2(900.0, 700.0), false);
        assert_size(layout.surface, egui::vec2(880.0, 495.0));
    }

    #[test]
    fn portrait_video_is_contained_in_the_stable_surface() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(900.0, 700.0),
            false,
        );
        assert_size(layout.surface, egui::vec2(880.0, 495.0));
        assert_size(layout.image, egui::vec2(278.4375, 495.0));
    }

    #[test]
    fn constrained_height_keeps_stage_and_toolbar_inside_the_page() {
        let layout = player_layout(None, egui::vec2(632.0, 356.0), false);
        assert_size(layout.surface, egui::vec2(540.444_46, 304.0));
        assert_eq!(layout.surface.y + Size::PLAYER_TOOLBAR, 356.0);
    }

    #[test]
    fn preparing_first_frame_and_watching_keep_identical_player_rects() {
        let available = Rect::from_min_size(egui::pos2(24.0, 32.0), egui::vec2(900.0, 700.0));
        let preparing = player_rects(available, false);
        let first_frame = player_rects(available, false);
        let watching = player_rects(available, false);
        assert_eq!(preparing, first_frame);
        assert_eq!(first_frame, watching);
        assert_eq!(preparing.toolbar.top(), preparing.stage.bottom());

        let fullscreen_preparing = player_rects(available, true);
        let fullscreen_watching = player_rects(available, true);
        assert_eq!(fullscreen_preparing, fullscreen_watching);
        assert_eq!(
            fullscreen_preparing.toolbar.bottom(),
            fullscreen_preparing.stage.bottom()
        );
    }

    #[test]
    fn fullscreen_surface_fills_the_available_viewport() {
        let layout = player_layout(None, egui::vec2(1440.0, 900.0), true);
        assert_size(layout.surface, egui::vec2(1440.0, 900.0));
    }

    #[test]
    fn compact_toolbar_reserves_volume_and_actions() {
        let layout = control_layout(360.0);
        assert_eq!(layout.actions_width, 328.0);
        assert_eq!(layout.info_width, 24.0);
        assert_eq!(layout.button_width, COMPACT_CONTROL_BUTTON_WIDTH);
    }

    #[test]
    fn volume_is_available_only_after_selecting_a_playable_audio_track() {
        assert!(!audio_playable(AudioPhase::Idle));
        assert!(!audio_playable(AudioPhase::Pending));
        assert!(!audio_playable(AudioPhase::NoAudio));
        assert!(audio_playable(AudioPhase::TrackSelected));
        assert!(audio_playable(AudioPhase::PcmDecoded));
        assert!(audio_playable(AudioPhase::PcmSubmitted));
        assert!(!audio_playable(AudioPhase::Failed));
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
