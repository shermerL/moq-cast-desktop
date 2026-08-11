//! Screen publication page.

use eframe::egui::{self, Color32, RichText};

use super::super::{
    AppSnapshot, Locale, PeerState, PublishState, UserCommand, heading, section_frame,
};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
) -> Option<UserCommand> {
    heading(ui, locale.screen_share(), locale.share_description());

    let connected = matches!(snapshot.peer, PeerState::Connected { .. });
    section_frame().show(ui, |ui| {
        ui.set_min_height(220.0);
        if let PeerState::Connected { peer_id } = &snapshot.peer {
            ui.label(RichText::new(locale.connected()).size(18.0).strong());
            ui.label(
                RichText::new(peer_id)
                    .size(13.0)
                    .color(Color32::from_rgb(99, 116, 112)),
            );
        } else {
            ui.label(RichText::new(locale.not_connected()).size(18.0).strong());
            ui.label(
                RichText::new(locale.connect_first())
                    .size(13.0)
                    .color(Color32::from_rgb(99, 116, 112)),
            );
        }
        ui.add_space(28.0);
    });

    ui.add_space(16.0);
    match snapshot.publish {
        PublishState::Publishing => ui
            .button(locale.stop_sharing())
            .clicked()
            .then_some(UserCommand::StopScreenShare),
        PublishState::Idle => ui
            .add_enabled(connected, egui::Button::new(locale.choose_screen()))
            .clicked()
            .then_some(UserCommand::StartScreenShare),
        PublishState::Preparing | PublishState::Stopping => {
            ui.add_enabled(false, egui::Button::new(locale.choose_screen()));
            None
        }
    }
}
