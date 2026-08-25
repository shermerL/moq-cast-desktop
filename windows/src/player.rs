//! Aspect-fit remote screen player for the Windows desktop UI.

use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, Event, Frame, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextureHandle, ViewportCommand,
};

use crate::{
    app::Locale,
    playback::{ViewAudioPhase, ViewPhase, ViewSnapshot},
};

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROLS_HIDE_AFTER: f64 = 2.8;
const CONTROL_HEIGHT: f32 = 50.0;
const CONTROL_BUTTON_HEIGHT: f32 = 34.0;
const CONTROL_BUTTON_WIDTH: f32 = 104.0;
const COMPACT_CONTROL_BUTTON_WIDTH: f32 = 88.0;
const CONTROL_GAP: f32 = 8.0;

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
    let surface = if fullscreen {
        available
    } else {
        egui::vec2(
            available.x,
            (available.x * FALLBACK_SOURCE.y / FALLBACK_SOURCE.x).min(available.y),
        )
    };
    let scale = (surface.x / source.x).min(surface.y / source.y);
    let image = source * scale;

    PlayerLayout { surface, image }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ControlLayout {
    info_width: f32,
    actions_width: f32,
    button_width: f32,
}

fn control_layout(available_width: f32, action_count: usize) -> ControlLayout {
    let available_width = valid_extent(available_width);
    let button_width = if available_width < 520.0 {
        COMPACT_CONTROL_BUTTON_WIDTH
    } else {
        CONTROL_BUTTON_WIDTH
    };
    let actions_width = if action_count == 0 {
        0.0
    } else {
        button_width * action_count as f32 + CONTROL_GAP * action_count.saturating_sub(1) as f32
    };
    let group_gap = if actions_width > 0.0 {
        CONTROL_GAP
    } else {
        0.0
    };

    ControlLayout {
        info_width: (available_width - actions_width - group_gap).max(0.0),
        actions_width: actions_width.min(available_width),
        button_width,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerAction {
    Stop,
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
    controls_last_active: f64,
    layout_signature: Option<PlayerLayoutSignature>,
}

impl Default for LivePlayer {
    fn default() -> Self {
        Self {
            fullscreen: FullscreenState::default(),
            controls_last_active: f64::NEG_INFINITY,
            layout_signature: None,
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
        let (actual, now) =
            context.input(|input| (input.viewport().fullscreen.unwrap_or(false), input.time));
        let was_active = self.fullscreen.active();
        self.fullscreen.sync(actual, viewing);
        let active = self.fullscreen.active();
        if active && !was_active {
            self.controls_last_active = now;
        }
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
        texture: Option<&TextureHandle>,
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
        let (now, activity, escape) = ui.ctx().input(|input| {
            let activity = input.events.iter().any(is_control_activity);
            (input.time, activity, input.key_pressed(egui::Key::Escape))
        });
        if activity || !fullscreen {
            self.controls_last_active = now;
        }
        if escape && fullscreen {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }

        let viewing = view.phase == ViewPhase::Viewing;
        let controls_visible =
            controls_visible(fullscreen, viewing, now, self.controls_last_active);
        if fullscreen && controls_visible && viewing {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            let remaining = (CONTROLS_HIDE_AFTER - (now - self.controls_last_active)).max(0.0);
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(remaining));
        } else if fullscreen && !controls_visible {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        }

        let mut action = None;
        ui.vertical_centered(|ui| {
            let (surface, _) = ui.allocate_exact_size(layout.surface, Sense::hover());
            ui.painter().rect_filled(surface, 0.0, Color32::BLACK);
            if let Some(texture) = texture {
                let image = Rect::from_center_size(surface.center(), layout.image);
                ui.painter().image(
                    texture.id(),
                    image,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.scope_builder(egui::UiBuilder::new().max_rect(surface), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                    });
                });
            }

            if controls_visible {
                let controls = Rect::from_min_max(
                    egui::pos2(
                        surface.left(),
                        (surface.bottom() - CONTROL_HEIGHT).max(surface.top()),
                    ),
                    surface.right_bottom(),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
                    Frame::new()
                        .fill(Color32::from_black_alpha(210))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                            let action_count = 1 + usize::from(viewing);
                            let row_layout = control_layout(ui.available_width(), action_count);
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), CONTROL_BUTTON_HEIGHT),
                                Sense::hover(),
                            );
                            let info = Rect::from_min_size(
                                row.min,
                                egui::vec2(row_layout.info_width, row.height()),
                            );
                            let actions = Rect::from_min_size(
                                egui::pos2(row.right() - row_layout.actions_width, row.top()),
                                egui::vec2(row_layout.actions_width, row.height()),
                            );

                            ui.scope_builder(egui::UiBuilder::new().max_rect(info), |ui| {
                                ui.horizontal(|ui| {
                                    if viewing {
                                        live_badge(ui);
                                    } else {
                                        ui.spinner();
                                        ui.label(
                                            RichText::new(playback_phase(locale, view.phase))
                                                .size(12.0)
                                                .color(Color32::WHITE),
                                        );
                                    }

                                    let resolution = view
                                        .width
                                        .zip(view.height)
                                        .map(|(width, height)| format!("{width} × {height}"))
                                        .unwrap_or_else(|| {
                                            waiting_for_first_frame(locale).to_owned()
                                        });
                                    let details = format!(
                                        "{}  ·  {resolution}  ·  {}",
                                        view.path.as_deref().unwrap_or(remote_screen(locale)),
                                        audio_status(locale, view.audio.phase)
                                    );
                                    let color = if view.audio.phase == ViewAudioPhase::Failed {
                                        Color32::from_rgb(255, 174, 174)
                                    } else {
                                        Color32::from_gray(205)
                                    };
                                    let status = ui.add_sized(
                                        ui.available_size(),
                                        egui::Label::new(
                                            RichText::new(&details).size(11.0).color(color),
                                        )
                                        .truncate(),
                                    );
                                    status.on_hover_text(hover_details(view, &details));
                                });
                            });
                            ui.scope_builder(egui::UiBuilder::new().max_rect(actions), |ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let enabled = view.phase != ViewPhase::Stopping;
                                    if viewing
                                        && player_button(
                                            ui,
                                            if fullscreen {
                                                exit_fullscreen(locale)
                                            } else {
                                                enter_fullscreen(locale)
                                            },
                                            enabled,
                                            false,
                                            row_layout.button_width,
                                        )
                                        .clicked()
                                    {
                                        ui.ctx().send_viewport_cmd(ViewportCommand::Fullscreen(
                                            !fullscreen,
                                        ));
                                    }
                                    if player_button(
                                        ui,
                                        stop_watching(locale),
                                        enabled,
                                        true,
                                        row_layout.button_width,
                                    )
                                    .clicked()
                                    {
                                        if fullscreen {
                                            ui.ctx().send_viewport_cmd(
                                                ViewportCommand::Fullscreen(false),
                                            );
                                        }
                                        action = Some(PlayerAction::Stop);
                                    }
                                });
                            });
                        });
                });
            }
        });
        action
    }
}

fn hover_details(view: &ViewSnapshot, details: &str) -> String {
    let mut hover = details.to_owned();
    for error in [view.audio.last_error.as_deref(), view.last_error.as_deref()]
        .into_iter()
        .flatten()
    {
        hover.push('\n');
        hover.push_str(error);
    }
    hover
}

fn is_control_activity(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerMoved(_)
            | Event::MouseMoved(_)
            | Event::PointerButton { pressed: true, .. }
            | Event::Key { pressed: true, .. }
    )
}

fn controls_visible(fullscreen: bool, viewing: bool, now: f64, last_active: f64) -> bool {
    !fullscreen || !viewing || now - last_active < CONTROLS_HIDE_AFTER
}

fn playback_phase(locale: Locale, phase: ViewPhase) -> &'static str {
    match (locale, phase) {
        (Locale::Chinese, ViewPhase::Preparing) => "正在准备",
        (Locale::English, ViewPhase::Preparing) => "Preparing",
        (Locale::Chinese, ViewPhase::Stopping) => "正在停止",
        (Locale::English, ViewPhase::Stopping) => "Stopping",
        (Locale::Chinese, _) => "播放已结束",
        (Locale::English, _) => "Playback ended",
    }
}

fn audio_status(locale: Locale, phase: ViewAudioPhase) -> &'static str {
    match (locale, phase) {
        (Locale::Chinese, ViewAudioPhase::Idle) => "无音频",
        (Locale::English, ViewAudioPhase::Idle) => "No audio",
        (Locale::Chinese, ViewAudioPhase::Pending) => "音频准备中",
        (Locale::English, ViewAudioPhase::Pending) => "Audio pending",
        (Locale::Chinese, ViewAudioPhase::TrackSelected) => "已选择音轨",
        (Locale::English, ViewAudioPhase::TrackSelected) => "Track selected",
        (Locale::Chinese, ViewAudioPhase::Decoded) => "PCM 已解码",
        (Locale::English, ViewAudioPhase::Decoded) => "PCM decoded",
        (Locale::Chinese, ViewAudioPhase::NotPublished) => "未发布音频",
        (Locale::English, ViewAudioPhase::NotPublished) => "Audio not published",
        (Locale::Chinese, ViewAudioPhase::Writing) => "Sink 已接受写入",
        (Locale::English, ViewAudioPhase::Writing) => "Sink write accepted",
        (Locale::Chinese, ViewAudioPhase::Failed) => "音频不可用",
        (Locale::English, ViewAudioPhase::Failed) => "Audio unavailable",
    }
}

fn remote_screen(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "远端屏幕",
        Locale::English => "Remote screen",
    }
}

fn waiting_for_first_frame(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "等待首帧",
        Locale::English => "Waiting for first frame",
    }
}

fn stop_watching(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "停止观看",
        Locale::English => "Stop watching",
    }
}

fn enter_fullscreen(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "全屏",
        Locale::English => "Fullscreen",
    }
}

fn exit_fullscreen(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "退出全屏",
        Locale::English => "Exit fullscreen",
    }
}

fn live_badge(ui: &mut egui::Ui) {
    Frame::new()
        .fill(Color32::from_rgb(178, 24, 32))
        .stroke(Stroke::new(1.0, Color32::from_rgb(235, 96, 101)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.label(
                RichText::new("LIVE")
                    .size(11.0)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
}

fn player_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    danger: bool,
    width: f32,
) -> egui::Response {
    let color = if danger {
        Color32::from_rgb(255, 174, 174)
    } else {
        Color32::WHITE
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).size(12.0).strong().color(color))
            .fill(Color32::from_black_alpha(120))
            .stroke(Stroke::new(1.0, Color32::from_gray(105)))
            .corner_radius(egui::CornerRadius::same(8))
            .min_size(egui::vec2(width, CONTROL_BUTTON_HEIGHT)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_size(actual: egui::Vec2, expected: egui::Vec2) {
        assert!((actual.x - expected.x).abs() < 0.001);
        assert!((actual.y - expected.y).abs() < 0.001);
    }

    #[test]
    fn normal_player_uses_a_stable_sixteen_by_nine_surface() {
        let landscape = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(1000.0, 700.0),
            false,
        );
        assert_size(landscape.surface, egui::vec2(1000.0, 562.5));
        assert_size(landscape.image, landscape.surface);

        let portrait = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1000.0, 700.0),
            false,
        );
        assert_size(portrait.surface, landscape.surface);
        assert_size(portrait.image, egui::vec2(316.406_25, 562.5));
    }

    #[test]
    fn normal_player_respects_insufficient_available_height() {
        let layout = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(500.0, 200.0),
            false,
        );

        assert_size(layout.surface, egui::vec2(500.0, 200.0));
        assert_size(layout.image, egui::vec2(355.555_57, 200.0));
    }

    #[test]
    fn fullscreen_fills_available_space_and_contains_portrait_video() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1200.0, 800.0),
            true,
        );
        assert_size(layout.surface, egui::vec2(1200.0, 800.0));
        assert_size(layout.image, egui::vec2(450.0, 800.0));
    }

    #[test]
    fn missing_or_invalid_source_uses_the_stable_fallback() {
        let missing = player_layout(None, egui::vec2(800.0, 600.0), false);
        let invalid = player_layout(
            Some(egui::vec2(f32::NAN, 0.0)),
            egui::vec2(800.0, 600.0),
            false,
        );

        assert_eq!(missing, invalid);
        assert_size(missing.surface, egui::vec2(800.0, 450.0));
    }

    #[test]
    fn narrow_controls_reserve_fixed_actions_without_overflow() {
        let layout = control_layout(360.0, 2);

        assert_eq!(layout.button_width, COMPACT_CONTROL_BUTTON_WIDTH);
        assert_eq!(layout.actions_width, 184.0);
        assert_eq!(layout.info_width, 168.0);
        assert_eq!(
            layout.info_width + CONTROL_GAP + layout.actions_width,
            360.0
        );
    }

    #[test]
    fn fullscreen_state_follows_viewport_and_exits_outside_viewing() {
        let mut state = FullscreenState::default();

        state.sync(true, true);
        assert!(state.active());
        assert!(!FullscreenState::should_exit(true, true));
        assert!(FullscreenState::should_exit(true, false));

        state.sync(true, false);
        assert!(!state.active());
        state.sync(false, true);
        assert!(!state.active());
    }

    #[test]
    fn fullscreen_controls_hide_only_while_actively_viewing() {
        assert!(controls_visible(false, true, 10.0, 0.0));
        assert!(controls_visible(true, false, 10.0, 0.0));
        assert!(controls_visible(true, true, 2.79, 0.0));
        assert!(!controls_visible(true, true, 2.8, 0.0));
    }

    #[test]
    fn audio_labels_preserve_processing_evidence_in_both_locales() {
        assert_eq!(
            audio_status(Locale::Chinese, ViewAudioPhase::TrackSelected),
            "已选择音轨"
        );
        assert_eq!(
            audio_status(Locale::Chinese, ViewAudioPhase::Decoded),
            "PCM 已解码"
        );
        assert_eq!(
            audio_status(Locale::Chinese, ViewAudioPhase::Writing),
            "Sink 已接受写入"
        );
        assert_eq!(
            audio_status(Locale::English, ViewAudioPhase::TrackSelected),
            "Track selected"
        );
        assert_eq!(
            audio_status(Locale::English, ViewAudioPhase::Decoded),
            "PCM decoded"
        );
        assert_eq!(
            audio_status(Locale::English, ViewAudioPhase::Writing),
            "Sink write accepted"
        );
    }

    #[test]
    fn layout_log_signature_changes_only_with_observable_player_layout() {
        let signature = PlayerLayoutSignature {
            view_generation: 3,
            fullscreen: false,
            texture_ready: true,
            source: Some(egui::vec2(1080.0, 1920.0)),
            surface: egui::vec2(1000.0, 562.5),
            image: egui::vec2(316.406_25, 562.5),
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
}
