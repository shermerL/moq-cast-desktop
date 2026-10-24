//! Shared settings presentation with Linux-owned diagnostics actions.
use super::super::{Locale, diagnostics::DiagnosticsUi};
use eframe::egui;
use moqcast_diagnostics::FileStatus;
use moqcast_ui::v2::{self, Action, ActionTone, StatusTone};
pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    theme: &mut egui::ThemePreference,
    developer: &mut bool,
    diagnostics: &mut DiagnosticsUi,
) -> Option<Locale> {
    let mut selected = locale;
    let height = (ui.ctx().content_rect().height() - 160.0)
        .clamp(120.0, if *developer { 368.0 } else { 256.0 });
    egui::ScrollArea::vertical()
        .id_salt("v2-settings-content")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            v2::settings_row(ui, locale.text("外观", "Appearance"), |ui| {
                for (value, label) in [
                    (egui::ThemePreference::Dark, locale.text("深色", "Dark")),
                    (egui::ThemePreference::Light, locale.text("浅色", "Light")),
                    (egui::ThemePreference::System, locale.text("系统", "System")),
                ] {
                    if ui.selectable_value(theme, value, label).changed() {
                        ui.ctx().set_theme(*theme);
                    }
                }
            });
            v2::settings_row(ui, locale.language(), |ui| {
                egui::ComboBox::from_id_salt("v2-language")
                    .selected_text(if selected == Locale::Chinese {
                        "简体中文"
                    } else {
                        "English"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, Locale::Chinese, "简体中文");
                        ui.selectable_value(&mut selected, Locale::English, "English");
                    });
            });
            let available = matches!(diagnostics.file_status(), FileStatus::Available(_));
            v2::settings_row(ui, locale.text("日志", "Logs"), |ui| {
                if (Action {
                    label: locale.export_logs(),
                    size: egui::vec2(100.0, 40.0),
                    enabled: available,
                    tone: ActionTone::Neutral,
                })
                .show(ui)
                .clicked()
                {
                    diagnostics.choose_export();
                }
                if (Action {
                    label: locale.text("打开目录", "Open folder"),
                    size: egui::vec2(100.0, 40.0),
                    enabled: available,
                    tone: ActionTone::Neutral,
                })
                .show(ui)
                .clicked()
                {
                    diagnostics.open_directory();
                }
            });
            if !available {
                v2::notice(
                    ui,
                    StatusTone::Warning,
                    locale.file_diagnostics_unavailable_short(),
                );
            }
            if let Some(status) = diagnostics.localized_status(locale) {
                ui.label(status);
            }
            let previous = *developer;
            v2::settings_row(ui, locale.developer_mode(), |ui| {
                ui.checkbox(developer, "")
                    .on_hover_text(locale.developer_mode());
            });
            if previous && !*developer {
                diagnostics.hide_window();
            }
            if *developer {
                if diagnostics.dropped_count() > 0 {
                    v2::notice(
                        ui,
                        StatusTone::Warning,
                        &format!(
                            "{}: {}",
                            locale.text("已丢弃日志", "Dropped log entries"),
                            diagnostics.dropped_count()
                        ),
                    );
                }
                let mut detailed = diagnostics.detailed();
                v2::settings_row(ui, locale.detailed_diagnostics(), |ui| {
                    if ui.checkbox(&mut detailed, "").changed() {
                        diagnostics.set_detailed(detailed);
                    }
                });
                v2::settings_row(ui, locale.show_logs(), |ui| {
                    if v2::action(
                        ui,
                        locale.text("打开日志", "Open logs"),
                        egui::vec2(108.0, 40.0),
                        true,
                    )
                    .clicked()
                    {
                        diagnostics.set_visible(true);
                    }
                });
            }
        });
    (selected != locale).then_some(selected)
}
