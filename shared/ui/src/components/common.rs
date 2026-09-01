use egui::{
    Color32, CornerRadius, CursorIcon, Rect, Response, Stroke, StrokeKind, Ui, WidgetText, pos2,
};

use crate::{
    COLORS, Color, ControlRole, Interaction, Radius, ResolvedVisual, Size, TypographyRole,
    resolve_control_visual, typography,
};

pub(crate) fn interaction(
    response: &Response,
    enabled: bool,
    selected: bool,
    preview: Option<Interaction>,
) -> Interaction {
    if let Some(preview) = preview {
        preview
    } else if !enabled {
        Interaction::Disabled
    } else if response.is_pointer_button_down_on() {
        Interaction::Pressed
    } else if response.hovered() {
        Interaction::Hovered
    } else if response.has_focus() {
        Interaction::Focused
    } else if selected {
        Interaction::Selected
    } else {
        Interaction::Rest
    }
}

pub(crate) fn effective_enabled(enabled: bool, preview: Option<Interaction>) -> bool {
    enabled && preview != Some(Interaction::Disabled)
}

pub(crate) fn pointing_hand(response: Response, enabled: bool) -> Response {
    match hover_cursor(enabled) {
        Some(cursor) => response.on_hover_cursor(cursor),
        None => response,
    }
}

fn hover_cursor(enabled: bool) -> Option<CursorIcon> {
    enabled.then_some(CursorIcon::PointingHand)
}

pub(crate) fn resolve(
    response: &Response,
    role: ControlRole,
    enabled: bool,
    selected: bool,
    preview: Option<Interaction>,
) -> (Interaction, ResolvedVisual) {
    let state = interaction(response, enabled, selected, preview);
    (state, resolve_control_visual(role, state))
}

pub(crate) fn paint_surface(ui: &Ui, rect: Rect, visual: ResolvedVisual, radius: u8) {
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(radius),
        color(visual.fill, visual.opacity),
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(visual.border_width, color(visual.border, visual.opacity)),
        StrokeKind::Inside,
    );
}

pub(crate) fn paint_focus(
    ui: &Ui,
    rect: Rect,
    response: &Response,
    state: Interaction,
    radius: f32,
) {
    if response.has_focus() || state == Interaction::Focused {
        ui.painter().rect_stroke(
            rect.expand(Size::FOCUS_OUTSET),
            CornerRadius::same((radius + Size::FOCUS_OUTSET) as u8),
            Stroke::new(Size::FOCUS, Color32::from(COLORS.focus)),
            StrokeKind::Outside,
        );
    }
}

pub(crate) fn text_width(ui: &Ui, text: &str, role: TypographyRole) -> f32 {
    text_galley(ui, text, role, COLORS.text.into()).size().x
}

pub(crate) fn text_galley(
    ui: &Ui,
    text: &str,
    role: TypographyRole,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    WidgetText::from(typography(text, role, color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::FontSelection::Default,
    )
}

pub(crate) fn paint_centered_text(
    ui: &Ui,
    rect: Rect,
    text: &str,
    role: TypographyRole,
    color_value: Color,
    opacity: f32,
) {
    let color = color(color_value, opacity);
    let galley = text_galley(ui, text, role, color);
    ui.painter().galley(
        pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

pub(crate) fn color(value: Color, opacity: f32) -> Color32 {
    Color32::from(value).gamma_multiply(opacity)
}

pub(crate) fn sense(enabled: bool) -> egui::Sense {
    if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    }
}

pub(crate) const CONTROL_RADIUS: f32 = Radius::MD;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_enabled_controls_request_the_pointing_hand_cursor() {
        assert_eq!(hover_cursor(true), Some(CursorIcon::PointingHand));
        assert_eq!(hover_cursor(false), None);
        assert!(!effective_enabled(true, Some(Interaction::Disabled)));
        assert!(effective_enabled(true, Some(Interaction::Hovered)));
    }
}
