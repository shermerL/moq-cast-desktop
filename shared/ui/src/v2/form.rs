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
                // TextEdit uses the stroke width for its focus frame and the color for selected glyphs.
                ui.visuals_mut().selection.stroke.width = 0.0;
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
        if response.has_focus() || self.error.is_some() {
            let color = if self.error.is_some() {
                p.danger
            } else {
                p.accent
            };
            ui.painter().rect_stroke(
                response.rect,
                7,
                Stroke::new(if response.has_focus() { 2.0 } else { 1.5 }, color),
                egui::StrokeKind::Inside,
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
    ui.scope(|ui| {
        style(ui);
        ui.spacing_mut().button_padding = egui::vec2(12.0, 10.0);
        ui.spacing_mut().interact_size.y = 40.0;
        // ComboBox uses widgets.active for keyboard focus and widgets.open for its popup.
        combo.width(164.0).show_ui(ui, contents)
    })
    .inner
}

/// Choose one value without opening a separate popup.
pub fn segmented<T: Copy + Eq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) {
    let p = Palette::for_ui(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &(option, label) in options {
            let selected = *value == option;
            let button = egui::Button::selectable(selected, label)
                .min_size(egui::vec2(76.0, 40.0))
                .fill(if selected { p.soft } else { p.surface })
                .stroke(Stroke::new(1.0, if selected { p.accent } else { p.border }));
            if ui.add(button).clicked() {
                *value = option;
            }
        }
    });
}

/// A keyboard-accessible switch; the caller keeps ownership of the setting.
pub fn switch(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let p = Palette::for_ui(ui);
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(48.0, 40.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            response.enabled(),
            *value,
            label,
        )
    });
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(44.0, 24.0));
    let fill = if *value { p.accent } else { p.muted };
    ui.painter().rect_filled(
        track,
        12.0,
        if response.enabled() { fill } else { p.border },
    );
    let x = if *value {
        track.right() - 12.0
    } else {
        track.left() + 12.0
    };
    ui.painter()
        .circle_filled(egui::pos2(x, track.center().y), 9.0, p.surface);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            7,
            Stroke::new(2.0, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// A full-width disclosure row. The stable id keeps editing and language changes independent.
pub fn disclosure(ui: &mut Ui, id: Id, label: &str, force_open: bool, body: impl FnOnce(&mut Ui)) {
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if force_open {
        state.set_open(true);
    }
    let p = Palette::for_ui(ui);
    let response = ui
        .scope(|ui| {
            style(ui);
            ui.add(
                egui::Button::new(label)
                    .min_size(egui::vec2(ui.available_width(), 40.0))
                    .fill(p.hover),
            )
        })
        .inner;
    let center = egui::pos2(response.rect.right() - 18.0, response.rect.center().y);
    let points = if state.is_open() {
        [(-4.0, -2.0), (0.0, 2.0), (4.0, -2.0)]
    } else {
        [(-2.0, -4.0), (2.0, 0.0), (-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::line(
        points
            .into_iter()
            .map(|(x, y)| center + egui::vec2(x, y))
            .collect(),
        Stroke::new(1.5, p.muted),
    ));
    if response.clicked() {
        state.toggle(ui);
    }
    state.show_body_unindented(ui, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_controls_keep_one_ring_and_selected_field_text_is_readable() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            super::super::install(&ctx);
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let id = Id::new("prefix");
            let mut text = String::from("moqt://relay.test/room");
            let mut combo_id = None;
            for (index, error) in [None, None, Some("Invalid prefix"), None]
                .into_iter()
                .enumerate()
            {
                if index > 0 {
                    ctx.memory_mut(|m| {
                        m.request_focus(if index == 3 { combo_id.unwrap() } else { id })
                    });
                }
                let mut input = egui::RawInput::default();
                let selecting_text = matches!(index, 1 | 2);
                if selecting_text {
                    input.events.push(egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
                    });
                }
                let mut expected = None;
                let mut output = ctx.run_ui(input, |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        let field = TextField {
                            id,
                            label: "Prefix",
                            hint: "",
                            enabled: true,
                            error,
                        }
                        .show(ui, &mut text);
                        let combo = select(
                            ui,
                            ComboBox::from_id_salt("decode").selected_text("Auto"),
                            |_| {},
                        );
                        combo_id = Some(combo.response.id);
                        let response = if index == 3 { &combo.response } else { &field };
                        if index != 0 {
                            assert!(response.has_focus());
                            let palette = Palette::for_ui(ui);
                            expected = Some((
                                response.rect,
                                if error.is_some() {
                                    palette.danger
                                } else {
                                    palette.accent
                                },
                            ));
                        }
                    });
                });
                output.textures_delta.clear();
                if let Some((rect, color)) = expected {
                    let strokes: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| {
                            let egui::Shape::Rect(r) = &shape.shape else {
                                return None;
                            };
                            // Include inset and outset outlines as well as the exact control edge.
                            (!r.stroke.is_empty()
                                && rect.expand(3.0).contains_rect(r.rect)
                                && r.rect.contains_rect(rect.shrink(3.0)))
                            .then_some((r.rect, r.stroke))
                        })
                        .collect();
                    assert_eq!(
                        strokes,
                        [(rect, Stroke::new(2.0, color))],
                        "theme={dark}, pass={index}"
                    );
                }
                if selecting_text {
                    let selection = egui::TextEdit::load_state(&ctx, id)
                        .unwrap()
                        .cursor
                        .char_range()
                        .unwrap();
                    let range = selection.as_sorted_char_range();
                    assert_eq!(range.start.0..range.end.0, 0..text.chars().count());
                    let painted = output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.job.text == text => Some(t),
                            _ => None,
                        })
                        .unwrap();
                    let row = &painted.galley.rows[0].visuals;
                    let glyphs = &row.mesh.vertices[row.glyph_vertex_range.clone()];
                    assert!(!glyphs.is_empty());
                    let foreground = Palette::new(dark).accent;
                    assert!(
                        glyphs
                            .iter()
                            .all(|v| painted.override_text_color.unwrap_or(v.color) == foreground),
                        "selected glyphs must use the theme foreground: dark={dark}, error={error:?}"
                    );
                }
            }
        }
    }

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
