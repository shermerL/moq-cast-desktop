use egui::{Color32, Stroke};

use crate::{COLORS, Color, Size};

/// The visual interaction state of a component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interaction {
    /// Resting state.
    Rest,
    /// Pointer hover state.
    Hovered,
    /// Pointer or keyboard press state.
    Pressed,
    /// Active or selected state.
    Selected,
    /// Keyboard focus state.
    Focused,
    /// Disabled state.
    Disabled,
}

/// The visual role of an interactive control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlRole {
    /// Top-level navigation.
    Nav,
    /// Primary action.
    Primary,
    /// Secondary action.
    Secondary,
    /// Destructive action.
    Danger,
    /// Light-surface icon action.
    Icon,
    /// Dark player-toolbar icon action.
    PlayerIcon,
}

/// Fully resolved colors, opacity, and strokes for one interaction state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedVisual {
    /// Component background.
    pub fill: Color,
    /// Component foreground text or icon.
    pub text: Color,
    /// Component border color.
    pub border: Color,
    /// Component border width.
    pub border_width: f32,
    /// Content opacity applied after state colors resolve.
    pub opacity: f32,
    /// Optional keyboard focus ring.
    pub focus: Option<Stroke>,
    /// Distance between the focus ring and component edge.
    pub focus_outset: f32,
    /// Optional selected navigation underline width.
    pub underline: f32,
}

/// Resolves a control role and interaction state into stable visuals.
pub fn resolve_control_visual(role: ControlRole, state: Interaction) -> ResolvedVisual {
    let (fill, text, border) = match role {
        ControlRole::Nav => nav_colors(state),
        ControlRole::Primary => primary_colors(state),
        ControlRole::Secondary => secondary_colors(state),
        ControlRole::Danger => danger_colors(state),
        ControlRole::Icon => icon_colors(state),
        ControlRole::PlayerIcon => player_icon_colors(state),
    };

    ResolvedVisual {
        fill,
        text,
        border,
        border_width: Size::BORDER,
        opacity: if state == Interaction::Disabled {
            Size::DISABLED_ALPHA
        } else {
            1.0
        },
        focus: (state == Interaction::Focused)
            .then_some(Stroke::new(Size::FOCUS, Color32::from(COLORS.focus))),
        focus_outset: Size::FOCUS_OUTSET,
        underline: if role == ControlRole::Nav && state == Interaction::Selected {
            Size::NAV_UNDERLINE
        } else {
            0.0
        },
    }
}

fn nav_colors(state: Interaction) -> (Color, Color, Color) {
    match state {
        Interaction::Hovered => (COLORS.surface_muted, COLORS.text, COLORS.surface_muted),
        Interaction::Pressed => (
            COLORS.secondary_pressed,
            COLORS.text,
            COLORS.secondary_pressed,
        ),
        Interaction::Selected => (COLORS.brand_soft, COLORS.brand, COLORS.brand_soft),
        Interaction::Rest | Interaction::Focused | Interaction::Disabled => {
            (COLORS.chrome, COLORS.text, COLORS.chrome)
        }
    }
}

fn primary_colors(state: Interaction) -> (Color, Color, Color) {
    match state {
        Interaction::Hovered => (COLORS.brand_hover, COLORS.surface, COLORS.brand_hover),
        Interaction::Pressed | Interaction::Selected => {
            (COLORS.brand_pressed, COLORS.surface, COLORS.brand_pressed)
        }
        Interaction::Rest | Interaction::Focused | Interaction::Disabled => {
            (COLORS.brand, COLORS.surface, COLORS.brand)
        }
    }
}

fn secondary_colors(state: Interaction) -> (Color, Color, Color) {
    match state {
        Interaction::Hovered => (COLORS.surface_muted, COLORS.text, COLORS.border_strong),
        Interaction::Pressed => (COLORS.secondary_pressed, COLORS.text, COLORS.border_strong),
        Interaction::Selected => (COLORS.brand_soft, COLORS.brand, COLORS.brand),
        Interaction::Rest | Interaction::Focused | Interaction::Disabled => {
            (COLORS.surface, COLORS.text, COLORS.border)
        }
    }
}

fn danger_colors(state: Interaction) -> (Color, Color, Color) {
    match state {
        Interaction::Hovered => (COLORS.danger_hover, COLORS.danger, COLORS.danger),
        Interaction::Pressed | Interaction::Selected => {
            (COLORS.danger_pressed, COLORS.danger, COLORS.danger)
        }
        Interaction::Rest | Interaction::Focused | Interaction::Disabled => {
            (COLORS.danger_soft, COLORS.danger, COLORS.danger)
        }
    }
}

fn icon_colors(state: Interaction) -> (Color, Color, Color) {
    match state {
        Interaction::Hovered => (COLORS.surface_muted, COLORS.text, COLORS.border_strong),
        Interaction::Pressed => (COLORS.secondary_pressed, COLORS.text, COLORS.border_strong),
        Interaction::Selected => (COLORS.brand_soft, COLORS.brand, COLORS.brand),
        Interaction::Rest | Interaction::Focused | Interaction::Disabled => {
            (COLORS.surface, COLORS.text, COLORS.border)
        }
    }
}

fn player_icon_colors(state: Interaction) -> (Color, Color, Color) {
    match state {
        Interaction::Hovered => (COLORS.text, COLORS.player_text, COLORS.player_muted),
        Interaction::Pressed => (COLORS.player, COLORS.player_text, COLORS.player_muted),
        Interaction::Selected => (COLORS.brand, COLORS.surface, COLORS.brand),
        Interaction::Rest | Interaction::Focused => {
            (COLORS.player_bar, COLORS.player_text, COLORS.player_bar)
        }
        Interaction::Disabled => (COLORS.player_bar, COLORS.player_muted, COLORS.player_bar),
    }
}
