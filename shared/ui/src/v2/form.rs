//! Theme-aware form controls with native egui editing and popup behavior.

use super::{Icon, Palette, StatusTone, notice};
use egui::{Color32, ComboBox, Frame, Id, InnerResponse, Margin, Response, RichText, Stroke, Ui};

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

/// Choose one value inside a framed segmented trough without opening a popup.
/// Returns `true` when the selected option changed on this frame.
pub fn segmented<T: Copy + Eq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = Palette::for_ui(ui);
    let enabled = ui.is_enabled();
    let mut changed = false;
    Frame::new()
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 32.0;
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            ui.horizontal(|ui| {
                for &(option, label) in options {
                    let selected = *value == option;
                    let text_color = if !enabled {
                        p.muted.gamma_multiply(0.6)
                    } else if selected {
                        p.accent
                    } else {
                        p.muted
                    };
                    let rich = RichText::new(label)
                        .size(13.5)
                        .variation(b"wght", if selected { 500.0 } else { 400.0 })
                        .color(text_color);
                    let galley = egui::WidgetText::from(rich).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::FontSelection::Default,
                    );
                    let width = (galley.size().x + 22.0).max(58.0);
                    let sense = if enabled {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    };
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 32.0), sense);
                    let hovered = enabled && response.hovered();
                    if selected {
                        ui.painter().rect(
                            rect,
                            6,
                            p.surface,
                            Stroke::new(1.0, p.border),
                            egui::StrokeKind::Inside,
                        );
                    } else if hovered {
                        ui.painter().rect_filled(rect, 6, p.hover);
                    }
                    if response.has_focus() {
                        ui.painter().rect_stroke(
                            rect,
                            6,
                            Stroke::new(2.0, p.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let final_color = if hovered && !selected {
                        p.text
                    } else {
                        text_color
                    };
                    let text_pos = egui::pos2(
                        rect.center().x - galley.size().x * 0.5,
                        rect.center().y - galley.size().y * 0.5,
                    );
                    ui.painter().galley(text_pos, galley, final_color);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::RadioButton,
                            enabled,
                            selected,
                            label,
                        )
                    });
                    let response = if enabled {
                        response.on_hover_cursor(egui::CursorIcon::PointingHand)
                    } else {
                        response
                    };
                    if enabled && response.clicked() && !selected {
                        *value = option;
                        changed = true;
                    }
                }
            });
        });
    changed
}

fn paint_switch_track(
    ui: &Ui,
    track: egui::Rect,
    value: bool,
    enabled: bool,
    hovered: bool,
    has_focus: bool,
) {
    let p = Palette::for_ui(ui);
    let alpha = if enabled { 1.0 } else { 0.5 };
    let (track_fill, track_stroke) = if value {
        (p.accent.gamma_multiply(alpha), Stroke::NONE)
    } else {
        (
            p.canvas.gamma_multiply(alpha),
            Stroke::new(
                1.0,
                if hovered {
                    p.accent
                } else {
                    p.border.gamma_multiply(alpha)
                },
            ),
        )
    };
    ui.painter().rect(
        track,
        12.0,
        track_fill,
        track_stroke,
        egui::StrokeKind::Inside,
    );
    let knob_x = if value {
        track.right() - 12.0
    } else {
        track.left() + 12.0
    };
    let dark = ui.visuals().dark_mode;
    let knob_color = if value {
        if dark {
            Color32::from_rgb(0x14, 0x17, 0x18)
        } else {
            Color32::WHITE
        }
        .gamma_multiply(alpha)
    } else if hovered {
        p.text
    } else {
        p.muted.gamma_multiply(alpha)
    };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, track.center().y), 8.5, knob_color);
    if has_focus {
        ui.painter().rect_stroke(
            track.expand(2.5),
            14.0,
            Stroke::new(2.0, p.accent),
            egui::StrokeKind::Outside,
        );
    }
}

/// A keyboard-accessible switch; the caller keeps ownership of the setting.
pub fn switch(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let enabled = ui.is_enabled();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(48.0, 40.0), sense);
    if enabled && response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *value, label)
    });
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(44.0, 24.0));
    paint_switch_track(
        ui,
        track,
        *value,
        enabled,
        enabled && response.hovered(),
        response.has_focus(),
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A single keyboard-accessible switch paired with a clickable inline text label.
pub fn switch_with_label(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let p = Palette::for_ui(ui);
    let enabled = ui.is_enabled();
    let text_color = if enabled {
        p.text
    } else {
        p.muted.gamma_multiply(0.6)
    };
    let galley = egui::WidgetText::from(RichText::new(label).size(14.0).color(text_color))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontSelection::Default,
        );
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let size = egui::vec2(
        48.0 + 6.0 + galley.size().x + 4.0,
        40.0_f32.max(galley.size().y + 8.0),
    );
    let (rect, mut response) = ui.allocate_exact_size(size, sense);
    if enabled && response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *value, label)
    });
    let track = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        egui::vec2(44.0, 24.0),
    );
    paint_switch_track(
        ui,
        track,
        *value,
        enabled,
        enabled && response.hovered(),
        response.has_focus(),
    );
    let text_pos = egui::pos2(rect.left() + 54.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, text_color);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A full-width disclosure row. The stable id keeps editing and language changes independent.
pub fn disclosure(ui: &mut Ui, id: Id, label: &str, force_open: bool, body: impl FnOnce(&mut Ui)) {
    disclosure_with_badge(ui, id, label, None, force_open, body);
}

/// A full-width disclosure row with an optional compact status pill badge beside the title.
pub fn disclosure_with_badge(
    ui: &mut Ui,
    id: Id,
    label: &str,
    badge: Option<&str>,
    force_open: bool,
    body: impl FnOnce(&mut Ui),
) {
    let badge = badge.filter(|text| !text.trim().is_empty());
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if force_open {
        state.set_open(true);
    }
    let p = Palette::for_ui(ui);
    let enabled = ui.is_enabled();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 40.0), sense);
    let visual_rect = rect.shrink2(egui::vec2(0.0, 2.0));
    if enabled && response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(visual_rect, 8, p.surface.lerp_to_gamma(p.accent, 0.14));
    } else if enabled && response.hovered() {
        ui.painter().rect_filled(visual_rect, 8, p.hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            visual_rect.shrink(1.0),
            8,
            Stroke::new(2.0, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    let icon = if state.is_open() {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(visual_rect.left() + 16.0, visual_rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    let icon_tint = if !enabled {
        p.muted.gamma_multiply(0.6)
    } else if response.hovered() || response.is_pointer_button_down_on() {
        p.text
    } else {
        p.muted
    };
    icon.paint(ui, icon_rect, icon_tint);

    let title_color = if enabled {
        p.text
    } else {
        p.muted.gamma_multiply(0.75)
    };
    let title_galley = egui::WidgetText::from(
        RichText::new(label)
            .size(14.0)
            .variation(b"wght", 500.0)
            .color(title_color),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::FontSelection::Default,
    );
    let title_pos = egui::pos2(
        icon_rect.right() + 6.0,
        visual_rect.center().y - title_galley.size().y * 0.5,
    );
    let title_right = title_pos.x + title_galley.size().x;
    ui.painter().galley(title_pos, title_galley, title_color);

    if let Some(badge_text) = badge {
        let badge_galley = egui::WidgetText::from(
            RichText::new(badge_text)
                .size(12.0)
                .variation(b"wght", 500.0)
                .color(p.accent),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontSelection::Default,
        );
        let pill_w = badge_galley.size().x + 16.0;
        let pill_rect = egui::Rect::from_min_size(
            egui::pos2(title_right + 8.0, visual_rect.center().y - 11.0),
            egui::vec2(pill_w, 22.0),
        );
        if pill_rect.right() <= visual_rect.right() - 4.0 {
            ui.painter().rect_filled(pill_rect, 11.0, p.soft);
            ui.painter().galley(
                egui::pos2(
                    pill_rect.center().x - badge_galley.size().x * 0.5,
                    pill_rect.center().y - badge_galley.size().y * 0.5,
                ),
                badge_galley,
                p.accent,
            );
        }
    }

    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    if enabled && response.clicked() {
        state.toggle(ui);
    }
    state.store(ui.ctx());
    let accessible_label = badge.map(|badge_text| format!("{label} · {badge_text}"));
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            enabled,
            state.is_open(),
            accessible_label.as_deref().unwrap_or(label),
        )
    });
    if state.is_open() {
        ui.add_space(6.0);
        state.show_body_unindented(ui, |ui| {
            Frame::new()
                .fill(p.canvas)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(10)
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    body(ui);
                });
        });
    }
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

    #[test]
    fn segmented_switch_and_disclosure_batch2_controls_behave_in_light_and_dark() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            super::super::install(&ctx);
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let mut mode = 0u8;
            let mut enabled_flag = false;

            let run_pass =
                |events: Vec<egui::Event>,
                 mode: &mut u8,
                 enabled_flag: &mut bool|
                 -> (egui::FullOutput, egui::Rect, egui::Rect, egui::Rect, bool) {
                    let mut seg_rect = egui::Rect::NOTHING;
                    let mut sw_rect = egui::Rect::NOTHING;
                    let mut disc_rect = egui::Rect::NOTHING;
                    let mut body_drawn = false;
                    let out = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(520.0, 320.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default().show(root, |ui| {
                                seg_rect = ui
                                    .scope(|ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        segmented(
                                            ui,
                                            mode,
                                            &[(0, "Dark"), (1, "Light"), (2, "System")],
                                        );
                                    })
                                    .response
                                    .rect;
                                sw_rect = switch(ui, enabled_flag, "Developer mode").rect;
                                disc_rect = ui
                                    .scope(|ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        disclosure_with_badge(
                                            ui,
                                            Id::new("pin-test"),
                                            "Certificate verification",
                                            Some("Set"),
                                            false,
                                            |ui| {
                                                body_drawn = true;
                                                ui.label("SHA-256 body");
                                            },
                                        );
                                    })
                                    .response
                                    .rect;
                            });
                        },
                    );
                    (out, seg_rect, sw_rect, disc_rect, body_drawn)
                };

            // Warmup + measure pass
            let (mut out, seg_rect, sw_rect, disc_rect, body_drawn) =
                run_pass(vec![], &mut mode, &mut enabled_flag);
            out.textures_delta.clear();
            let (mut out, _, _, _, _) = run_pass(vec![], &mut mode, &mut enabled_flag);
            out.textures_delta.clear();

            assert!(!body_drawn, "disclosure must start collapsed");
            assert!(
                seg_rect.height() <= 40.0,
                "segmented height={} must fit inside 40pt settings_row",
                seg_rect.height()
            );
            assert!(
                sw_rect.height() >= 40.0,
                "switch hit height={}",
                sw_rect.height()
            );
            assert!(
                disc_rect.height() >= 40.0,
                "disclosure hit height={}",
                disc_rect.height()
            );
            assert!(
                out.shapes.iter().any(
                    |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "Set")
                ),
                "disclosure badge must be rendered"
            );
            let second_seg_center = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == "Light" => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("Light segment label");

            // Click Light segment, switch, and disclosure trigger to open
            let mut body_drawn_after_open = false;
            for click_pos in [second_seg_center, sw_rect.center(), disc_rect.center()] {
                for pressed in [true, false] {
                    let (mut out, _, _, _, drawn) = run_pass(
                        vec![
                            egui::Event::PointerMoved(click_pos),
                            egui::Event::PointerButton {
                                pos: click_pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        &mut mode,
                        &mut enabled_flag,
                    );
                    out.textures_delta.clear();
                    body_drawn_after_open = drawn;
                }
            }
            assert_eq!(mode, 1, "segmented must switch to Light");
            assert!(enabled_flag, "switch must toggle on");
            assert!(body_drawn_after_open, "disclosure must expand on click");

            // Click disclosure trigger again and verify collapse persists on next frame
            for pressed in [true, false] {
                let (mut out, _, _, _, _) = run_pass(
                    vec![
                        egui::Event::PointerMoved(disc_rect.center()),
                        egui::Event::PointerButton {
                            pos: disc_rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    &mut mode,
                    &mut enabled_flag,
                );
                out.textures_delta.clear();
            }
            let (mut out, _, _, _, body_drawn_after_close) =
                run_pass(vec![], &mut mode, &mut enabled_flag);
            out.textures_delta.clear();
            assert!(
                !body_drawn_after_close,
                "disclosure collapse must persist across frames"
            );
        }
    }

    #[test]
    fn disclosure_with_badge_preserves_badge_in_accesskit_when_collapsed() {
        for (title, badge, expected_with_badge) in [
            ("证书校验", "已设置", "证书校验 · 已设置"),
            (
                "Certificate verification",
                "Set",
                "Certificate verification · Set",
            ),
        ] {
            for has_badge in [false, true] {
                let ctx = egui::Context::default();
                super::super::install(&ctx);
                ctx.enable_accesskit();
                let mut body_drawn = false;
                let mut trigger_id = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(520.0, 240.0),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            disclosure_with_badge(
                                ui,
                                Id::new("relay-pin-a11y"),
                                title,
                                has_badge.then_some(badge),
                                false,
                                |ui| {
                                    body_drawn = true;
                                    ui.label("SHA-256");
                                },
                            );
                        });
                    },
                );
                assert!(!body_drawn, "disclosure should start collapsed");
                let update = output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .expect("AccessKit update must be emitted");
                let button_labels: Vec<&str> = update
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.role() == egui::accesskit::Role::Button)
                    .filter_map(|(_, node)| node.label())
                    .collect();
                let expected = if has_badge {
                    expected_with_badge
                } else {
                    title
                };
                assert!(
                    button_labels.contains(&expected),
                    "expected AccessKit button label {expected:?}, got {button_labels:?}"
                );
                if !has_badge {
                    assert!(
                        !button_labels.iter().any(|l| l.contains(badge)),
                        "badge must not appear when unset: {button_labels:?}"
                    );
                }
                output.textures_delta.clear();

                // Keyboard activation (Enter on focused disclosure) must still expand and keep badge label
                let mut trigger_rect = egui::Rect::NOTHING;
                let mut out2 = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(520.0, 240.0),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let scope = ui.scope(|ui| {
                                disclosure_with_badge(
                                    ui,
                                    Id::new("relay-pin-a11y"),
                                    title,
                                    has_badge.then_some(badge),
                                    false,
                                    |ui| {
                                        body_drawn = true;
                                        ui.label("SHA-256");
                                    },
                                );
                            });
                            trigger_rect = scope.response.rect;
                        });
                    },
                );
                out2.textures_delta.clear();
                // Tab focus into the disclosure trigger and press Space
                let mut out3 = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(520.0, 240.0),
                        )),
                        events: vec![egui::Event::Key {
                            key: egui::Key::Tab,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            disclosure_with_badge(
                                ui,
                                Id::new("relay-pin-a11y"),
                                title,
                                has_badge.then_some(badge),
                                false,
                                |ui| {
                                    body_drawn = true;
                                    ui.label("SHA-256");
                                },
                            );
                            trigger_id = ui.ctx().memory(|m| m.focused());
                        });
                    },
                );
                out3.textures_delta.clear();
                assert!(
                    trigger_id.is_some() && trigger_rect.height() >= 40.0,
                    "disclosure trigger must receive keyboard focus"
                );
                let mut out4 = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(520.0, 240.0),
                        )),
                        events: vec![egui::Event::Key {
                            key: egui::Key::Space,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            disclosure_with_badge(
                                ui,
                                Id::new("relay-pin-a11y"),
                                title,
                                has_badge.then_some(badge),
                                false,
                                |ui| {
                                    body_drawn = true;
                                    ui.label("SHA-256");
                                },
                            );
                        });
                    },
                );
                out4.textures_delta.clear();
                assert!(body_drawn, "Space on focused disclosure must expand it");
            }
        }
    }

    #[test]
    fn switch_with_label_toggles_once_from_text_or_track_with_single_focus_stop() {
        for label in ["暂停自动滚动", "Pause auto-scroll"] {
            let ctx = egui::Context::default();
            super::super::install(&ctx);
            ctx.enable_accesskit();
            let mut value = false;

            let render = |events: Vec<egui::Event>, value: &mut bool| {
                let mut control_rect = egui::Rect::NOTHING;
                let mut changed = false;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(480.0, 120.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let response = switch_with_label(ui, value, label);
                            control_rect = response.rect;
                            changed = response.changed();
                        });
                    },
                );
                output.textures_delta.clear();
                (output, control_rect, changed)
            };

            // Warmup and locate painted text center and track center
            let (out, control_rect, _) = render(vec![], &mut value);
            let update = out
                .platform_output
                .accesskit_update
                .as_ref()
                .expect("AccessKit update");
            let checkboxes: Vec<&str> = update
                .nodes
                .iter()
                .filter(|(_, node)| node.role() == egui::accesskit::Role::CheckBox)
                .filter_map(|(_, node)| node.label())
                .collect();
            assert_eq!(
                checkboxes,
                vec![label],
                "switch_with_label must expose exactly one CheckBox control"
            );

            let text_center = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("inline switch label must be painted");
            let track_center = egui::pos2(control_rect.left() + 24.0, control_rect.center().y);

            // Click painted label text: false -> true (single toggle, no double trigger)
            let mut changed_on_release = false;
            for pressed in [true, false] {
                let (_, _, changed) = render(
                    vec![
                        egui::Event::PointerMoved(text_center),
                        egui::Event::PointerButton {
                            pos: text_center,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    &mut value,
                );
                changed_on_release = changed;
            }
            assert!(value, "clicking label text must toggle false -> true");
            assert!(changed_on_release);

            // Click painted label text again: true -> false
            for pressed in [true, false] {
                let _ = render(
                    vec![
                        egui::Event::PointerMoved(text_center),
                        egui::Event::PointerButton {
                            pos: text_center,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    &mut value,
                );
            }
            assert!(!value, "second label click must toggle true -> false");

            // Click switch track: false -> true
            for pressed in [true, false] {
                let _ = render(
                    vec![
                        egui::Event::PointerMoved(track_center),
                        egui::Event::PointerButton {
                            pos: track_center,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    &mut value,
                );
            }
            assert!(value, "clicking track must toggle false -> true");

            // Keyboard Tab + Space must toggle once with a single focusable stop
            let _ = render(
                vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                &mut value,
            );
            let _ = render(
                vec![egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                &mut value,
            );
            assert!(
                !value,
                "Space on focused switch_with_label must toggle once"
            );
        }
    }
}
