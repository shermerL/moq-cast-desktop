use egui::{Color32, CornerRadius, Id, Rect, Response, Ui, WidgetInfo, WidgetType, pos2, vec2};

use crate::{COLORS, ControlRole, Interaction, Size, TypographyRole, resolve_control_visual};

use super::common::{
    effective_enabled, interaction, paint_centered_text, paint_focus, paint_surface, pointing_hand,
    sense, text_width,
};

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
    let enabled = effective_enabled(spec.enabled, spec.preview);
    let width =
        text_width(ui, spec.label, TypographyRole::Button) + Size::NAV_HORIZONTAL_PADDING * 2.0;
    let rect = ui.allocate_space(vec2(width, Size::NAV)).1;
    let response = ui.interact(rect, spec.id, sense(enabled));
    let response = pointing_hand(response, enabled);
    let input_state = interaction(&response, enabled, spec.selected, spec.preview);
    let state = nav_visual_state(spec.selected, input_state);
    let visual = resolve_control_visual(ControlRole::Nav, state);
    paint_surface(ui, rect, visual, Size::NAV_RADIUS as u8);
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
    paint_focus(ui, rect, &response, state, Size::NAV_RADIUS);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            enabled,
            spec.selected,
            spec.accessible_label,
        )
    });
    response
}

fn nav_visual_state(selected: bool, input_state: Interaction) -> Interaction {
    if selected {
        Interaction::Selected
    } else {
        input_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_navigation_keeps_its_active_surface_during_pointer_states() {
        for state in [
            Interaction::Rest,
            Interaction::Hovered,
            Interaction::Pressed,
            Interaction::Focused,
        ] {
            assert_eq!(nav_visual_state(true, state), Interaction::Selected);
        }
        assert_eq!(
            nav_visual_state(false, Interaction::Hovered),
            Interaction::Hovered
        );
    }
}
