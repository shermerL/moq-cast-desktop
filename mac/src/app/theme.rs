//! Light visual tokens shared by the M2 macOS interface.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub(super) const BRAND: Color32 = Color32::from_rgb(0x0c, 0x7c, 0x68);
pub(super) const BRAND_PRESSED: Color32 = Color32::from_rgb(0x07, 0x5b, 0x50);
pub(super) const BRAND_SOFT: Color32 = Color32::from_rgb(0xdc, 0xef, 0xe9);
pub(super) const CANVAS: Color32 = Color32::from_rgb(0xf5, 0xf6, 0xf7);
pub(super) const SURFACE: Color32 = Color32::WHITE;
pub(super) const SURFACE_MUTED: Color32 = Color32::from_rgb(0xf0, 0xf3, 0xf2);
pub(super) const BORDER: Color32 = Color32::from_rgb(0xd9, 0xde, 0xdc);
pub(super) const BORDER_STRONG: Color32 = Color32::from_rgb(0xb8, 0xc1, 0xbd);
pub(super) const TEXT: Color32 = Color32::from_rgb(0x1d, 0x23, 0x21);
pub(super) const MUTED: Color32 = Color32::from_rgb(0x68, 0x73, 0x6f);
pub(super) const WARNING: Color32 = Color32::from_rgb(0x9a, 0x62, 0x00);
pub(super) const WARNING_SOFT: Color32 = Color32::from_rgb(0xff, 0xf5, 0xdc);
pub(super) const ERROR: Color32 = Color32::from_rgb(0xba, 0x1a, 0x1a);
pub(super) const ERROR_SOFT: Color32 = Color32::from_rgb(0xfd, 0xec, 0xeb);
pub(super) const FOCUS: Color32 = Color32::from_rgb(0x00, 0x7a, 0xff);

pub(super) fn apply(context: &egui::Context) {
    context.set_theme(egui::Theme::Light);
    let mut style = (*context.style_of(egui::Theme::Light)).clone();
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(22.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(13.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.combo_width = 180.0;

    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(MUTED);
    visuals.panel_fill = CANVAS;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.faint_bg_color = SURFACE_MUTED;
    visuals.extreme_bg_color = SURFACE;
    visuals.text_edit_bg_color = Some(SURFACE);
    visuals.selection.bg_fill = BRAND_SOFT;
    visuals.selection.stroke = Stroke::new(1.0, BRAND_PRESSED);
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.disabled_alpha = 0.46;

    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.weak_bg_fill = SURFACE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = SURFACE_MUTED;
    visuals.widgets.hovered.weak_bg_fill = SURFACE_MUTED;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_fill = BRAND_SOFT;
    visuals.widgets.active.weak_bg_fill = BRAND_SOFT;
    visuals.widgets.active.bg_stroke = Stroke::new(2.0, FOCUS);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, BRAND_PRESSED);
    visuals.widgets.open = visuals.widgets.active;
    style.visuals = visuals;
    context.set_style_of(egui::Theme::Light, style);
}
