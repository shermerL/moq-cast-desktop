//! Shared-system remote screen player surface and controls.

use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, TextureHandle, ViewportCommand};
use moqcast_ui::{
    ButtonSpec, COLORS, ControlRole, IconButtonSpec, Size, TypographyRole, control_button,
    player_icon_button, player_rects, player_stage_at, player_toolbar_at, typography,
};

use super::Locale;
use crate::runtime::MediaPhase;

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
}

#[derive(Default)]
pub(super) struct Player {
    fullscreen: bool,
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

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        locale: Locale,
        phase: MediaPhase,
        device_name: &str,
        texture: Option<(&TextureHandle, (u32, u32))>,
    ) -> Option<PlayerAction> {
        if self.fullscreen && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Fullscreen(false));
            self.fullscreen = false;
        }
        let source = texture.map(|(_, (width, height))| egui::vec2(width as f32, height as f32));
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
        let mut action = None;
        let occupied = if self.fullscreen {
            rects.stage
        } else {
            rects.stage.union(rects.toolbar)
        };
        ui.allocate_rect(occupied, Sense::hover());
        player_stage_at(ui, rects.stage, |_| ());
        paint_surface(ui, rects.stage, layout.image, texture);
        paint_status(ui, rects.stage, locale, phase);
        show_toolbar(
            ui,
            rects.toolbar,
            locale,
            phase,
            device_name,
            self.fullscreen,
            &mut action,
        );
        action
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
    locale: Locale,
    phase: MediaPhase,
    device_name: &str,
    fullscreen: bool,
    action: &mut Option<PlayerAction>,
) {
    player_toolbar_at(ui, toolbar, |ui| {
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), Size::CONTROL),
            Sense::hover(),
        );
        let actions_width = 108.0 + Size::CONTROL + Size::PLAYER_TOOLBAR_ITEM_SPACING;
        let info = Rect::from_min_max(
            row.min,
            egui::pos2((row.right() - actions_width).max(row.left()), row.bottom()),
        );
        let actions = Rect::from_min_max(egui::pos2(info.right(), row.top()), row.right_bottom());
        let mut info_ui = ui.new_child(egui::UiBuilder::new().max_rect(info));
        info_ui.horizontal(|ui| {
            if phase == MediaPhase::Watching {
                live_badge(ui);
            } else {
                ui.label(typography(
                    match (locale, phase) {
                        (Locale::Chinese, MediaPhase::Stopping) => "正在停止",
                        (Locale::English, MediaPhase::Stopping) => "Stopping",
                        (Locale::Chinese, _) => "正在准备",
                        (Locale::English, _) => "Preparing",
                    },
                    TypographyRole::Meta,
                    COLORS.player_muted.into(),
                ));
            }
            ui.add_sized(
                ui.available_size(),
                egui::Label::new(typography(
                    format!(
                        "{} · {device_name}",
                        match locale {
                            Locale::Chinese => "附近屏幕",
                            Locale::English => "Nearby screen",
                        }
                    ),
                    TypographyRole::Meta,
                    COLORS.player_text.into(),
                ))
                .truncate(),
            );
        });
        let mut actions_ui = ui.new_child(egui::UiBuilder::new().max_rect(actions));
        actions_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let enabled = phase != MediaPhase::Stopping;
            if player_icon_button(
                ui,
                IconButtonSpec::player(
                    "⛶",
                    match (locale, fullscreen) {
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
                    match locale {
                        Locale::Chinese => "停止观看",
                        Locale::English => "Stop watching",
                    },
                    ControlRole::PlayerIcon,
                )
                .enabled(enabled)
                .min_width(108.0),
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
}
