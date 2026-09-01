use egui::{Color32, CornerRadius, Id, Rect, Response, Ui, WidgetInfo, WidgetType, pos2, vec2};

use crate::{COLORS, ControlRole, Interaction, Size, Spacing, TypographyRole};

use super::common::{paint_centered_text, paint_focus, paint_surface, resolve, sense, text_width};

/// Display-only configuration for a top-navigation item.
#[derive(Clone, Copy, Debug)]
pub struct NavItemSpec<'a> {
    id: Id,
    label: &'a str,
    accessible_label: &'a str,
    selected: bool,
    enabled: bool,
    preview: Option<Interaction>,
}

impl<'a> NavItemSpec<'a> {
    /// Creates a navigation item with a stable ID and display label.
    pub fn new(id: Id, label: &'a str) -> Self {
        Self {
            id,
            label,
            accessible_label: label,
            selected: false,
            enabled: true,
            preview: None,
        }
    }

    /// Sets a distinct accessibility label.
    pub fn accessible_label(mut self, label: &'a str) -> Self {
        self.accessible_label = label;
        self
    }

    /// Sets whether this item identifies the active destination.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets whether this item accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Forces a deterministic state for catalogs and visual tests.
    pub fn preview_interaction(mut self, state: Interaction) -> Self {
        self.preview = Some(state);
        self
    }
}

/// Renders one top-navigation destination.
pub fn nav_item(ui: &mut Ui, spec: NavItemSpec<'_>) -> Response {
    let width = text_width(ui, spec.label, TypographyRole::Button) + Spacing::XL * 2.0;
    let rect = ui.allocate_space(vec2(width, Size::NAV)).1;
    let response = ui.interact(rect, spec.id, sense(spec.enabled));
    let (state, visual) = resolve(
        &response,
        ControlRole::Nav,
        spec.enabled,
        spec.selected,
        spec.preview,
    );
    paint_surface(ui, rect, visual, 0);
    paint_centered_text(
        ui,
        rect,
        spec.label,
        TypographyRole::Button,
        visual.text,
        visual.opacity,
    );
    if spec.selected || state == Interaction::Selected {
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(rect.left(), rect.bottom() - Size::NAV_UNDERLINE),
                rect.right_bottom(),
            ),
            CornerRadius::ZERO,
            Color32::from(COLORS.brand).gamma_multiply(visual.opacity),
        );
    }
    paint_focus(ui, rect, &response, state, 0.0);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            spec.enabled,
            spec.selected,
            spec.accessible_label,
        )
    });
    response
}
