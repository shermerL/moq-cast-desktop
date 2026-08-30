//! Stable remote-screen player surface and controls.

use eframe::egui::{
    self, Align, Color32, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, TextureHandle,
    ViewportCommand,
};

use super::Locale;
use crate::runtime::MediaPhase;

const FALLBACK_ASPECT: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROL_HEIGHT: f32 = 52.0;

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
    let surface = if fullscreen {
        available
    } else {
        egui::vec2(
            available.x,
            (available.x * FALLBACK_ASPECT.y / FALLBACK_ASPECT.x).min(available.y),
        )
    };
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
        let layout = player_layout(source, ui.available_size(), self.fullscreen);
        let mut action = None;
        ui.vertical_centered(|ui| {
            let (surface, _) = ui.allocate_exact_size(layout.surface, Sense::hover());
            ui.painter().rect_filled(
                surface,
                if self.fullscreen { 0.0 } else { 8.0 },
                Color32::BLACK,
            );
            if let Some((texture, _)) = texture {
                let image = Rect::from_center_size(surface.center(), layout.image);
                ui.painter().image(
                    texture.id(),
                    image,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            if phase != MediaPhase::Watching {
                let status = Rect::from_min_max(
                    surface.min,
                    egui::pos2(surface.right(), (surface.bottom() - CONTROL_HEIGHT).max(surface.top())),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            if phase == MediaPhase::PreparingWatch {
                                ui.spinner();
                            }
                            let (title, body) = match (locale, phase) {
                                (Locale::Chinese, MediaPhase::Failed) => (
                                    "无法读取兼容的视频",
                                    "安全连接仍然可用。停止观看后可从设备详情重试。",
                                ),
                                (Locale::English, MediaPhase::Failed) => (
                                    "Could not play compatible video",
                                    "The secure session is still available. Stop watching, then retry from device details.",
                                ),
                                (Locale::Chinese, _) => (
                                    "正在准备画面",
                                    "安全连接已建立，正在读取兼容的视频目录。",
                                ),
                                (Locale::English, _) => (
                                    "Preparing video",
                                    "The secure session is ready while a compatible video catalog is opened.",
                                ),
                            };
                            ui.label(RichText::new(title).size(16.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(body).size(12.0).color(Color32::from_gray(180)));
                        });
                    });
                });
            }

            let controls = Rect::from_min_max(
                egui::pos2(surface.left(), (surface.bottom() - CONTROL_HEIGHT).max(surface.top())),
                surface.right_bottom(),
            );
            ui.painter()
                .rect_filled(controls, 0.0, Color32::from_black_alpha(230));
            ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
                Frame::NONE
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_min_height(36.0);
                        ui.horizontal(|ui| {
                            if phase == MediaPhase::Watching {
                                live_badge(ui);
                            } else {
                                ui.label(
                                    RichText::new(match locale {
                                        Locale::Chinese => "正在准备",
                                        Locale::English => "PREPARING",
                                    })
                                    .size(11.0)
                                    .strong()
                                    .color(Color32::from_gray(200)),
                                );
                            }
                            ui.label(
                                RichText::new(device_name)
                                    .size(12.0)
                                    .color(Color32::from_gray(232)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if player_button(
                                    ui,
                                    match locale {
                                        Locale::Chinese => "停止观看",
                                        Locale::English => "Stop Watching",
                                    },
                                    true,
                                )
                                .clicked()
                                {
                                    action = Some(PlayerAction::Stop);
                                }
                                if player_button(
                                    ui,
                                    match (locale, self.fullscreen) {
                                        (Locale::Chinese, true) => "退出全屏",
                                        (Locale::Chinese, false) => "全屏",
                                        (Locale::English, true) => "Exit Fullscreen",
                                        (Locale::English, false) => "Fullscreen",
                                    },
                                    false,
                                )
                                .clicked()
                                {
                                    self.fullscreen = !self.fullscreen;
                                    ui.ctx().send_viewport_cmd(ViewportCommand::Fullscreen(
                                        self.fullscreen,
                                    ));
                                }
                            });
                        });
                    });
            });
        });
        action
    }
}

fn live_badge(ui: &mut egui::Ui) {
    Frame::NONE
        .fill(Color32::from_rgb(178, 24, 32))
        .stroke(Stroke::new(1.0, Color32::from_rgb(235, 96, 101)))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new("LIVE")
                    .size(11.0)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
}

fn player_button(ui: &mut egui::Ui, label: &str, danger: bool) -> egui::Response {
    ui.add_sized(
        [112.0, 32.0],
        egui::Button::new(RichText::new(label).size(12.0).strong().color(if danger {
            Color32::from_rgb(255, 201, 196)
        } else {
            Color32::WHITE
        }))
        .fill(Color32::from_rgb(36, 39, 37))
        .stroke(Stroke::new(1.0, Color32::from_gray(93)))
        .corner_radius(egui::CornerRadius::same(6)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windowed_surface_stays_sixteen_by_nine() {
        let layout = player_layout(None, egui::vec2(900.0, 700.0), false);
        assert_eq!(layout.surface, egui::vec2(900.0, 506.25));
    }

    #[test]
    fn portrait_video_is_contained_in_the_stable_surface() {
        let layout = player_layout(
            Some(egui::vec2(1080.0, 1920.0)),
            egui::vec2(900.0, 700.0),
            false,
        );
        assert_eq!(layout.surface, egui::vec2(900.0, 506.25));
        assert_eq!(layout.image, egui::vec2(284.765_63, 506.25));
    }

    #[test]
    fn fullscreen_surface_fills_the_available_viewport() {
        let layout = player_layout(None, egui::vec2(1440.0, 900.0), true);
        assert_eq!(layout.surface, egui::vec2(1440.0, 900.0));
    }
}
