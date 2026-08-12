//! Local settings and build information page.

use eframe::egui::{self, RichText};

use super::super::Locale;
use super::super::components;
use super::super::theme::{MUTED, TEXT};

pub(in crate::app) fn show(ui: &mut egui::Ui, locale: Locale) -> Option<Locale> {
    let mut selected = locale;

    components::surface().show(ui, |ui| {
        components::section_title(ui, locale.language(), None);
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut selected, Locale::Chinese, "简体中文");
            ui.selectable_value(&mut selected, Locale::English, "English");
        });
    });

    ui.add_space(16.0);
    components::surface().show(ui, |ui| {
        components::section_title(ui, locale.about(), Some(locale.about_description()));
        ui.add_space(14.0);
        settings_row(ui, locale.app_version(), env!("CARGO_PKG_VERSION"));
        ui.separator();
        settings_row(ui, locale.protocol(), "MoQ / QUIC · mDNS");
    });

    (selected != locale).then_some(selected)
}

fn settings_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).size(13.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).size(13.0).strong().color(TEXT));
        });
    });
}
