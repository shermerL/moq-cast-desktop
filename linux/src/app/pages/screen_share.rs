//! Local publication and remote screen playback page.

use eframe::egui::{self, Frame, Margin, RichText, Stroke};

use super::super::components::{self, BadgeTone, danger_button, primary_button, status_badge};
use super::super::theme::{BORDER, MUTED, PLAYER, RADIUS, TEXT};
use super::super::{AppSnapshot, Locale, MediaState, UserCommand};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    playback: Option<&egui::TextureHandle>,
) -> Option<UserCommand> {
    if !snapshot.has_mesh_session() && snapshot.media == MediaState::Idle {
        components::empty_state(
            ui,
            locale.not_connected(),
            locale.connect_first(),
            snapshot.discovery.is_active(),
        );
        return None;
    }

    let mut command = None;
    match &snapshot.media {
        MediaState::Viewing { path } | MediaState::StoppingView { path } => {
            Frame::new()
                .fill(PLAYER)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(RADIUS)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    let width = ui.available_width().max(1.0);
                    let height = (width * 9.0 / 16.0).clamp(240.0, 540.0);
                    ui.set_min_height(height);
                    if let Some(texture) = playback {
                        let source = texture.size_vec2();
                        let scale = (width / source.x).min(height / source.y);
                        ui.vertical_centered(|ui| {
                            ui.image((texture.id(), source * scale));
                        });
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.spinner();
                        });
                    }
                });
            ui.add_space(14.0);
            components::surface().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    status_badge(ui, locale.viewing_screen(), BadgeTone::Success);
                    ui.add(egui::Label::new(RichText::new(path).size(12.0).color(MUTED)).wrap());
                    let stopping = matches!(snapshot.media, MediaState::StoppingView { .. });
                    if danger_button(ui, locale.stop_watching(), !stopping).clicked() {
                        command = Some(UserCommand::StopWatching);
                    }
                });
            });
        }
        MediaState::PreparingView { path } => {
            components::muted_surface().show(ui, |ui| {
                ui.set_min_height(260.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(70.0);
                    ui.spinner();
                    ui.label(
                        RichText::new(locale.preparing_view())
                            .size(17.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add(egui::Label::new(RichText::new(path).size(12.0).color(MUTED)).wrap());
                    ui.add_space(8.0);
                    if danger_button(ui, locale.stop_watching(), true).clicked() {
                        command = Some(UserCommand::StopWatching);
                    }
                });
            });
        }
        MediaState::Publishing | MediaState::StoppingPublish => {
            components::surface().show(ui, |ui| {
                ui.set_min_height(300.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(78.0);
                    status_badge(ui, locale.sharing_screen(), BadgeTone::Success);
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(locale.share_description())
                            .size(14.0)
                            .color(MUTED),
                    );
                    ui.add_space(18.0);
                    let stopping = snapshot.media == MediaState::StoppingPublish;
                    if stopping {
                        ui.spinner();
                        danger_button(ui, locale.stopping_share(), false);
                    } else if danger_button(ui, locale.stop_sharing(), true).clicked() {
                        command = Some(UserCommand::StopScreenShare);
                    }
                });
            });
        }
        MediaState::PreparingPublish => {
            components::muted_surface().show(ui, |ui| {
                ui.set_min_height(300.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(90.0);
                    ui.spinner();
                    ui.label(
                        RichText::new(locale.preparing_share())
                            .size(17.0)
                            .strong()
                            .color(TEXT),
                    );
                    primary_button(ui, locale.preparing_share(), false);
                });
            });
        }
        MediaState::Idle => {
            components::muted_surface().show(ui, |ui| {
                ui.set_min_height(300.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(76.0);
                    ui.label(
                        RichText::new(locale.media_idle())
                            .size(19.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(locale.media_idle_hint())
                            .size(13.0)
                            .color(MUTED),
                    );
                    ui.add_space(18.0);
                    if primary_button(ui, locale.choose_screen(), true).clicked() {
                        command = Some(UserCommand::StartScreenShare);
                    }
                });
            });
        }
    }

    command
}
