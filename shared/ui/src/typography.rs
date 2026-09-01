use egui::{Color32, FontFamily, RichText};

/// A semantic typography role shared by desktop presentation layers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypographyRole {
    /// Page title.
    PageTitle,
    /// Section heading.
    Section,
    /// Primary row label.
    Row,
    /// Button label.
    Button,
    /// Body copy.
    Body,
    /// Supporting help copy.
    Help,
    /// Compact metadata.
    Meta,
    /// Technical metadata rendered with egui's open-source monospace font.
    Mono,
}

/// Resolved metrics and variable-font weight for a typography role.
#[derive(Clone, Debug, PartialEq)]
pub struct TypographySpec {
    /// Font size in logical points.
    pub size: f32,
    /// Explicit line height in logical points.
    pub line_height: f32,
    /// OpenType `wght` variation coordinate.
    pub weight: f32,
    /// Font family used for the role.
    pub family: FontFamily,
}

/// Resolves the frozen metrics for a semantic typography role.
pub fn typography_spec(role: TypographyRole) -> TypographySpec {
    match role {
        TypographyRole::PageTitle => spec(30.0, 38.0, 600.0, FontFamily::Proportional),
        TypographyRole::Section => spec(18.0, 26.0, 600.0, FontFamily::Proportional),
        TypographyRole::Row => spec(15.0, 22.0, 500.0, FontFamily::Proportional),
        TypographyRole::Button => spec(14.0, 20.0, 600.0, FontFamily::Proportional),
        TypographyRole::Body => spec(14.0, 20.0, 400.0, FontFamily::Proportional),
        TypographyRole::Help => spec(13.0, 18.0, 400.0, FontFamily::Proportional),
        TypographyRole::Meta => spec(12.0, 16.0, 500.0, FontFamily::Proportional),
        TypographyRole::Mono => spec(12.0, 18.0, 400.0, FontFamily::Monospace),
    }
}

/// Styles display text with the exact metrics and variable weight for a role.
pub fn typography(text: impl Into<String>, role: TypographyRole, color: Color32) -> RichText {
    let spec = typography_spec(role);
    RichText::new(text)
        .size(spec.size)
        .line_height(Some(spec.line_height))
        .family(spec.family)
        .variation(b"wght", spec.weight)
        .color(color)
}

fn spec(size: f32, line_height: f32, weight: f32, family: FontFamily) -> TypographySpec {
    TypographySpec {
        size,
        line_height,
        weight,
        family,
    }
}
