//! Local settings page.

use eframe::egui;
use moqcast_diagnostics::FileStatus;
use moqcast_ui::{
    COLORS, SelectSpec, SettingRowSpec, Spacing, SwitchSpec, TypographyRole, major_section_break,
    section_header, select, setting_row, switch, typography,
};

use super::super::Locale;
use super::super::components;
use super::super::diagnostics::DiagnosticsUi;

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    developer_mode: &mut bool,
    diagnostics: &mut DiagnosticsUi,
) -> Option<Locale> {
    section_header(ui, locale.general(), None);
    const LANGUAGES: [&str; 2] = ["简体中文", "English"];
    let mut language_index = usize::from(locale == Locale::English);
    setting_row(
        ui,
        SettingRowSpec::new(locale.language()).description(locale.language_hint()),
        |ui| {
            select(
                ui,
                &mut language_index,
                SelectSpec::new(
                    egui::Id::new("linux-settings-language"),
                    locale.language(),
                    &LANGUAGES,
                )
                .expect("the language list is not empty"),
            )
            .expect("the language selection is valid");
        },
    );
    let selected = if language_index == 0 {
        Locale::Chinese
    } else {
        Locale::English
    };

    major_section_break(ui);
    section_header(ui, locale.advanced(), None);
    let mut auto_watch = false;
    setting_row(
        ui,
        SettingRowSpec::new(locale.auto_watch()).description(locale.auto_watch_hint()),
        |ui| {
            ui.horizontal(|ui| {
                ui.label(typography(
                    locale.temporarily_unavailable(),
                    TypographyRole::Meta,
                    COLORS.muted.into(),
                ));
                switch(
                    ui,
                    &mut auto_watch,
                    SwitchSpec::new(locale.auto_watch()).enabled(false),
                );
            });
        },
    );
    ui.separator();
    let was_developer_mode = *developer_mode;
    setting_row(
        ui,
        SettingRowSpec::new(locale.developer_mode()).description(locale.developer_mode_hint()),
        |ui| {
            switch(ui, developer_mode, SwitchSpec::new(locale.developer_mode()));
        },
    );
    if was_developer_mode && !*developer_mode {
        diagnostics.hide_window();
    }

    if *developer_mode {
        major_section_break(ui);
        section_header(
            ui,
            locale.diagnostics(),
            Some(locale.diagnostics_local_hint()),
        );

        let mut detailed = diagnostics.detailed();
        setting_row(
            ui,
            SettingRowSpec::new(locale.detailed_diagnostics())
                .description(locale.detailed_diagnostics_hint()),
            |ui| {
                if switch(
                    ui,
                    &mut detailed,
                    SwitchSpec::new(locale.detailed_diagnostics()),
                )
                .changed()
                {
                    diagnostics.set_detailed(detailed);
                }
            },
        );

        let mut visible = diagnostics.visible();
        ui.separator();
        setting_row(
            ui,
            SettingRowSpec::new(locale.show_logs()).description(locale.show_logs_hint()),
            |ui| {
                if switch(ui, &mut visible, SwitchSpec::new(locale.show_logs())).changed() {
                    diagnostics.set_visible(visible);
                }
            },
        );
        ui.separator();
        let file_available = match diagnostics.file_status() {
            FileStatus::Available(_) => true,
            FileStatus::Unavailable(reason) => {
                ui.label(typography(
                    locale.file_diagnostics_unavailable(&reason),
                    TypographyRole::Help,
                    COLORS.danger.into(),
                ));
                false
            }
        };
        let file_summary = format!(
            "{} · {}: {}",
            if file_available {
                locale.file_diagnostics_available()
            } else {
                locale.file_diagnostics_unavailable_short()
            },
            locale.dropped_diagnostics(),
            diagnostics.dropped_count()
        );
        setting_row(
            ui,
            SettingRowSpec::new(locale.local_log_files()).description(&file_summary),
            |ui| {
                ui.horizontal(|ui| {
                    if components::secondary_button(ui, locale.open_log_directory(), file_available)
                        .clicked()
                    {
                        diagnostics.open_directory();
                    }
                    if components::secondary_button(ui, locale.export_logs(), file_available)
                        .clicked()
                    {
                        diagnostics.choose_export();
                    }
                });
            },
        );
        if let Some(status) = diagnostics.localized_status(locale) {
            ui.add_space(Spacing::SM);
            ui.label(typography(
                status,
                TypographyRole::Help,
                COLORS.muted.into(),
            ));
        }
    }

    (selected != locale).then_some(selected)
}
