//! Theme-aware form controls with native egui editing and popup behavior.

use super::{Palette, StatusTone, notice};
use egui::{ComboBox, Id, InnerResponse, Margin, Response, RichText, Stroke, Ui};

fn style(ui: &mut Ui) {
    let p = Palette::for_ui(ui);
    let visuals = ui.visuals_mut();
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = 7.into();
        widget.expansion = 0.0;
        widget.fg_stroke = Stroke::new(1.0, p.text);
    }
    visuals.widgets.inactive.weak_bg_fill = p.surface;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.open.weak_bg_fill = p.surface;
    visuals.widgets.open.bg_stroke = Stroke::new(2.0, p.accent);
    visuals.widgets.active.weak_bg_fill = p.soft;
    visuals.widgets.active.bg_stroke = Stroke::new(2.0, p.accent);
    visuals.selection.stroke = Stroke::new(2.0, p.accent);
}

/// A labeled single-line field; the caller owns its value and validation.
pub struct TextField<'a> {
    /// Stable identity across changes of label or language.
    pub id: Id,
    /// Visible and accessible field label.
    pub label: &'a str,
    /// Hint shown while the value is empty.
    pub hint: &'a str,
    /// Whether editing is currently allowed.
    pub enabled: bool,
    /// Validation message displayed beside an error outline.
    pub error: Option<&'a str>,
}

impl TextField<'_> {
    /// Shows a 44-point field, horizontally scrolling text inside its available width.
    pub fn show(self, ui: &mut Ui, value: &mut String) -> Response {
        let p = Palette::for_ui(ui);
        let label = ui.label(
            RichText::new(self.label)
                .size(13.0)
                .variation(b"wght", 500.0),
        );
        let response = ui
            .scope(|ui| {
                style(ui);
                ui.spacing_mut().item_spacing.y = 8.0;
                let font = egui::FontId::proportional(14.0);
                let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
                ui.spacing_mut().extra_text_line_spacing = (20.0 - row_height).max(0.0);
                ui.add_enabled(
                    self.enabled,
                    egui::TextEdit::singleline(value)
                        .id(self.id)
                        .interactive(self.enabled && ui.is_enabled())
                        .hint_text(RichText::new(self.hint).color(p.muted))
                        .font(font)
                        .margin(Margin::symmetric(12, 12))
                        .desired_width(ui.available_width())
                        .background_color(p.surface)
                        .clip_text(true),
                )
            })
            .inner
            .labelled_by(label.id);
        if self.error.is_some() {
            ui.painter().rect_stroke(
                response.rect,
                7,
                Stroke::new(1.5, p.danger),
                egui::StrokeKind::Inside,
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect.expand(2.0),
                9,
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(error) = self.error {
            notice(ui, StatusTone::Error, error);
        }
        response
    }
}

/// Shows a themed select with the native popup, keyboard navigation and selection behavior.
pub fn select<R>(
    ui: &mut Ui,
    combo: ComboBox,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let result = ui
        .scope(|ui| {
            style(ui);
            ui.spacing_mut().button_padding = egui::vec2(12.0, 10.0);
            ui.spacing_mut().interact_size.y = 40.0;
            combo.width(164.0).show_ui(ui, contents)
        })
        .inner;
    if result.response.has_focus() {
        ui.painter().rect_stroke(
            result.response.rect.expand(2.0),
            9,
            Stroke::new(2.0, Palette::for_ui(ui).accent),
            egui::StrokeKind::Outside,
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_keeps_field_bounded_and_disabled_editing_is_refused() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            super::super::install(&ctx);
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let id = Id::new("address");
            let mut value = "https://example.com/".repeat(80);
            let original = value.clone();
            for enabled in [true, false] {
                ctx.memory_mut(|m| m.request_focus(id));
                let mut bounds = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(360.0, 220.0),
                        )),
                        events: if enabled {
                            vec![]
                        } else {
                            vec![egui::Event::Paste("must not edit".into())]
                        },
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let available = ui.available_rect_before_wrap();
                            let response = TextField {
                                id,
                                label: "Server URL",
                                hint: "",
                                enabled,
                                error: Some("Invalid URL"),
                            }
                            .show(ui, &mut value);
                            bounds = Some((available, response.rect));
                        });
                    },
                );
                let (available, field) = bounds.unwrap();
                assert!(field.height() >= 44.0, "{field:?}");
                assert!(
                    available.contains_rect(field),
                    "text must scroll inside the field"
                );
                assert_eq!(value, original);
                assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.job.text == "Invalid URL")));
                output.textures_delta.clear();
            }
        }
    }
}
