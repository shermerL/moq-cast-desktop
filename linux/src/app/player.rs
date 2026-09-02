//! Real-time screen player layout and local interaction state.

use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, Event, Layout, Rect, Sense, TextureHandle, ViewportCommand,
};
use moqcast_ui::{
    ButtonSpec, COLORS, ControlRole, IconButtonSpec, Size, TypographyRole, control_button,
    player_icon_button, player_rects, player_stage_at, player_toolbar_at, typography,
};

use super::{Locale, RemoteAudioPhase, RemoteAudioSnapshot};

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROLS_HIDE_AFTER: f64 = 2.8;
const CONTROL_BUTTON_WIDTH: f32 = 108.0;
const COMPACT_CONTROL_BUTTON_WIDTH: f32 = 92.0;
const CONTROL_GAP: f32 = Size::PLAYER_TOOLBAR_ITEM_SPACING;

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
    let surface = player_rects(Rect::from_min_size(egui::Pos2::ZERO, available), fullscreen)
        .stage
        .size();
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
}

pub(super) struct LivePlayer {
    fullscreen: FullscreenState,
    controls_last_active: f64,
}

impl Default for LivePlayer {
    fn default() -> Self {
        Self {
            fullscreen: FullscreenState::default(),
            controls_last_active: f64::NEG_INFINITY,
        }
    }
}

impl LivePlayer {
    pub(super) fn reconcile_fullscreen(&mut self, context: &egui::Context, viewing: bool) -> bool {
        let (actual, now) =
            context.input(|input| (input.viewport().fullscreen.unwrap_or(false), input.time));
        if actual && !self.fullscreen.active() {
            self.controls_last_active = now;
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
        mode: PlayerMode<'_>,
        texture: Option<&TextureHandle>,
    ) -> Option<PlayerAction> {
        let fullscreen = self.fullscreen.active();
        let available = ui.available_rect_before_wrap();
        let rects = player_rects(available, fullscreen);
        let layout = player_layout(
            texture.map(TextureHandle::size_vec2),
            available.size(),
            fullscreen,
        );
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

        let controls_visible =
            controls_visible(fullscreen, mode.viewing(), now, self.controls_last_active);
        if fullscreen && controls_visible {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            let remaining = (CONTROLS_HIDE_AFTER - (now - self.controls_last_active)).max(0.0);
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(remaining));
        } else if fullscreen {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        }

        let mut action = None;
        let occupied = if fullscreen {
            rects.stage
        } else {
            rects.stage.union(rects.toolbar)
        };
        ui.allocate_rect(occupied, Sense::hover());
        player_stage_at(ui, rects.stage, |_| ());
        if let Some(texture) = texture {
            let image = Rect::from_center_size(rects.stage.center(), layout.image);
            ui.painter().image(
                texture.id(),
                image,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rects.stage), |ui| {
                ui.centered_and_justified(|ui| ui.spinner());
            });
        }
        if controls_visible {
            player_toolbar_at(ui, rects.toolbar, |ui| {
                show_controls(ui, locale, &mode, texture, fullscreen, &mut action);
            });
        }

        action
    }
}

fn show_controls(
    ui: &mut egui::Ui,
    locale: Locale,
    mode: &PlayerMode<'_>,
    texture: Option<&TextureHandle>,
    fullscreen: bool,
    action: &mut Option<PlayerAction>,
) {
    let action_count = 1 + usize::from(mode.viewing());
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
            if mode.viewing() {
                live_badge(ui);
            } else {
                ui.spinner();
                ui.label(typography(
                    locale.preparing_view(),
                    TypographyRole::Meta,
                    COLORS.player_text.into(),
                ));
            }
            ui.label(typography(
                mode.device(),
                TypographyRole::Meta,
                COLORS.player_text.into(),
            ));
            let resolution = texture
                .map(|texture| {
                    let [width, height] = texture.size();
                    format!("{width} × {height}")
                })
                .unwrap_or_else(|| locale.waiting_for_first_frame().to_owned());
            let audio = mode.audio();
            let details = format!("{resolution} · {}", locale.remote_audio_status(audio.phase));
            let color = if audio.phase == RemoteAudioPhase::Failed {
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
            let enabled = !mode.stopping();
            if mode.viewing()
                && player_icon_button(
                    ui,
                    IconButtonSpec::player(
                        "⛶",
                        if fullscreen {
                            locale.exit_fullscreen()
                        } else {
                            locale.enter_fullscreen()
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
                ButtonSpec::new(locale.stop_watching(), ControlRole::PlayerIcon)
                    .enabled(enabled)
                    .min_width(row_layout.button_width),
            )
            .clicked()
            {
                if fullscreen {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
                *action = Some(PlayerAction::StopWatching);
            }
        });
    });
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

fn live_badge(ui: &mut egui::Ui) {
    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(dot.center(), 4.0, COLORS.live);
    ui.label(typography("LIVE", TypographyRole::Meta, COLORS.live.into()));
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

        assert_size(layout.surface, egui::vec2(880.0, 495.0));
        assert_size(layout.image, layout.surface);
    }

    #[test]
    fn portrait_frame_uses_side_bars_inside_the_stable_stage() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1000.0, 700.0),
            false,
        );

        assert_size(layout.surface, egui::vec2(880.0, 495.0));
        assert_size(layout.image, egui::vec2(278.437_5, 495.0));
    }

    #[test]
    fn image_never_exceeds_the_available_region() {
        let layout = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(500.0, 200.0),
            false,
        );

        assert!((layout.surface.x - 263.111_1).abs() < 0.001);
        assert_eq!(layout.surface.y, 148.0);
        assert_size(layout.image, layout.surface);
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
        assert_size(missing.surface, egui::vec2(800.0, 450.0));
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

        assert_size(landscape.surface, egui::vec2(420.0, 236.25));
        assert_size(portrait.surface, landscape.surface);
        assert_size(portrait.image, egui::vec2(132.890_63, 236.25));
    }

    #[test]
    fn narrow_controls_reserve_compact_actions_without_overflow() {
        let layout = control_layout(360.0, 2);

        assert_eq!(layout.button_width, COMPACT_CONTROL_BUTTON_WIDTH);
        assert_eq!(layout.actions_width, 192.0);
        assert_eq!(layout.info_width, 160.0);
        assert_eq!(
            layout.info_width + CONTROL_GAP + layout.actions_width,
            360.0
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
        let preparing = player_rects(available, false);
        let first_frame = player_rects(available, false);
        let watching = player_rects(available, false);

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
    fn fullscreen_controls_hide_after_the_idle_window() {
        assert!(controls_visible(false, true, 10.0, 0.0));
        assert!(controls_visible(true, false, 10.0, 0.0));
        assert!(controls_visible(true, true, 2.79, 0.0));
        assert!(!controls_visible(true, true, 2.8, 0.0));
    }
}
