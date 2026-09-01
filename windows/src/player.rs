//! Aspect-fit remote screen player for the Windows desktop UI.

use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, Event, Layout, Rect, Sense, TextureHandle, ViewportCommand,
};
use moqcast_ui::{
    ButtonSpec, COLORS, ControlRole, IconButtonSpec, Size, TypographyRole, control_button,
    player_icon_button, player_stage, player_toolbar, typography,
};

use crate::{
    app::Locale,
    playback::{ViewAudioPhase, ViewPhase, ViewSnapshot},
};

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROLS_HIDE_AFTER: f64 = 2.8;
/// Height reserved for the normal-window player toolbar.
pub(crate) const TOOLBAR_HEIGHT: f32 = Size::PLAYER_TOOLBAR;
const CONTROL_BUTTON_WIDTH: f32 = 108.0;
const COMPACT_CONTROL_BUTTON_WIDTH: f32 = 92.0;
const CONTROL_GAP: f32 = Size::PLAYER_TOOLBAR_ITEM_SPACING;

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
        let available_stage_height = (available.y - TOOLBAR_HEIGHT).max(1.0);
        let width = available
            .x
            .min(Size::WATCH_MAX)
            .min(available_stage_height * Size::PLAYER_ASPECT[0] / Size::PLAYER_ASPECT[1]);
        egui::vec2(
            width,
            width * Size::PLAYER_ASPECT[1] / Size::PLAYER_ASPECT[0],
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
        if self.fullscreen.active() && !was_active {
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
            (
                input.time,
                input.events.iter().any(is_control_activity),
                input.key_pressed(egui::Key::Escape),
            )
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
            let surface = if fullscreen {
                let (surface, _) = ui.allocate_exact_size(layout.surface, Sense::hover());
                ui.painter().rect_filled(surface, 0.0, COLORS.player);
                surface
            } else {
                player_stage(ui, |ui| ui.max_rect())
            };
            paint_surface(ui, surface, layout.image, texture);

            if texture.is_none() {
                ui.scope_builder(egui::UiBuilder::new().max_rect(surface), |ui| {
                    ui.centered_and_justified(|ui| ui.spinner());
                });
            }

            if controls_visible {
                if fullscreen {
                    let controls = Rect::from_min_max(
                        egui::pos2(
                            surface.left(),
                            (surface.bottom() - TOOLBAR_HEIGHT).max(surface.top()),
                        ),
                        surface.right_bottom(),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
                        player_toolbar(ui, |ui| {
                            show_controls(ui, locale, view, texture.is_some(), true, &mut action);
                        });
                    });
                } else {
                    player_toolbar(ui, |ui| {
                        show_controls(ui, locale, view, texture.is_some(), false, &mut action);
                    });
                }
            }
        });
        action
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

fn show_controls(
    ui: &mut egui::Ui,
    locale: Locale,
    view: &ViewSnapshot,
    texture_ready: bool,
    fullscreen: bool,
    action: &mut Option<PlayerAction>,
) {
    let presentation = presentation(view.phase, texture_ready);
    let action_count = 1 + usize::from(presentation == PlayerPresentation::Live);
    let row_layout = control_layout(ui.available_width(), action_count);
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), Size::CONTROL),
        Sense::hover(),
    );
    let info = Rect::from_min_size(row.min, egui::vec2(row_layout.info_width, row.height()));
    let actions = Rect::from_min_size(
        egui::pos2(row.right() - row_layout.actions_width, row.top()),
        egui::vec2(row_layout.actions_width, row.height()),
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(info), |ui| {
        ui.horizontal(|ui| {
            if presentation == PlayerPresentation::Live {
                live_badge(ui);
            } else {
                ui.spinner();
                ui.label(typography(
                    presentation_label(locale, presentation),
                    TypographyRole::Meta,
                    COLORS.player_text.into(),
                ));
            }
            let resolution = view
                .width
                .zip(view.height)
                .map(|(width, height)| format!("{width} × {height}"))
                .unwrap_or_else(|| waiting_for_first_frame(locale).to_owned());
            let details = format!(
                "{}  ·  {resolution}  ·  {}",
                remote_screen(locale),
                audio_status(locale, view.audio.phase)
            );
            let color = if view.audio.phase == ViewAudioPhase::Failed {
                COLORS.danger
            } else {
                COLORS.player_muted
            };
            ui.add_sized(
                ui.available_size(),
                egui::Label::new(typography(details, TypographyRole::Meta, color.into()))
                    .truncate(),
            );
        });
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(actions), |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let enabled = view.phase != ViewPhase::Stopping;
            if presentation == PlayerPresentation::Live
                && player_icon_button(
                    ui,
                    IconButtonSpec::player(
                        "⛶",
                        if fullscreen {
                            exit_fullscreen(locale)
                        } else {
                            enter_fullscreen(locale)
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
                ButtonSpec::new(stop_watching(locale), ControlRole::PlayerIcon)
                    .enabled(enabled)
                    .min_width(row_layout.button_width),
            )
            .clicked()
            {
                if fullscreen {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                *action = Some(PlayerAction::Stop);
            }
        });
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayerPresentation {
    Preparing,
    Buffering,
    Live,
    Stopping,
    Ended,
}

fn presentation(phase: ViewPhase, texture_ready: bool) -> PlayerPresentation {
    match phase {
        ViewPhase::Preparing => PlayerPresentation::Preparing,
        ViewPhase::Viewing if texture_ready => PlayerPresentation::Live,
        ViewPhase::Viewing => PlayerPresentation::Buffering,
        ViewPhase::Stopping => PlayerPresentation::Stopping,
        ViewPhase::Idle | ViewPhase::Failed => PlayerPresentation::Ended,
    }
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

fn presentation_label(locale: Locale, phase: PlayerPresentation) -> &'static str {
    match (locale, phase) {
        (Locale::Chinese, PlayerPresentation::Preparing) => "正在准备",
        (Locale::English, PlayerPresentation::Preparing) => "Preparing",
        (Locale::Chinese, PlayerPresentation::Buffering) => "正在缓冲",
        (Locale::English, PlayerPresentation::Buffering) => "Buffering",
        (Locale::Chinese, PlayerPresentation::Stopping) => "正在停止",
        (Locale::English, PlayerPresentation::Stopping) => "Stopping",
        (Locale::Chinese, PlayerPresentation::Ended) => "播放已结束",
        (Locale::English, PlayerPresentation::Ended) => "Playback ended",
        (_, PlayerPresentation::Live) => "LIVE",
    }
}

fn audio_status(locale: Locale, phase: ViewAudioPhase) -> &'static str {
    match (locale, phase) {
        (Locale::Chinese, ViewAudioPhase::Idle | ViewAudioPhase::NotPublished) => "无音频",
        (Locale::English, ViewAudioPhase::Idle | ViewAudioPhase::NotPublished) => "No audio",
        (
            Locale::Chinese,
            ViewAudioPhase::Pending
            | ViewAudioPhase::TrackSelected
            | ViewAudioPhase::Decoded
            | ViewAudioPhase::Writing,
        ) => "音频…",
        (
            Locale::English,
            ViewAudioPhase::Pending
            | ViewAudioPhase::TrackSelected
            | ViewAudioPhase::Decoded
            | ViewAudioPhase::Writing,
        ) => "Audio...",
        (Locale::Chinese, ViewAudioPhase::CallbackConsumed) => "音频",
        (Locale::English, ViewAudioPhase::CallbackConsumed) => "Audio",
        (Locale::Chinese, ViewAudioPhase::Failed) => "音频错误",
        (Locale::English, ViewAudioPhase::Failed) => "Audio error",
    }
}

fn remote_screen(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "附近屏幕",
        Locale::English => "Nearby screen",
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
    ui.label(typography(
        "LIVE",
        TypographyRole::Meta,
        COLORS.brand.into(),
    ));
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
        assert_size(landscape.surface, egui::vec2(960.0, 540.0));
        assert_size(landscape.image, egui::vec2(960.0, 540.0));

        let portrait = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1000.0, 752.0),
            false,
        );
        assert_size(portrait.surface, egui::vec2(960.0, 540.0));
        assert_size(portrait.image, egui::vec2(303.75, 540.0));
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
        assert_size(layout.surface, egui::vec2(960.0, 540.0));
        assert_size(layout.image, egui::vec2(960.0, 540.0));
        assert!((layout.surface.x / layout.surface.y - 16.0 / 9.0).abs() < 0.001);
    }

    #[test]
    fn player_presentation_uses_texture_readiness_without_claiming_audio_output() {
        assert_eq!(
            presentation(ViewPhase::Preparing, false),
            PlayerPresentation::Preparing
        );
        assert_eq!(
            presentation(ViewPhase::Viewing, false),
            PlayerPresentation::Buffering
        );
        assert_eq!(
            presentation(ViewPhase::Viewing, true),
            PlayerPresentation::Live
        );
        assert_eq!(
            audio_status(Locale::English, ViewAudioPhase::CallbackConsumed),
            "Audio"
        );
    }

    #[test]
    fn fullscreen_controls_hide_only_during_active_viewing() {
        assert!(controls_visible(true, false, 20.0, 0.0));
        assert!(controls_visible(true, true, 2.7, 0.0));
        assert!(!controls_visible(true, true, 2.9, 0.0));
        assert!(controls_visible(false, true, 20.0, 0.0));
    }

    #[test]
    fn narrow_controls_keep_fixed_actions_inside_the_toolbar() {
        let layout = control_layout(360.0, 2);
        assert_eq!(layout.button_width, COMPACT_CONTROL_BUTTON_WIDTH);
        assert_eq!(layout.actions_width, 192.0);
        assert_eq!(layout.info_width, 160.0);
        assert!(layout.info_width + CONTROL_GAP + layout.actions_width <= 360.0);
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
}
