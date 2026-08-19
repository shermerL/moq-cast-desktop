//! Aspect-fit remote screen player for the Windows desktop UI.

use eframe::egui::{self, Color32, Frame, Margin, Rect, RichText, Sense, Stroke, ViewportCommand};

use crate::playback::{ViewAudioPhase, ViewPhase, ViewSnapshot};

const FALLBACK_SOURCE: egui::Vec2 = egui::vec2(16.0, 9.0);
const CONTROL_HEIGHT: f32 = 78.0;

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

fn player_layout(source: Option<egui::Vec2>, available: egui::Vec2) -> PlayerLayout {
    let available = egui::vec2(valid_extent(available.x), valid_extent(available.y));
    let source = source
        .filter(|size| valid_size(*size))
        .unwrap_or(FALLBACK_SOURCE);
    let scale = (available.x / source.x).min(available.y / source.y);
    let image = source * scale;
    PlayerLayout {
        surface: available,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerAction {
    Stop,
}

#[derive(Default)]
pub(crate) struct LivePlayer {
    layout_signature: Option<PlayerLayoutSignature>,
}

impl LivePlayer {
    fn update_layout_signature(&mut self, signature: PlayerLayoutSignature) -> bool {
        if self.layout_signature == Some(signature) {
            return false;
        }
        self.layout_signature = Some(signature);
        true
    }

    pub(crate) fn fullscreen(context: &egui::Context) -> bool {
        context.input(|input| input.viewport().fullscreen.unwrap_or(false))
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        view: &ViewSnapshot,
        texture: Option<&egui::TextureHandle>,
        fullscreen: bool,
    ) -> Option<PlayerAction> {
        let available = ui.available_size();
        let source = view
            .width
            .zip(view.height)
            .map(|(width, height)| egui::vec2(width as f32, height as f32));
        let layout = player_layout(source, available);
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
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_min_width((controls.width() - 28.0).max(1.0));
                        ui.horizontal(|ui| {
                            let status = match view.phase {
                                ViewPhase::Preparing => "PREPARING",
                                ViewPhase::Viewing => "LIVE",
                                ViewPhase::Stopping => "STOPPING",
                                ViewPhase::Idle | ViewPhase::Failed => "ENDED",
                            };
                            ui.label(RichText::new(status).small().strong().color(Color32::WHITE));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(view.path.as_deref().unwrap_or("remote screen"))
                                        .small()
                                        .color(Color32::from_gray(205)),
                                )
                                .truncate(),
                            );
                            ui.label(
                                RichText::new(match view.audio.phase {
                                    ViewAudioPhase::Idle => "NO AUDIO",
                                    ViewAudioPhase::Pending => "AUDIO...",
                                    ViewAudioPhase::NotPublished => "NO AUDIO",
                                    ViewAudioPhase::Playing => "AUDIO",
                                    ViewAudioPhase::Failed => "AUDIO ERROR",
                                })
                                .small()
                                .color(match view.audio.phase {
                                    ViewAudioPhase::Failed => Color32::from_rgb(255, 174, 174),
                                    _ => Color32::from_gray(205),
                                }),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let enabled = view.phase != ViewPhase::Stopping;
                                    if player_button(ui, "Stop", enabled, true).clicked() {
                                        action = Some(PlayerAction::Stop);
                                    }
                                    if view.phase == ViewPhase::Viewing
                                        && player_button(
                                            ui,
                                            if fullscreen {
                                                "Exit fullscreen"
                                            } else {
                                                "Fullscreen"
                                            },
                                            enabled,
                                            false,
                                        )
                                        .clicked()
                                    {
                                        ui.ctx().send_viewport_cmd(ViewportCommand::Fullscreen(
                                            !fullscreen,
                                        ));
                                    }
                                },
                            );
                        });
                    });
            });
        });
        action
    }
}

fn player_button(ui: &mut egui::Ui, label: &str, enabled: bool, danger: bool) -> egui::Response {
    let color = if danger {
        Color32::from_rgb(255, 174, 174)
    } else {
        Color32::WHITE
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).small().strong().color(color))
            .fill(Color32::from_black_alpha(120))
            .stroke(Stroke::new(1.0, Color32::from_gray(105)))
            .corner_radius(8.0),
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
    fn landscape_and_portrait_frames_remain_inside_the_player() {
        let landscape = player_layout(Some(egui::vec2(1920.0, 1080.0)), egui::vec2(1000.0, 700.0));
        assert_size(landscape.surface, egui::vec2(1000.0, 700.0));
        assert_size(landscape.image, egui::vec2(1000.0, 562.5));

        let portrait = player_layout(Some(egui::vec2(1080.0, 1920.0)), egui::vec2(1000.0, 700.0));
        assert_size(portrait.surface, egui::vec2(1000.0, 700.0));
        assert_size(portrait.image, egui::vec2(393.75, 700.0));
    }

    #[test]
    fn fullscreen_fills_the_surface_and_letterboxes_the_image() {
        let layout = player_layout(Some(egui::vec2(1080.0, 1920.0)), egui::vec2(1200.0, 800.0));
        assert_size(layout.surface, egui::vec2(1200.0, 800.0));
        assert_size(layout.image, egui::vec2(450.0, 800.0));
    }

    #[test]
    fn invalid_source_uses_a_stable_fallback() {
        assert_eq!(
            player_layout(None, egui::vec2(800.0, 600.0)),
            player_layout(Some(egui::vec2(f32::NAN, 0.0)), egui::vec2(800.0, 600.0),)
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
}
