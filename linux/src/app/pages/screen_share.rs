//! Local publication and remote screen playback page.

use eframe::egui::{self, RichText};

use super::super::{AppSnapshot, Locale, MUTED, MediaState, UserCommand, heading, section_frame};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    playback: Option<&egui::TextureHandle>,
) -> Option<UserCommand> {
    heading(ui, locale.screen_share(), locale.share_description());

    let connected = snapshot.has_mesh_session();
    section_frame().show(ui, |ui| {
        ui.set_min_height(220.0);
        if connected {
            let outbound = snapshot
                .peers
                .values()
                .filter(|peer| peer.transport == super::super::TransportState::Connected)
                .count();
            ui.label(RichText::new(locale.mesh_connected()).size(18.0).strong());
            ui.label(
                RichText::new(format!(
                    "{}: {} · {}: {}",
                    locale.outbound_sessions(),
                    outbound,
                    locale.inbound_sessions(),
                    snapshot.inbound_session_count
                ))
                .size(13.0)
                .color(MUTED),
            );
        } else {
            ui.label(RichText::new(locale.not_connected()).size(18.0).strong());
            ui.label(
                RichText::new(locale.connect_first())
                    .size(13.0)
                    .color(MUTED),
            );
        }

        match &snapshot.media {
            MediaState::PreparingPublish => {
                ui.label(RichText::new(locale.preparing_share()).size(15.0).strong());
            }
            MediaState::Publishing => {
                ui.label(RichText::new(locale.sharing_screen()).size(15.0).strong());
            }
            MediaState::StoppingPublish => {
                ui.label(RichText::new(locale.stopping_share()).size(15.0).strong());
            }
            MediaState::PreparingView { path } => {
                ui.label(RichText::new(locale.preparing_view()).size(15.0).strong());
                ui.label(RichText::new(path).size(12.0).color(MUTED));
            }
            MediaState::Viewing { path } | MediaState::StoppingView { path } => {
                ui.label(RichText::new(locale.viewing_screen()).size(15.0).strong());
                ui.label(RichText::new(path).size(12.0).color(MUTED));
                if let Some(texture) = playback {
                    let available = ui.available_size();
                    let source = texture.size_vec2();
                    let scale = (available.x / source.x)
                        .min(available.y.max(220.0) / source.y)
                        .min(1.0);
                    ui.image((texture.id(), source * scale));
                } else {
                    ui.spinner();
                }
            }
            MediaState::Idle => {}
        }
        ui.add_space(28.0);
    });

    ui.add_space(16.0);
    match snapshot.media {
        MediaState::Publishing => ui
            .button(locale.stop_sharing())
            .clicked()
            .then_some(UserCommand::StopScreenShare),
        MediaState::PreparingView { .. } | MediaState::Viewing { .. } => ui
            .button(locale.stop_watching())
            .clicked()
            .then_some(UserCommand::StopWatching),
        MediaState::Idle => ui
            .add_enabled(connected, egui::Button::new(locale.choose_screen()))
            .clicked()
            .then_some(UserCommand::StartScreenShare),
        _ => None,
    }
}
