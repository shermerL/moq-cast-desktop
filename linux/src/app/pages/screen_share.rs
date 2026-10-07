//! Local publication and remote screen playback page.

use eframe::egui;
use moqcast_ui::{
    BadgeTone, DeviceBadgeSpec, DeviceListItemSpec, DeviceListSpec, SettingRowSpec, StatePanelKind,
    StatePanelSpec, SwitchSpec, device_list, section_header, setting_row, state_panel, switch,
};

use super::super::components::{danger_button, primary_button, stable_status_strip};
use super::super::{AppSnapshot, CaptureSource, Locale, MediaState, SourceCatalog, UserCommand};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    system_audio: &mut bool,
    selected_source: &mut Option<CaptureSource>,
) -> Option<UserCommand> {
    section_header(
        ui,
        locale.share_local_screen(),
        Some(locale.share_description()),
    );
    let audio_enabled = snapshot.media == MediaState::Idle;
    let mut command = None;
    let text = |chinese, english| match locale {
        Locale::Chinese => chinese,
        Locale::English => english,
    };
    ui.add_enabled_ui(audio_enabled, |ui| {
        match &snapshot.sources {
            SourceCatalog::Unloaded => {
                if audio_enabled {
                    command = Some(UserCommand::RefreshCaptureSources);
                }
            }
            SourceCatalog::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(text("正在查找共享来源…", "Loading capture sources..."));
                });
            }
            SourceCatalog::Portal => {
                if audio_enabled
                    && !matches!(
                        selected_source,
                        Some(CaptureSource::Portal | CaptureSource::PortalWindow)
                    )
                {
                    *selected_source = Some(CaptureSource::Portal);
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        selected_source,
                        Some(CaptureSource::Portal),
                        text("屏幕", "Screen"),
                    );
                    ui.selectable_value(
                        selected_source,
                        Some(CaptureSource::PortalWindow),
                        text("窗口", "Window"),
                    );
                });
                ui.label(text(
                    "点击开始共享后，在系统弹窗中选择来源。停止后可重新选择。",
                    "Start sharing to choose a source in the system picker. Stop to choose again.",
                ));
            }
            SourceCatalog::Failed(error) => {
                *selected_source = None;
                ui.label(text(
                    "共享来源不可用，请刷新后重新选择。",
                    "Capture sources are unavailable. Refresh and select again.",
                ));
                ui.label(error);
            }
            SourceCatalog::Ready(choices) => {
                let selected = snapshot.sources.selected(selected_source.as_ref());
                let labels = choices.iter().map(CaptureSource::label).collect::<Vec<_>>();
                let items = choices
                    .iter()
                    .zip(&labels)
                    .enumerate()
                    .map(|(index, (choice, label))| {
                        let active = selected.as_ref() == Some(choice);
                        DeviceListItemSpec::new(index, label)
                            .subtitle(text("屏幕", "Screen"))
                            .badge(DeviceBadgeSpec::new(
                                if active {
                                    text("已选择", "Selected")
                                } else {
                                    text("可用", "Available")
                                },
                                if active {
                                    BadgeTone::Info
                                } else {
                                    BadgeTone::Neutral
                                },
                            ))
                            .selected(active)
                            .enabled(audio_enabled)
                    })
                    .collect::<Vec<_>>();
                if let Some(index) = device_list(
                    ui,
                    DeviceListSpec::new(egui::Id::new("linux-capture-sources"), &items)
                        .viewport_height(280.0),
                ) {
                    *selected_source = Some(choices[index].clone());
                }
                if choices.is_empty() {
                    ui.label(text("没有可共享的屏幕。", "No screens are available."));
                } else if selected_source.is_some() && selected.is_none() {
                    ui.label(text(
                        "原来源已不可用或屏幕布局已变化，请重新选择。",
                        "The source is unavailable or the display layout changed. Select again.",
                    ));
                }
            }
        }
        if !matches!(
            snapshot.sources,
            SourceCatalog::Unloaded | SourceCatalog::Loading | SourceCatalog::Portal
        ) && ui.button(text("刷新来源", "Refresh sources")).clicked()
        {
            command = Some(UserCommand::RefreshCaptureSources);
        }
    });
    ui.add_space(moqcast_ui::Spacing::XL);
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
        return command;
    }

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
                    let source = snapshot.sources.selected(selected_source.as_ref());
                    let label = if matches!(snapshot.sources, SourceCatalog::Portal) {
                        if source == Some(CaptureSource::PortalWindow) {
                            text("选择窗口并共享", "Choose window and share")
                        } else {
                            locale.choose_screen()
                        }
                    } else {
                        text("开始共享", "Start sharing")
                    };
                    if primary_button(ui, label, source.is_some()).clicked()
                        && let Some(source) = source
                    {
                        command = Some(UserCommand::StartScreenShare {
                            system_audio: *system_audio,
                            source,
                        });
                    }
                },
            );
        }
    }

    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_window_choice_survives_rendering() {
        let mut selected = Some(CaptureSource::PortalWindow);
        let mut audio = false;
        let snapshot = AppSnapshot {
            sources: SourceCatalog::Portal,
            ..AppSnapshot::default()
        };
        let context = egui::Context::default();
        let frame = context.run_ui(egui::RawInput::default(), |ui| {
            show(ui, Locale::English, &snapshot, &mut audio, &mut selected);
        });
        frame.drop_without_applying_deltas();
        assert_eq!(selected, Some(CaptureSource::PortalWindow));
    }

    #[test]
    fn failed_source_catalog_clears_selection_before_refresh() {
        let source = CaptureSource::Display {
            id: "x11:0".into(),
            name: "DP-1".into(),
            width: 1920,
            height: 1080,
        };
        let mut selected = Some(source.clone());
        let mut audio = false;
        let snapshot = AppSnapshot {
            sources: SourceCatalog::Failed("Source changed. Refresh and select again.".into()),
            ..AppSnapshot::default()
        };
        let context = egui::Context::default();
        let frame = context.run_ui(egui::RawInput::default(), |ui| {
            show(ui, Locale::English, &snapshot, &mut audio, &mut selected);
        });
        frame.drop_without_applying_deltas();
        assert!(selected.is_none());
        assert!(
            SourceCatalog::Ready(vec![source])
                .selected(selected.as_ref())
                .is_none()
        );
    }
}
