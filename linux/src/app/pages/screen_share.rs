//! Local publication and remote screen playback page.

use eframe::egui::{self, RichText};

use super::super::components::{self, BadgeTone, danger_button, primary_button, status_badge};
use super::super::player::{LivePlayer, PlayerAction, PlayerMode};
use super::super::theme::{MUTED, TEXT};
use super::super::{AppSnapshot, Locale, MediaState, UserCommand};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    playback: Option<&egui::TextureHandle>,
    player: &mut LivePlayer,
    system_audio: &mut bool,
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
            let stopping = matches!(snapshot.media, MediaState::StoppingView { .. });
            if matches!(
                player.show(ui, locale, PlayerMode::Viewing { path, stopping }, playback,),
                Some(PlayerAction::StopWatching)
            ) {
                command = Some(UserCommand::StopWatching);
            }
        }
        MediaState::PreparingView { path } => {
            if matches!(
                player.show(ui, locale, PlayerMode::Preparing { path }, playback),
                Some(PlayerAction::StopWatching)
            ) {
                command = Some(UserCommand::StopWatching);
            }
        }
        MediaState::Publishing | MediaState::StoppingPublish => {
            components::surface().show(ui, |ui| {
                ui.set_min_height(210.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(34.0);
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
            components::surface().show(ui, |ui| {
                ui.set_min_height(210.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(44.0);
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
            components::surface().show(ui, |ui| {
                ui.set_min_height(210.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(38.0);
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
                    ui.add_space(12.0);
                    ui.checkbox(system_audio, locale.system_audio());
                    ui.label(
                        RichText::new(locale.system_audio_hint())
                            .size(12.0)
                            .color(MUTED),
                    );
                    ui.add_space(18.0);
                    if primary_button(ui, locale.choose_screen(), true).clicked() {
                        command = Some(UserCommand::StartScreenShare {
                            system_audio: *system_audio,
                        });
                    }
                });
            });
        }
    }

    command
}
