use egui::{Align, Layout, Rect, Ui, UiBuilder, pos2};

use crate::{Size, Spacing};

/// A semantic maximum width for a centered page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageWidth {
    /// Dense workspaces and wide lists.
    Wide,
    /// Focused tasks and media surfaces.
    Medium,
    /// Settings and form-heavy pages.
    Narrow,
}

impl PageWidth {
    /// Returns the maximum content width in logical points.
    pub const fn max_width(self) -> f32 {
        match self {
            Self::Wide => Size::PAGE_WIDE_MAX,
            Self::Medium => Size::PAGE_MEDIUM_MAX,
            Self::Narrow => Size::PAGE_NARROW_MAX,
        }
    }
}

/// Returns the horizontal page inset for the available viewport width.
pub const fn page_horizontal_inset(available_width: f32) -> f32 {
    if available_width < Size::SPLIT_BREAKPOINT {
        Size::PAGE_HORIZONTAL_NARROW
    } else {
        Size::PAGE_HORIZONTAL_WIDE
    }
}

/// Resolves a centered page rectangle with shared outer insets.
pub fn page_content_rect(available: Rect, width: PageWidth) -> Rect {
    let narrow = available.width() < Size::SPLIT_BREAKPOINT;
    let horizontal = page_horizontal_inset(available.width());
    let top = if narrow {
        Size::PAGE_TOP_NARROW
    } else {
        Size::PAGE_TOP_WIDE
    };
    centered_rect(
        available,
        width.max_width(),
        horizontal,
        top,
        Size::PAGE_BOTTOM,
    )
}

/// Resolves a centered app-bar rectangle aligned to wide page content.
pub fn app_bar_content_rect(available: Rect) -> Rect {
    centered_rect(
        available,
        PageWidth::Wide.max_width(),
        page_horizontal_inset(available.width()),
        0.0,
        0.0,
    )
}

/// Renders a page inside a centered role-based content rectangle.
pub fn page_shell<R>(ui: &mut Ui, width: PageWidth, content: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = page_content_rect(ui.available_rect_before_wrap(), width);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_width(rect.width());
            content(ui)
        },
    )
    .inner
}

/// Reserves a fixed bottom row for page actions before laying out scrollable content.
pub fn page_actions<R>(ui: &mut Ui, actions: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Panel::bottom(ui.id().with("page-actions"))
        .exact_size(Size::CONTROL + Spacing::LG)
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin {
            top: Spacing::LG as i8,
            ..Default::default()
        }))
        .show(ui, actions)
        .inner
}

fn centered_rect(
    available: Rect,
    max_width: f32,
    horizontal_inset: f32,
    top_inset: f32,
    bottom_inset: f32,
) -> Rect {
    let usable_width = (available.width() - horizontal_inset * 2.0).max(1.0);
    let width = usable_width.min(max_width);
    Rect::from_min_max(
        pos2(
            available.center().x - width / 2.0,
            available.top() + top_inset,
        ),
        pos2(
            available.center().x + width / 2.0,
            (available.bottom() - bottom_inset).max(available.top() + top_inset + 1.0),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_widths_share_centering_but_keep_role_limits() {
        let available = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1440.0, 900.0));
        for (role, expected_width) in [
            (PageWidth::Wide, 1120.0),
            (PageWidth::Medium, 880.0),
            (PageWidth::Narrow, 720.0),
        ] {
            let rect = page_content_rect(available, role);
            assert_eq!(rect.width(), expected_width);
            assert_eq!(rect.center().x, available.center().x);
        }
    }

    #[test]
    fn minimum_viewport_keeps_symmetric_narrow_insets() {
        let available = Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(Size::MIN_VIEWPORT[0], Size::MIN_VIEWPORT[1]),
        );
        let rect = page_content_rect(available, PageWidth::Narrow);
        assert_eq!(rect.left(), Size::PAGE_HORIZONTAL_NARROW);
        assert_eq!(
            available.right() - rect.right(),
            Size::PAGE_HORIZONTAL_NARROW
        );
        assert_eq!(rect.top(), Size::PAGE_TOP_NARROW);
        assert_eq!(available.bottom() - rect.bottom(), Size::PAGE_BOTTOM);
    }

    #[test]
    fn page_actions_stay_visible_and_clickable_with_scrolling_content() {
        for viewport in [
            egui::vec2(680.0, 520.0),
            egui::vec2(680.0, 640.0),
            egui::vec2(1440.0, 900.0),
        ] {
            for label in ["开始共享", "Start sharing", "停止共享", "Stop sharing"] {
                let context = egui::Context::default();
                crate::Theme.apply(&context);
                let mut previous_button = None;
                for count in [1, 50] {
                    let mut button = None;
                    let mut clip = Rect::NOTHING;
                    let mut scrolling = Rect::NOTHING;
                    let output = context.run_ui(egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                        ..Default::default()
                    }, |ui| {
                        egui::Panel::top("navigation")
                            .exact_size(Size::APP_BAR_COMPACT)
                            .show(ui, |_| {});
                        egui::CentralPanel::default().show(ui, |ui| {
                            page_shell(ui, PageWidth::Medium, |ui| {
                                clip = ui.clip_rect();
                                button = Some(page_actions(ui, |ui| {
                                    crate::primary_button(ui, label, true)
                                }));
                                scrolling = egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        crate::page_header(ui, "Screen share", Some("Choose a source"));
                                        for index in 0..count {
                                            crate::device_row(ui,
                                                crate::DeviceRowSpec::new(egui::Id::new(index),
                                                    "A long window title with enough text to exercise the source list layout"), |_| {});
                                        }
                                    }).inner_rect;
                            });
                        });
                    });
                    output.drop_without_applying_deltas();
                    let response = button.unwrap();
                    assert!(clip.contains_rect(response.rect), "{viewport:?}: {label}");
                    assert!(scrolling.bottom() <= response.rect.top());
                    if let Some(previous) = previous_button {
                        assert_eq!(response.rect, previous, "The list must not move the button");
                    }
                    previous_button = Some(response.rect);
                }
                let center = previous_button.unwrap().center();
                let mut clicked = false;
                for pressed in [true, false] {
                    let output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                            events: vec![
                                egui::Event::PointerMoved(center),
                                egui::Event::PointerButton {
                                    pos: center,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                            ..Default::default()
                        },
                        |ui| {
                            egui::Panel::top("navigation")
                                .exact_size(Size::APP_BAR_COMPACT)
                                .show(ui, |_| {});
                            egui::CentralPanel::default().show(ui, |ui| {
                                page_shell(ui, PageWidth::Medium, |ui| {
                                    clicked |= page_actions(ui, |ui| {
                                        crate::primary_button(ui, label, true)
                                    })
                                    .clicked();
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            for _ in 0..50 {
                                                ui.label("Scrollable source information");
                                            }
                                        });
                                });
                            });
                        },
                    );
                    output.drop_without_applying_deltas();
                }
                assert!(clicked, "The fixed action must receive pointer input");
            }
        }
    }
}
