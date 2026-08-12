//! Shared visual tokens for the native interface.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub(super) const PAGE: Color32 = Color32::from_rgb(247, 248, 249);
pub(super) const SURFACE: Color32 = Color32::WHITE;
pub(super) const SURFACE_MUTED: Color32 = Color32::from_rgb(245, 246, 247);
pub(super) const BORDER: Color32 = Color32::from_rgb(225, 228, 231);
pub(super) const BRAND: Color32 = Color32::from_rgb(12, 124, 104);
pub(super) const BRAND_DARK: Color32 = Color32::from_rgb(7, 91, 80);
pub(super) const BRAND_SOFT: Color32 = Color32::from_rgb(220, 239, 233);
pub(super) const TEXT: Color32 = Color32::from_rgb(22, 32, 29);
pub(super) const MUTED: Color32 = Color32::from_rgb(100, 113, 109);
pub(super) const ERROR: Color32 = Color32::from_rgb(186, 26, 26);
pub(super) const ERROR_SOFT: Color32 = Color32::from_rgb(253, 236, 235);
pub(super) const RADIUS: CornerRadius = CornerRadius::same(8);

pub(super) fn configure(context: &egui::Context) {
    let mut style = (*context.style_of(egui::Theme::Light)).clone();
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = PAGE;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = SURFACE_MUTED;
    visuals.faint_bg_color = SURFACE_MUTED;
    visuals.selection.bg_fill = BRAND;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.hyperlink_color = BRAND;
    visuals.error_fg_color = ERROR;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.corner_radius = RADIUS;
    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.weak_bg_fill = SURFACE_MUTED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.corner_radius = RADIUS;
    visuals.widgets.hovered.bg_fill = BRAND_SOFT;
    visuals.widgets.hovered.weak_bg_fill = BRAND_SOFT;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, BRAND);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, BRAND_DARK);
    visuals.widgets.hovered.corner_radius = RADIUS;
    visuals.widgets.active.bg_fill = BRAND;
    visuals.widgets.active.weak_bg_fill = BRAND;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, BRAND_DARK);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = RADIUS;
    visuals.widgets.open.corner_radius = RADIUS;
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 36.0;
    context.set_style_of(egui::Theme::Light, style);
}
