//! Local publication and remote screen playback page.

use eframe::egui;
use moqcast_ui::{
    SettingRowSpec, StatePanelKind, StatePanelSpec, SwitchSpec, section_header, setting_row,
    state_panel, switch,
};

use super::super::components::{danger_button, primary_button, stable_status_strip};
use super::super::{AppSnapshot, Locale, MediaState, UserCommand};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    system_audio: &mut bool,
) -> Option<UserCommand> {
    section_header(
        ui,
        locale.share_local_screen(),
        Some(locale.share_description()),
    );
    let audio_enabled = snapshot.media == MediaState::Idle;
    setting_row(
        ui,
        SettingRowSpec::new(locale.system_audio()).description(locale.system_audio_hint()),
        |ui| {
            switch(
                ui,
                system_audio,
                SwitchSpec::new(locale.system_audio()).enabled(audio_enabled),
            );
        },
    );
    ui.add_space(moqcast_ui::Spacing::XL);

    if !snapshot.has_mesh_session() && snapshot.media == MediaState::Idle {
        state_panel(
            ui,
            StatePanelSpec::new(
                StatePanelKind::Empty,
                locale.not_connected(),
                locale.connect_first(),
            ),
            |_| (),
        );
        return None;
    }

    let mut command = None;
    match &snapshot.media {
        MediaState::Viewing { .. }
        | MediaState::StoppingView { .. }
        | MediaState::PreparingView { .. } => {
            state_panel(
                ui,
                StatePanelSpec::new(
                    StatePanelKind::Empty,
                    locale.viewing_screen(),
                    locale.open_watch_to_manage(),
                ),
                |_| (),
            );
        }
        MediaState::Publishing | MediaState::StoppingPublish => {
            stable_status_strip(
                ui,
                StatePanelSpec::new(
                    StatePanelKind::Pending,
                    locale.sharing_screen(),
                    locale.media_keeps_mesh(),
                ),
                |ui| {
                    let stopping = snapshot.media == MediaState::StoppingPublish;
                    if stopping {
                        ui.spinner();
                        danger_button(ui, locale.stopping_share(), false);
                    } else if danger_button(ui, locale.stop_sharing(), true).clicked() {
                        command = Some(UserCommand::StopScreenShare);
                    }
                },
            );
        }
        MediaState::PreparingPublish => {
            stable_status_strip(
                ui,
                StatePanelSpec::new(
                    StatePanelKind::Pending,
                    locale.preparing_share(),
                    locale.share_description(),
                ),
                |ui| {
                    ui.spinner();
                    primary_button(ui, locale.preparing_share(), false);
                },
            );
        }
        MediaState::Idle => {
            stable_status_strip(
                ui,
                StatePanelSpec::new(
                    StatePanelKind::Empty,
                    locale.media_idle(),
                    locale.media_idle_hint(),
                ),
                |ui| {
                    if primary_button(ui, locale.choose_screen(), true).clicked() {
                        command = Some(UserCommand::StartScreenShare {
                            system_audio: *system_audio,
                        });
                    }
                },
            );
        }
    }

    command
}
