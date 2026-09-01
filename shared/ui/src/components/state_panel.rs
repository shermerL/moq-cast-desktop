use egui::{CornerRadius, Frame, Margin, Stroke, Ui};

use crate::{COLORS, Color, Radius, Size, Spacing, TypographyRole, typography};

/// Semantic status-badge color.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BadgeTone {
    /// Neutral or inactive state.
    Neutral,
    /// Informational state.
    Info,
    /// Warning state.
    Warning,
    /// Error or destructive state.
    Danger,
}

/// Semantic empty or lifecycle-state panel kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatePanelKind {
    /// Empty data state.
    Empty,
    /// Work is in progress.
    Pending,
    /// A recoverable failure occurred.
    Failed,
}

/// Display-only configuration for a state panel.
#[derive(Clone, Copy, Debug)]
pub struct StatePanelSpec<'a> {
    kind: StatePanelKind,
    title: &'a str,
    description: &'a str,
}

impl<'a> StatePanelSpec<'a> {
    /// Creates a state panel with semantic kind and supporting copy.
    pub fn new(kind: StatePanelKind, title: &'a str, description: &'a str) -> Self {
        Self {
            kind,
            title,
            description,
        }
    }
}

/// Renders a compact semantic status badge.
pub fn status_badge(ui: &mut Ui, label: &str, tone: BadgeTone) -> egui::Response {
    let (text, fill) = badge_colors(tone);
    Frame::new()
        .fill(fill.into())
        .corner_radius(CornerRadius::same(Radius::SM as u8))
        .inner_margin(Margin::symmetric(Spacing::SM as i8, Spacing::XS as i8))
        .show(ui, |ui| {
            ui.label(typography(label, TypographyRole::Meta, text.into()))
        })
        .response
}

/// Renders a centered state panel with a caller-owned action area.
pub fn state_panel<R>(
    ui: &mut Ui,
    spec: StatePanelSpec<'_>,
    action: impl FnOnce(&mut Ui) -> R,
) -> R {
    let accent = match spec.kind {
        StatePanelKind::Empty => COLORS.muted,
        StatePanelKind::Pending => COLORS.info,
        StatePanelKind::Failed => COLORS.danger,
    };
    Frame::new()
        .fill(COLORS.surface.into())
        .stroke(Stroke::new(Size::BORDER, COLORS.border))
        .corner_radius(CornerRadius::same(Radius::LG as u8))
        .inner_margin(Margin::same(Spacing::XL as i8))
        .show(ui, |ui| {
            ui.set_min_height(Size::STATE_PANEL_MIN - Spacing::XL * 2.0);
            ui.vertical_centered(|ui| {
                ui.label(typography(
                    spec.title,
                    TypographyRole::Section,
                    accent.into(),
                ));
                ui.add_space(Spacing::SM);
                ui.label(typography(
                    spec.description,
                    TypographyRole::Body,
                    COLORS.muted.into(),
                ));
                ui.add_space(Spacing::LG);
                action(ui)
            })
            .inner
        })
        .inner
}

fn badge_colors(tone: BadgeTone) -> (Color, Color) {
    match tone {
        BadgeTone::Neutral => (COLORS.muted, COLORS.surface_muted),
        BadgeTone::Info => (COLORS.info, COLORS.info_soft),
        BadgeTone::Warning => (COLORS.warning, COLORS.warning_soft),
        BadgeTone::Danger => (COLORS.danger, COLORS.danger_soft),
    }
}
