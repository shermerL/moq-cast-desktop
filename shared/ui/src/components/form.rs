use egui::{
    ComboBox, CornerRadius, Id, Response, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, pos2,
    vec2,
};

use crate::{COLORS, ControlRole, Interaction, Radius, Size, resolve_control_visual};

use super::common::{
    color, effective_enabled, interaction, paint_focus, pointing_hand, sense, text_galley,
};

/// Display-only configuration for a switch.
#[derive(Clone, Copy, Debug)]
pub struct SwitchSpec<'a> {
    label: &'a str,
    enabled: bool,
    preview: Option<Interaction>,
}

impl<'a> SwitchSpec<'a> {
    /// Creates an enabled switch with an accessibility label.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            enabled: true,
            preview: None,
        }
    }

    /// Sets whether the switch accepts input.
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

/// Display-only configuration for a checkbox.
#[derive(Clone, Copy, Debug)]
pub struct CheckboxSpec<'a> {
    label: &'a str,
    enabled: bool,
    preview: Option<Interaction>,
}

impl<'a> CheckboxSpec<'a> {
    /// Creates an enabled checkbox with an accessibility label.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            enabled: true,
            preview: None,
        }
    }

    /// Sets whether the checkbox accepts input.
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

/// Error returned when a select specification or value is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectError {
    /// A select must contain at least one visible option.
    EmptyOptions,
    /// The selected index does not identify an available option.
    InvalidSelection,
}

impl std::fmt::Display for SelectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyOptions => formatter.write_str("a select requires at least one option"),
            Self::InvalidSelection => formatter.write_str("the selected index is out of range"),
        }
    }
}

impl std::error::Error for SelectError {}

/// Display-only configuration for a select control.
#[derive(Clone, Copy, Debug)]
pub struct SelectSpec<'a> {
    id: Id,
    label: &'a str,
    options: &'a [&'a str],
    enabled: bool,
    preview: Option<Interaction>,
}

impl<'a> SelectSpec<'a> {
    /// Creates a select with a stable ID, label, and non-empty option list.
    pub fn new(id: Id, label: &'a str, options: &'a [&'a str]) -> Result<Self, SelectError> {
        if options.is_empty() {
            return Err(SelectError::EmptyOptions);
        }
        Ok(Self {
            id,
            label,
            options,
            enabled: true,
            preview: None,
        })
    }

    /// Sets whether the select accepts input.
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

/// Renders a compact switch with a forty-point hit target.
pub fn switch(ui: &mut Ui, value: &mut bool, spec: SwitchSpec<'_>) -> Response {
    let enabled = effective_enabled(spec.enabled, spec.preview);
    let (rect, response) =
        ui.allocate_exact_size(vec2(Size::SWITCH[0], Size::CONTROL), sense(enabled));
    let mut response = pointing_hand(response, enabled);
    let state = interaction(&response, enabled, *value, spec.preview);
    if response.clicked() && enabled {
        *value = !*value;
        response.mark_changed();
    }
    let opacity = if enabled { 1.0 } else { Size::DISABLED_ALPHA };
    let track_rect =
        egui::Rect::from_center_size(rect.center(), vec2(Size::SWITCH[0], Size::SWITCH[1]));
    let (fill, border) = switch_track_visual(*value, state);
    ui.painter()
        .rect_filled(track_rect, CornerRadius::same(13), color(fill, opacity));
    ui.painter().rect_stroke(
        track_rect,
        CornerRadius::same(13),
        Stroke::new(Size::BORDER, color(border, opacity)),
        StrokeKind::Inside,
    );
    let thumb_x = if *value {
        track_rect.right() - 9.0
    } else {
        track_rect.left() + 9.0
    };
    ui.painter().circle_filled(
        pos2(thumb_x, track_rect.center().y),
        8.0,
        color(COLORS.surface, opacity),
    );
    ui.painter().circle_stroke(
        pos2(thumb_x, track_rect.center().y),
        8.0,
        Stroke::new(Size::BORDER, color(COLORS.border_strong, opacity)),
    );
    paint_focus(ui, track_rect, &response, state, 13.0);
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *value, spec.label));
    response
}

/// Renders a compact checkbox with a forty-point hit target.
pub fn checkbox(ui: &mut Ui, value: &mut bool, spec: CheckboxSpec<'_>) -> Response {
    let enabled = effective_enabled(spec.enabled, spec.preview);
    let (rect, response) =
        ui.allocate_exact_size(vec2(Size::CONTROL, Size::CONTROL), sense(enabled));
    let mut response = pointing_hand(response, enabled);
    let state = interaction(&response, enabled, *value, spec.preview);
    if response.clicked() && enabled {
        *value = !*value;
        response.mark_changed();
    }
    let opacity = if enabled { 1.0 } else { Size::DISABLED_ALPHA };
    let box_rect = egui::Rect::from_center_size(rect.center(), vec2(20.0, 20.0));
    let (fill, border) = checkbox_visual(*value, state);
    ui.painter().rect_filled(
        box_rect,
        CornerRadius::same(Radius::SM as u8),
        color(fill, opacity),
    );
    ui.painter().rect_stroke(
        box_rect,
        CornerRadius::same(Radius::SM as u8),
        Stroke::new(Size::BORDER, color(border, opacity)),
        StrokeKind::Inside,
    );
    if *value {
        let glyph_color = color(COLORS.surface, opacity);
        let glyph = text_galley(ui, "✓", crate::TypographyRole::Button, glyph_color);
        ui.painter().galley(
            pos2(
                rect.center().x - glyph.size().x / 2.0,
                rect.center().y - glyph.size().y / 2.0,
            ),
            glyph,
            glyph_color,
        );
    }
    paint_focus(ui, box_rect, &response, state, Radius::SM);
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *value, spec.label));
    response
}

/// Renders a select and rejects invalid state instead of displaying blank content.
pub fn select(
    ui: &mut Ui,
    selected: &mut usize,
    spec: SelectSpec<'_>,
) -> Result<Response, SelectError> {
    let Some(current) = spec.options.get(*selected).copied() else {
        return Err(SelectError::InvalidSelection);
    };
    let enabled = effective_enabled(spec.enabled, spec.preview);
    let forced = spec.preview.unwrap_or(Interaction::Rest);
    let visual = resolve_control_visual(ControlRole::Secondary, forced);
    let response = ui
        .scope(|ui| {
            let hovered = resolve_control_visual(ControlRole::Secondary, Interaction::Hovered);
            let pressed = resolve_control_visual(ControlRole::Secondary, Interaction::Pressed);
            let open = resolve_control_visual(ControlRole::Secondary, Interaction::Selected);
            ui.visuals_mut().widgets.inactive.bg_fill = color(visual.fill, visual.opacity);
            ui.visuals_mut().widgets.inactive.bg_stroke =
                Stroke::new(visual.border_width, color(visual.border, visual.opacity));
            ui.visuals_mut().widgets.inactive.fg_stroke.color = color(visual.text, visual.opacity);
            ui.visuals_mut().widgets.hovered.bg_fill = hovered.fill.into();
            ui.visuals_mut().widgets.hovered.bg_stroke =
                Stroke::new(hovered.border_width, hovered.border);
            ui.visuals_mut().widgets.active.bg_fill = pressed.fill.into();
            ui.visuals_mut().widgets.active.bg_stroke =
                Stroke::new(pressed.border_width, pressed.border);
            ui.visuals_mut().widgets.open.bg_fill = open.fill.into();
            ui.visuals_mut().widgets.open.bg_stroke = Stroke::new(open.border_width, open.border);
            ui.add_enabled_ui(enabled, |ui| {
                ComboBox::from_id_salt(spec.id)
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (index, option) in spec.options.iter().enumerate() {
                            ui.selectable_value(selected, index, *option);
                        }
                    })
                    .response
            })
            .inner
        })
        .inner;
    let response = pointing_hand(response, enabled);
    let state = interaction(&response, enabled, false, spec.preview);
    paint_focus(ui, response.rect, &response, state, Radius::MD);
    response.widget_info(|| WidgetInfo {
        enabled,
        label: Some(spec.label.to_owned()),
        current_text_value: Some(current.to_owned()),
        ..WidgetInfo::new(WidgetType::ComboBox)
    });
    Ok(response)
}

fn switch_track_visual(value: bool, state: Interaction) -> (crate::Color, crate::Color) {
    if state == Interaction::Disabled {
        return if value {
            (COLORS.brand, COLORS.brand)
        } else {
            (COLORS.surface_muted, COLORS.border)
        };
    }
    if value {
        return match state {
            Interaction::Hovered => (COLORS.brand_hover, COLORS.brand_pressed),
            Interaction::Pressed => (COLORS.brand_pressed, COLORS.text),
            _ => (COLORS.brand, COLORS.brand_pressed),
        };
    }
    match state {
        Interaction::Hovered => (COLORS.border, COLORS.brand),
        Interaction::Pressed => (COLORS.border_strong, COLORS.brand_pressed),
        _ => (COLORS.secondary_pressed, COLORS.border_strong),
    }
}

fn checkbox_visual(value: bool, state: Interaction) -> (crate::Color, crate::Color) {
    if value {
        return match state {
            Interaction::Hovered => (COLORS.brand_hover, COLORS.brand_pressed),
            Interaction::Pressed => (COLORS.brand_pressed, COLORS.text),
            _ => (COLORS.brand, COLORS.brand_pressed),
        };
    }
    match state {
        Interaction::Hovered => (COLORS.brand_soft, COLORS.brand),
        Interaction::Pressed => (COLORS.secondary_pressed, COLORS.brand_pressed),
        Interaction::Disabled => (COLORS.surface_muted, COLORS.border),
        _ => (COLORS.surface, COLORS.border_strong),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_rejects_empty_options_and_invalid_selection() {
        assert_eq!(
            SelectSpec::new(Id::new("empty"), "Mode", &[]).unwrap_err(),
            SelectError::EmptyOptions
        );
        egui::__run_test_ui(|ui| {
            let options = ["Auto"];
            let spec = SelectSpec::new(Id::new("mode"), "Mode", &options).unwrap();
            assert_eq!(
                select(ui, &mut 1, spec).unwrap_err(),
                SelectError::InvalidSelection
            );
        });
    }

    #[test]
    fn enabled_off_controls_remain_distinct_from_hovered_and_disabled() {
        let off = switch_track_visual(false, Interaction::Rest);
        let hovered = switch_track_visual(false, Interaction::Hovered);
        let pressed = switch_track_visual(false, Interaction::Pressed);
        let disabled = switch_track_visual(false, Interaction::Disabled);
        assert_ne!(off, hovered);
        assert_ne!(hovered, pressed);
        assert_ne!(off, disabled);
        assert_eq!(off.1, COLORS.border_strong);

        let checked = checkbox_visual(true, Interaction::Rest);
        let checked_hovered = checkbox_visual(true, Interaction::Hovered);
        let unchecked = checkbox_visual(false, Interaction::Rest);
        assert_ne!(checked, checked_hovered);
        assert_ne!(checked, unchecked);
    }
}
