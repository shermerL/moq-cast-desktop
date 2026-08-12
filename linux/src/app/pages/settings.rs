//! Local settings page.

use eframe::egui;

use super::super::Locale;
use super::super::components;

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

    (selected != locale).then_some(selected)
}
