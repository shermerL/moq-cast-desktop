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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum InteractionState {
    Rest,
    Selected,
    Hovered,
    Active,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct InteractionVisual {
    pub fill: Color32,
    pub stroke: Stroke,
    pub text: Color32,
}

pub(super) fn interaction_visual(state: InteractionState) -> InteractionVisual {
    match state {
        InteractionState::Rest => InteractionVisual {
            fill: SURFACE,
            stroke: Stroke::new(1.0, BORDER),
            text: TEXT,
        },
        InteractionState::Selected => InteractionVisual {
            fill: BRAND_SOFT,
            stroke: Stroke::new(1.0, BRAND),
            text: BRAND_DARK,
        },
        InteractionState::Hovered => InteractionVisual {
            fill: Color32::from_rgb(238, 246, 243),
            stroke: Stroke::new(1.0, BRAND),
            text: BRAND_DARK,
        },
        InteractionState::Active => InteractionVisual {
            fill: Color32::from_rgb(229, 242, 238),
            stroke: Stroke::new(2.0, BRAND_DARK),
            text: BRAND_DARK,
        },
        InteractionState::Disabled => InteractionVisual {
            fill: SURFACE_MUTED,
            stroke: Stroke::new(1.0, BORDER),
            text: MUTED,
        },
    }
}

pub(super) fn selectable_visual(selected: bool, state: InteractionState) -> InteractionVisual {
    let mut visual = interaction_visual(state);
    if selected && state != InteractionState::Disabled {
        visual.fill = interaction_visual(InteractionState::Selected).fill;
    }
    visual
}

pub(super) fn apply_widget_visual(
    target: &mut egui::style::WidgetVisuals,
    selected: bool,
    state: InteractionState,
) {
    let visual = selectable_visual(selected, state);
    target.bg_fill = visual.fill;
    target.weak_bg_fill = visual.fill;
    target.bg_stroke = visual.stroke;
    target.fg_stroke = Stroke::new(1.0, visual.text);
    target.corner_radius = RADIUS;
}

pub(super) fn configure(context: &egui::Context) {
    let mut style = (*context.style_of(egui::Theme::Light)).clone();
    let mut visuals = egui::Visuals::light();
    let selected = interaction_visual(InteractionState::Selected);
    visuals.panel_fill = PAGE;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = SURFACE_MUTED;
    visuals.faint_bg_color = SURFACE_MUTED;
    visuals.selection.bg_fill = selected.fill;
    visuals.selection.stroke = Stroke::new(1.0, selected.text);
    visuals.hyperlink_color = BRAND;
    visuals.error_fg_color = ERROR;
    apply_widget_visual(
        &mut visuals.widgets.noninteractive,
        false,
        InteractionState::Disabled,
    );
    apply_widget_visual(&mut visuals.widgets.inactive, false, InteractionState::Rest);
    apply_widget_visual(
        &mut visuals.widgets.hovered,
        false,
        InteractionState::Hovered,
    );
    apply_widget_visual(&mut visuals.widgets.active, false, InteractionState::Active);
    apply_widget_visual(&mut visuals.widgets.open, false, InteractionState::Active);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 36.0;
    context.set_style_of(egui::Theme::Light, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_states_have_distinct_semantics() {
        let rest = interaction_visual(InteractionState::Rest);
        let selected = interaction_visual(InteractionState::Selected);
        let hovered = interaction_visual(InteractionState::Hovered);
        let active = interaction_visual(InteractionState::Active);
        let disabled = interaction_visual(InteractionState::Disabled);

        assert_eq!(rest.fill, SURFACE);
        assert_ne!(selected.fill, rest.fill);
        assert_ne!(hovered.fill, rest.fill);
        assert!(active.stroke.width > hovered.stroke.width);
        assert_eq!(disabled.text, MUTED);
    }

    #[test]
    fn selected_state_uses_a_soft_fill_and_dark_text() {
        let selected = interaction_visual(InteractionState::Selected);

        assert_eq!(selected.fill, BRAND_SOFT);
        assert_eq!(selected.text, BRAND_DARK);
        assert_ne!(selected.fill, BRAND);
    }

    #[test]
    fn checked_interactive_states_keep_selected_fill_and_state_semantics() {
        for state in [
            InteractionState::Rest,
            InteractionState::Hovered,
            InteractionState::Active,
        ] {
            let base = interaction_visual(state);
            let checked = selectable_visual(true, state);

            assert_eq!(checked.fill, BRAND_SOFT);
            assert_eq!(checked.stroke, base.stroke);
            assert_eq!(checked.text, base.text);
        }
    }

    #[test]
    fn checked_disabled_state_stays_neutral() {
        assert_eq!(
            selectable_visual(true, InteractionState::Disabled),
            interaction_visual(InteractionState::Disabled)
        );
    }

    #[test]
    fn configured_selectable_buttons_use_surface_and_selected_tokens() {
        let context = egui::Context::default();
        configure(&context);
        let style = context.style_of(egui::Theme::Light);

        assert_eq!(style.visuals.widgets.inactive.weak_bg_fill, SURFACE);
        assert_eq!(style.visuals.selection.bg_fill, BRAND_SOFT);
        assert_eq!(style.visuals.selection.stroke.color, BRAND_DARK);
    }
}
