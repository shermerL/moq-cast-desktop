//! Local settings page.

use eframe::egui;
use moqcast_diagnostics::FileStatus;

use super::super::Locale;
use super::super::components;
use super::super::diagnostics::DiagnosticsUi;

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    diagnostics: &mut DiagnosticsUi,
) -> Option<Locale> {
    let mut selected = locale;

    components::surface().show(ui, |ui| {
        components::section_title(ui, locale.language(), None);
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut selected, Locale::Chinese, "简体中文");
            ui.selectable_value(&mut selected, Locale::English, "English");
        });
    });

    ui.add_space(12.0);
    components::surface().show(ui, |ui| {
        components::section_title(
            ui,
            locale.diagnostics(),
            Some(locale.diagnostics_local_hint()),
        );
        ui.add_space(10.0);

        let mut detailed = diagnostics.detailed();
        if components::selection_checkbox(ui, &mut detailed, locale.detailed_diagnostics(), true)
            .changed()
        {
            diagnostics.set_detailed(detailed);
        }
        ui.label(egui::RichText::new(locale.detailed_diagnostics_hint()).small());

        let mut visible = diagnostics.visible();
        if components::selection_checkbox(ui, &mut visible, locale.show_logs(), true).changed() {
            diagnostics.set_visible(visible);
        }

        ui.add_space(8.0);
        let file_available = match diagnostics.file_status() {
            FileStatus::Available(directory) => {
                ui.label(
                    egui::RichText::new(format!(
                        "{}: {}",
                        locale.log_directory(),
                        directory.display()
                    ))
                    .small(),
                );
                true
            }
            FileStatus::Unavailable(reason) => {
                ui.label(
                    egui::RichText::new(locale.file_diagnostics_unavailable(&reason))
                        .small()
                        .color(super::super::theme::ERROR),
                );
                false
            }
        };
        ui.label(
            egui::RichText::new(format!(
                "{}: {}",
                locale.dropped_diagnostics(),
                diagnostics.dropped_count()
            ))
            .small(),
        );
        ui.horizontal_wrapped(|ui| {
            if components::secondary_button(ui, locale.open_log_directory(), file_available)
                .clicked()
            {
                diagnostics.open_directory();
            }
            if components::secondary_button(ui, locale.export_logs(), file_available).clicked() {
                diagnostics.choose_export();
            }
        });
        if let Some(status) = diagnostics.localized_status(locale) {
            ui.label(egui::RichText::new(status).small());
        }
    });

    (selected != locale).then_some(selected)
}
