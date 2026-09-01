use std::borrow::Cow;

use egui::{
    Context, FontData, FontFamily, Margin, Stroke, Visuals,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
};

use crate::{COLORS, Radius, Size, Spacing};

/// The shared light visual theme for desktop UI components.
#[derive(Clone, Copy, Debug, Default)]
pub struct Theme;

impl Theme {
    /// Applies shared neutral visuals and spacing to an egui context.
    pub fn apply(self, context: &Context) {
        context.set_theme(egui::Theme::Light);
        context.style_mut_of(egui::Theme::Light, |style| {
            let mut visuals = Visuals::light();
            visuals.panel_fill = COLORS.canvas.into();
            visuals.window_fill = COLORS.surface.into();
            visuals.window_stroke = Stroke::new(Size::BORDER, COLORS.border);
            visuals.window_corner_radius = egui::CornerRadius::same(Radius::LG as u8);
            visuals.selection.bg_fill = COLORS.brand_soft.into();
            visuals.selection.stroke = Stroke::new(Size::BORDER, COLORS.brand);
            visuals.hyperlink_color = COLORS.brand.into();
            visuals.faint_bg_color = COLORS.surface_muted.into();
            visuals.extreme_bg_color = COLORS.surface.into();
            style.visuals = visuals;
            style.visuals.disabled_alpha = Size::DISABLED_ALPHA;
            style.spacing.item_spacing = egui::vec2(Spacing::SM, Spacing::SM);
            style.spacing.button_padding = egui::vec2(Spacing::MD, Spacing::SM);
            style.spacing.window_margin = Margin::same(Spacing::LG as i8);
            style.spacing.interact_size.y = Size::CONTROL;
        });
    }
}

/// Installs Noto Sans SC as proportional primary and monospace CJK fallback.
pub fn install_ui_font(context: &Context, bytes: Cow<'static, [u8]>) {
    let data = match bytes {
        Cow::Borrowed(bytes) => FontData::from_static(bytes),
        Cow::Owned(bytes) => FontData::from_owned(bytes),
    };
    context.add_font(FontInsert::new(
        "Noto Sans SC Variable",
        data,
        font_families(),
    ));
}

fn font_families() -> Vec<InsertFontFamily> {
    vec![
        InsertFontFamily {
            family: FontFamily::Proportional,
            priority: FontPriority::Highest,
        },
        InsertFontFamily {
            family: FontFamily::Monospace,
            priority: FontPriority::Lowest,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_font_keeps_default_latin_monospace_primary() {
        let families = font_families();
        assert_eq!(families.len(), 2);
        assert_eq!(families[0].family, FontFamily::Proportional);
        assert!(matches!(families[0].priority, FontPriority::Highest));
        assert_eq!(families[1].family, FontFamily::Monospace);
        assert!(matches!(families[1].priority, FontPriority::Lowest));
    }
}
