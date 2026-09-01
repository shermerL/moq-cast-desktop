use egui::{Id, Response, Ui, WidgetInfo, WidgetType, vec2};

use crate::{ControlRole, Interaction, Size, Spacing, TypographyRole};

use super::common::{
    CONTROL_RADIUS, paint_centered_text, paint_focus, paint_surface, resolve, sense, text_width,
};

/// Display-only configuration for a text control button.
#[derive(Clone, Copy, Debug)]
pub struct ButtonSpec<'a> {
    label: &'a str,
    accessible_label: &'a str,
    role: ControlRole,
    enabled: bool,
    selected: bool,
    id: Option<Id>,
    min_width: f32,
    preview: Option<Interaction>,
}

impl<'a> ButtonSpec<'a> {
    /// Creates a button spec with an accessible label equal to its visible label.
    pub fn new(label: &'a str, role: ControlRole) -> Self {
        Self {
            label,
            accessible_label: label,
            role,
            enabled: true,
            selected: false,
            id: None,
            min_width: 108.0,
            preview: None,
        }
    }

    /// Sets the accessibility label without changing visible content.
    pub fn accessible_label(mut self, label: &'a str) -> Self {
        self.accessible_label = label;
        self
    }

    /// Sets whether the control accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the active or selected visual and accessibility state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Assigns a stable interaction ID, including safe dialog focus targets.
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the minimum control width in logical points.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width.max(Size::CONTROL);
        self
    }

    /// Forces a deterministic state for catalogs and visual tests.
    pub fn preview_interaction(mut self, state: Interaction) -> Self {
        self.preview = Some(state);
        self
    }
}

/// Display-only configuration for a square icon button.
#[derive(Clone, Copy, Debug)]
pub struct IconButtonSpec<'a> {
    button: ButtonSpec<'a>,
}

impl<'a> IconButtonSpec<'a> {
    /// Creates a light-surface icon button with a required accessible label.
    pub fn new(icon: &'a str, accessible_label: &'a str) -> Self {
        Self {
            button: ButtonSpec::new(icon, ControlRole::Icon)
                .accessible_label(accessible_label)
                .min_width(Size::CONTROL),
        }
    }

    /// Creates a dark player-toolbar icon button with a required accessible label.
    pub fn player(icon: &'a str, accessible_label: &'a str) -> Self {
        Self {
            button: ButtonSpec::new(icon, ControlRole::PlayerIcon)
                .accessible_label(accessible_label)
                .min_width(Size::CONTROL),
        }
    }

    /// Sets whether the icon control accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.button = self.button.enabled(enabled);
        self
    }

    /// Sets the icon control's active state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }

    /// Assigns a stable interaction ID.
    pub fn id(mut self, id: Id) -> Self {
        self.button = self.button.id(id);
        self
    }

    /// Forces a deterministic state for catalogs and visual tests.
    pub fn preview_interaction(mut self, state: Interaction) -> Self {
        self.button = self.button.preview_interaction(state);
        self
    }
}

/// Renders a text button from a business-neutral display spec.
pub fn control_button(ui: &mut Ui, spec: ButtonSpec<'_>) -> Response {
    let width = spec
        .min_width
        .max(text_width(ui, spec.label, TypographyRole::Button) + Spacing::XL);
    let (rect, response) = if let Some(id) = spec.id {
        let rect = ui.allocate_space(vec2(width, Size::CONTROL)).1;
        let response = ui.interact(rect, id, sense(spec.enabled));
        (rect, response)
    } else {
        ui.allocate_exact_size(vec2(width, Size::CONTROL), sense(spec.enabled))
    };
    let (state, visual) = resolve(
        &response,
        spec.role,
        spec.enabled,
        spec.selected,
        spec.preview,
    );
    paint_surface(ui, rect, visual, CONTROL_RADIUS as u8);
    paint_centered_text(
        ui,
        rect,
        spec.label,
        TypographyRole::Button,
        visual.text,
        visual.opacity,
    );
    paint_focus(ui, rect, &response, state, CONTROL_RADIUS);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::Button,
            spec.enabled,
            spec.selected,
            spec.accessible_label,
        )
    });
    response
}

/// Renders a primary action button.
pub fn primary_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    control_button(
        ui,
        ButtonSpec::new(label, ControlRole::Primary).enabled(enabled),
    )
}

/// Renders a secondary action button.
pub fn secondary_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    control_button(
        ui,
        ButtonSpec::new(label, ControlRole::Secondary).enabled(enabled),
    )
}

/// Renders a destructive action button.
pub fn danger_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    control_button(
        ui,
        ButtonSpec::new(label, ControlRole::Danger).enabled(enabled),
    )
}

/// Renders a light-surface icon action.
pub fn icon_button(ui: &mut Ui, spec: IconButtonSpec<'_>) -> Response {
    control_button(ui, spec.button)
}

/// Renders a dark player-toolbar icon action.
pub fn player_icon_button(ui: &mut Ui, spec: IconButtonSpec<'_>) -> Response {
    control_button(ui, spec.button)
}
