//! Real-time screen player layout and local interaction state.

use std::time::Duration;

use eframe::egui::{
    self, Color32, Event, Frame, Margin, Rect, RichText, Sense, Stroke, TextureHandle,
    ViewportCommand,
};

use super::Locale;

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROLS_HIDE_AFTER: f64 = 2.8;
const CONTROL_HEIGHT: f32 = 92.0;

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
    let scale = (available.x / source.x).min(available.y / source.y);
    let image = source * scale;

    PlayerLayout {
        surface: if fullscreen { available } else { image },
        image,
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
    Preparing { path: &'a str },
    Viewing { path: &'a str, stopping: bool },
}

impl PlayerMode<'_> {
    fn path(&self) -> &str {
        match self {
            Self::Preparing { path } | Self::Viewing { path, .. } => path,
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
        let available = if fullscreen {
            ui.available_size()
        } else {
            egui::vec2(ui.available_width(), ui.available_height().min(540.0))
        };
        let layout = player_layout(texture.map(TextureHandle::size_vec2), available, fullscreen);
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
            let remaining = (CONTROLS_HIDE_AFTER - (now - self.controls_last_active)).max(0.0);
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(remaining));
        } else if fullscreen {
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
                        .fill(Color32::from_black_alpha(205))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_min_width((controls.width() - 28.0).max(1.0));
                            ui.horizontal(|ui| {
                                if mode.viewing() {
                                    live_badge(ui);
                                } else {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new(locale.preparing_view())
                                            .size(12.0)
                                            .color(Color32::WHITE),
                                    );
                                }

                                let resolution = texture
                                    .map(|texture| {
                                        let [width, height] = texture.size();
                                        format!("{width} × {height}")
                                    })
                                    .unwrap_or_else(|| locale.waiting_for_first_frame().to_owned());
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("{}  ·  {resolution}", mode.path()))
                                            .size(11.0)
                                            .color(Color32::from_gray(205)),
                                    )
                                    .truncate(),
                                );
                            });
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                if player_button(ui, locale.stop_watching(), !mode.stopping(), true)
                                    .clicked()
                                {
                                    action = Some(PlayerAction::StopWatching);
                                }
                                if mode.viewing()
                                    && player_button(
                                        ui,
                                        if fullscreen {
                                            locale.exit_fullscreen()
                                        } else {
                                            locale.enter_fullscreen()
                                        },
                                        !mode.stopping(),
                                        false,
                                    )
                                    .clicked()
                                {
                                    ui.ctx().send_viewport_cmd(ViewportCommand::Fullscreen(
                                        !fullscreen,
                                    ));
                                }
                            });
                        });
                });
            }
        });

        action
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

fn player_button(ui: &mut egui::Ui, label: &str, enabled: bool, danger: bool) -> egui::Response {
    let text = if danger {
        Color32::from_rgb(255, 174, 174)
    } else {
        Color32::WHITE
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).size(12.0).strong().color(text))
            .fill(Color32::from_black_alpha(120))
            .stroke(Stroke::new(1.0, Color32::from_gray(105)))
            .corner_radius(egui::CornerRadius::same(8))
            .min_size(egui::vec2(112.0, 36.0)),
    )
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

        assert_size(layout.surface, egui::vec2(1000.0, 562.5));
        assert_size(layout.image, layout.surface);
    }

    #[test]
    fn portrait_frame_is_constrained_by_available_height() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(1000.0, 700.0),
            false,
        );

        assert_size(layout.surface, egui::vec2(393.75, 700.0));
        assert_size(layout.image, layout.surface);
    }

    #[test]
    fn image_never_exceeds_the_available_region() {
        let layout = player_layout(
            Some(egui::vec2(1920.0, 1080.0)),
            egui::vec2(500.0, 200.0),
            false,
        );

        assert!((layout.surface.x - 355.555_57).abs() < 0.001);
        assert_eq!(layout.surface.y, 200.0);
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
