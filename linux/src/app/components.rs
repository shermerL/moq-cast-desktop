//! Small reusable controls for the desktop pages.

use eframe::egui::{self, Color32, Frame, Margin, Response, RichText, Stroke};

use super::theme::{
    BORDER, BRAND, BRAND_DARK, BRAND_SOFT, ERROR, ERROR_SOFT, MUTED, RADIUS, SURFACE,
    SURFACE_MUTED, TEXT,
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
        .inner_margin(Margin::same(20))
}

pub(super) fn muted_surface() -> Frame {
    Frame::new()
        .fill(SURFACE_MUTED)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(20))
}

pub(super) fn page_header(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    locale: Locale,
    snapshot: &AppSnapshot,
) {
    ui.horizontal_wrapped(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(28.0).strong().color(TEXT));
            if !description.is_empty() {
                ui.label(RichText::new(description).size(14.0).color(MUTED));
            }
        });
        ui.add_space(16.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let outbound = snapshot
                .peers
                .values()
                .filter(|peer| {
                    peer.dial_role == DialRole::Outbound
                        && peer.transport == TransportState::Connected
                })
                .count();
            let summary = format!(
                "{}: {outbound}  ·  {}: {}",
                locale.outbound_sessions(),
                locale.inbound_sessions(),
                snapshot.inbound_session_count
            );
            status_badge(
                ui,
                &summary,
                if snapshot.has_mesh_session() {
                    BadgeTone::Success
                } else {
                    BadgeTone::Neutral
                },
            );
        });
    });
    ui.add_space(22.0);
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
            .min_size(egui::vec2(132.0, 40.0)),
    )
}

pub(super) fn secondary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(TEXT))
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(RADIUS)
            .min_size(egui::vec2(120.0, 40.0)),
    )
}

pub(super) fn danger_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(ERROR))
            .fill(ERROR_SOFT)
            .stroke(Stroke::new(1.0, Color32::from_rgb(239, 184, 181)))
            .corner_radius(RADIUS)
            .min_size(egui::vec2(120.0, 40.0)),
    )
}

pub(super) fn empty_state(ui: &mut egui::Ui, title: &str, description: &str, busy: bool) {
    muted_surface().show(ui, |ui| {
        ui.set_min_height(220.0);
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            if busy {
                ui.spinner();
            }
            ui.label(RichText::new(title).size(18.0).strong().color(TEXT));
            ui.label(RichText::new(description).size(13.0).color(MUTED));
        });
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
