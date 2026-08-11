//! Local settings and build information page.

use eframe::egui::{self, RichText};

use super::super::{Locale, heading, section_frame};

pub(in crate::app) fn show(ui: &mut egui::Ui, locale: Locale) -> Option<Locale> {
    heading(ui, locale.settings(), "");
    let mut selected = locale;

    section_frame().show(ui, |ui| {
        ui.label(RichText::new(locale.language()).size(16.0).strong());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut selected, Locale::Chinese, "简体中文");
            ui.selectable_value(&mut selected, Locale::English, "English");
        });
    });

    ui.add_space(16.0);
    section_frame().show(ui, |ui| {
        ui.label(RichText::new(locale.about()).size(16.0).strong());
        egui::Grid::new("about-grid")
            .num_columns(2)
            .spacing([28.0, 10.0])
            .show(ui, |ui| {
                ui.label(locale.app_version());
                ui.label(env!("CARGO_PKG_VERSION"));
                ui.end_row();
                ui.label(locale.protocol());
                ui.label("MoQ / QUIC");
                ui.end_row();
            });
    });

    (selected != locale).then_some(selected)
}
