use egui::{Align, Layout, Ui};

use crate::{COLORS, Size, Spacing, TypographyRole, typography};

/// Renders a page title and optional supporting copy with the frozen header spacing.
pub fn page_header(ui: &mut Ui, title: &str, description: Option<&str>) {
    let item_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = Spacing::NONE;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), Size::PAGE_HEADER_MIN),
        Layout::top_down(Align::Min),
        |ui| {
            ui.label(typography(
                title,
                TypographyRole::PageTitle,
                COLORS.text.into(),
            ));
            if let Some(description) = description {
                ui.add_space(Size::PAGE_TITLE_SPACING);
                ui.label(typography(
                    description,
                    TypographyRole::Help,
                    COLORS.muted.into(),
                ));
            }
        },
    );
    ui.add_space(Size::PAGE_HEADER_SPACING);
    ui.spacing_mut().item_spacing.y = item_spacing;
}

/// Renders a section title and optional supporting copy.
pub fn section_header(ui: &mut Ui, title: &str, description: Option<&str>) {
    let item_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = Spacing::NONE;
    ui.label(typography(
        title,
        TypographyRole::Section,
        COLORS.text.into(),
    ));
    if let Some(description) = description {
        ui.add_space(Spacing::XS);
        ui.label(typography(
            description,
            TypographyRole::Help,
            COLORS.muted.into(),
        ));
    }
    ui.add_space(Size::SECTION_CONTENT_SPACING);
    ui.spacing_mut().item_spacing.y = item_spacing;
}

/// Separates major page sections with one divider and the shared section gap.
pub fn major_section_break(ui: &mut Ui) {
    let item_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = Spacing::NONE;
    ui.separator();
    ui.add_space(Size::MAJOR_SECTION_SPACING);
    ui.spacing_mut().item_spacing.y = item_spacing;
}
