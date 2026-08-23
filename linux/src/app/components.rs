//! Small reusable controls for the desktop pages.

use eframe::egui::{self, Color32, Frame, Margin, Response, RichText, Stroke};

use super::theme::{
    BORDER, BRAND, BRAND_DARK, BRAND_SOFT, ERROR, ERROR_SOFT, InteractionState, MUTED, RADIUS,
    SURFACE, SURFACE_MUTED, TEXT, apply_widget_visual,
};
use super::{AppSnapshot, DialRole, Locale, TransportState};

#[derive(Clone, Copy)]
pub(super) enum BadgeTone {
    Neutral,
    Info,
    Success,
    Error,
}

pub(super) fn surface() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(16))
}

pub(super) fn page_header(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(24.0).strong().color(TEXT));
    if !description.is_empty() {
        ui.label(RichText::new(description).size(13.0).color(MUTED));
    }
    ui.add_space(18.0);
}

pub(super) fn mesh_summary(ui: &mut egui::Ui, locale: Locale, snapshot: &AppSnapshot) {
    let outbound = snapshot
        .peers
        .values()
        .filter(|peer| {
            peer.dial_role == DialRole::Outbound && peer.transport == TransportState::Connected
        })
        .count();
    ui.label(
        RichText::new(format!(
            "{} {outbound}  ·  {} {}",
            locale.outbound_sessions(),
            locale.inbound_sessions(),
            snapshot.inbound_session_count
        ))
        .size(11.0)
        .color(MUTED),
    );
}

pub(super) fn status_badge(ui: &mut egui::Ui, label: &str, tone: BadgeTone) -> Response {
    let (fill, stroke, text) = match tone {
        BadgeTone::Neutral => (SURFACE_MUTED, BORDER, MUTED),
        BadgeTone::Info => (
            Color32::from_rgb(235, 242, 248),
            Color32::from_rgb(184, 207, 224),
            TEXT,
        ),
        BadgeTone::Success => (BRAND_SOFT, Color32::from_rgb(172, 215, 204), BRAND_DARK),
        BadgeTone::Error => (ERROR_SOFT, Color32::from_rgb(239, 184, 181), ERROR),
    };
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(RADIUS)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(11.0).strong().color(text))
        })
        .response
}

pub(super) fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(BRAND)
            .stroke(Stroke::new(1.0, BRAND_DARK))
            .corner_radius(RADIUS)
            .min_size(egui::vec2(112.0, 36.0)),
    )
}

pub(super) fn secondary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(TEXT))
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(RADIUS)
            .min_size(egui::vec2(108.0, 36.0)),
    )
}

pub(super) fn selection_checkbox(
    ui: &mut egui::Ui,
    checked: &mut bool,
    label: &str,
    enabled: bool,
) -> Response {
    ui.scope(|ui| {
        if *checked {
            let widgets = &mut ui.visuals_mut().widgets;
            apply_widget_visual(&mut widgets.inactive, true, InteractionState::Rest);
            apply_widget_visual(&mut widgets.hovered, true, InteractionState::Hovered);
            apply_widget_visual(&mut widgets.active, true, InteractionState::Active);
        }
        ui.add_enabled(enabled, egui::Checkbox::new(checked, label))
    })
    .inner
}

pub(super) fn danger_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(ERROR))
            .fill(ERROR_SOFT)
            .stroke(Stroke::new(1.0, Color32::from_rgb(239, 184, 181)))
            .corner_radius(RADIUS)
            .min_size(egui::vec2(108.0, 36.0)),
    )
}

pub(super) fn empty_state(ui: &mut egui::Ui, title: &str, description: &str, busy: bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 144.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(24.0);
            if busy {
                ui.spinner();
            }
            ui.label(RichText::new(title).size(17.0).strong().color(TEXT));
            ui.label(RichText::new(description).size(13.0).color(MUTED));
        },
    );
}

pub(super) fn status_line(ui: &mut egui::Ui, label: &str, tone: BadgeTone) {
    let color = match tone {
        BadgeTone::Neutral => MUTED,
        BadgeTone::Info => Color32::from_rgb(65, 105, 135),
        BadgeTone::Success => BRAND,
        BadgeTone::Error => ERROR,
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("●").size(9.0).color(color));
        ui.label(RichText::new(label).size(12.0).color(TEXT));
    });
}

pub(super) fn error_banner(ui: &mut egui::Ui, error: &str, action_label: Option<&str>) -> bool {
    let mut clicked = false;
    Frame::new()
        .fill(ERROR_SOFT)
        .stroke(Stroke::new(1.0, Color32::from_rgb(239, 184, 181)))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(error).size(13.0).color(ERROR));
                if let Some(label) = action_label {
                    clicked = secondary_button(ui, label, true).clicked();
                }
            });
        });
    clicked
}

pub(super) fn section_title(ui: &mut egui::Ui, title: &str, description: Option<&str>) {
    ui.label(RichText::new(title).size(16.0).strong().color(TEXT));
    if let Some(description) = description {
        ui.label(RichText::new(description).size(12.0).color(MUTED));
    }
}
