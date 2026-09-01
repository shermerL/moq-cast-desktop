use egui::{Align, CornerRadius, Layout, Response, Sense, Ui, UiBuilder, vec2};

use crate::{COLORS, Radius, Size, Spacing};

/// Renders a stable 16:9 dark player stage up to the shared watch width.
pub fn player_stage<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width().min(Size::PAGE_MEDIUM_MAX);
    let height = width * Size::PLAYER_ASPECT[1] / Size::PLAYER_ASPECT[0];
    let rect = ui.allocate_space(vec2(width, height)).1;
    ui.painter().rect_filled(
        rect,
        CornerRadius {
            nw: Radius::LG as u8,
            ne: Radius::LG as u8,
            sw: 0,
            se: 0,
        },
        COLORS.player,
    );
    ui.scope_builder(UiBuilder::new().max_rect(rect), content)
        .inner
}

/// Renders a fifty-two-point dark toolbar with forty-point controls.
pub fn player_toolbar<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), Size::PLAYER_TOOLBAR),
        Sense::hover(),
    );
    ui.painter().rect_filled(
        rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: Radius::LG as u8,
            se: Radius::LG as u8,
        },
        COLORS.player_bar,
    );
    let inner_rect = rect.shrink2(vec2(Spacing::SM, 6.0));
    let inner = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(inner_rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = Size::PLAYER_TOOLBAR_ITEM_SPACING;
                ui.set_min_height(Size::CONTROL);
                ui.set_max_height(Size::CONTROL);
                content(ui)
            },
        )
        .inner;
    (response, inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_toolbar_has_exact_outer_height() {
        egui::__run_test_ui(|ui| {
            let (response, ()) = player_toolbar(ui, |_| ());
            assert_eq!(response.rect.height(), Size::PLAYER_TOOLBAR);
        });
    }

    #[test]
    fn player_stage_exposes_the_allocated_sixteen_by_nine_rect() {
        egui::__run_test_ui(|ui| {
            let rect = player_stage(ui, |ui| ui.max_rect());
            assert_eq!(rect.width() / rect.height(), 16.0 / 9.0);
            assert!(rect.width() <= Size::PAGE_MEDIUM_MAX);
        });
    }

    #[test]
    fn player_toolbar_fills_the_available_width() {
        egui::__run_test_ui(|ui| {
            ui.set_width(640.0);
            let (response, ()) = player_toolbar(ui, |_| ());
            assert_eq!(response.rect.width(), 640.0);
        });
    }
}
