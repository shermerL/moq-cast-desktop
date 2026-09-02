use egui::{Align, CornerRadius, Layout, Rect, Response, Sense, Ui, UiBuilder, pos2, vec2};

use crate::{COLORS, Radius, Size, Spacing};

/// Stable stage and bottom-toolbar rectangles for one player surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerRects {
    /// The full video stage rectangle.
    pub stage: Rect,
    /// The toolbar rectangle attached below the stage or overlaid in fullscreen.
    pub toolbar: Rect,
}

/// Outputs from one coupled windowed player surface.
pub struct PlayerSurfaceResponse<S, T> {
    /// The allocated stage and toolbar rectangles.
    pub rects: PlayerRects,
    /// The caller-owned stage content output.
    pub stage: S,
    /// The caller-owned toolbar content output.
    pub toolbar: T,
    /// The response for the complete player surface.
    pub response: Response,
}

/// Resolves a player stage and toolbar from one stable available rectangle.
pub fn player_rects(available: Rect, fullscreen: bool) -> PlayerRects {
    let available = Rect::from_min_max(
        available.min,
        pos2(
            available.right().max(available.left() + 1.0),
            available.bottom().max(available.top() + 1.0),
        ),
    );
    let stage = if fullscreen {
        available
    } else {
        let stage_height = (available.height() - Size::PLAYER_TOOLBAR).max(1.0);
        let width = available
            .width()
            .min(Size::PAGE_MEDIUM_MAX)
            .min(stage_height * Size::PLAYER_ASPECT[0] / Size::PLAYER_ASPECT[1]);
        let height = width * Size::PLAYER_ASPECT[1] / Size::PLAYER_ASPECT[0];
        Rect::from_min_size(
            pos2(available.center().x - width / 2.0, available.top()),
            vec2(width, height),
        )
    };
    let toolbar = if fullscreen {
        Rect::from_min_max(
            pos2(
                stage.left(),
                (stage.bottom() - Size::PLAYER_TOOLBAR).max(stage.top()),
            ),
            stage.right_bottom(),
        )
    } else {
        Rect::from_min_size(
            pos2(stage.left(), stage.bottom()),
            vec2(stage.width(), Size::PLAYER_TOOLBAR),
        )
    };
    PlayerRects { stage, toolbar }
}

/// Renders one windowed player whose stage and attached toolbar share a width.
pub fn player_surface<S, T>(
    ui: &mut Ui,
    stage: impl FnOnce(&mut Ui) -> S,
    toolbar: impl FnOnce(&mut Ui) -> T,
) -> PlayerSurfaceResponse<S, T> {
    let available = ui.available_rect_before_wrap();
    let available = Rect::from_min_size(
        available.min,
        vec2(
            available.width().min(Size::PAGE_MEDIUM_MAX),
            available.height(),
        ),
    );
    let rects = player_rects(available, false);
    let response = ui.allocate_rect(rects.stage.union(rects.toolbar), Sense::hover());
    let stage = player_stage_at(ui, rects.stage, stage);
    let toolbar = player_toolbar_at(ui, rects.toolbar, toolbar);
    PlayerSurfaceResponse {
        rects,
        stage,
        toolbar,
        response,
    }
}

/// Renders a stable 16:9 dark player stage up to the shared watch width.
pub fn player_stage<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width().min(Size::PAGE_MEDIUM_MAX);
    let height = width * Size::PLAYER_ASPECT[1] / Size::PLAYER_ASPECT[0];
    let item_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let rect = ui.allocate_space(vec2(width, height)).1;
    ui.spacing_mut().item_spacing.y = item_spacing;
    player_stage_at(ui, rect, content)
}

/// Renders a dark player stage at a caller-owned stable rectangle.
pub fn player_stage_at<R>(ui: &mut Ui, rect: Rect, content: impl FnOnce(&mut Ui) -> R) -> R {
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
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    content(&mut child)
}

/// Renders a fifty-two-point dark toolbar with forty-point controls.
pub fn player_toolbar<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> (Response, R) {
    let width = ui.available_width().min(Size::PAGE_MEDIUM_MAX);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, Size::PLAYER_TOOLBAR), Sense::hover());
    let inner = player_toolbar_at(ui, rect, content);
    (response, inner)
}

/// Renders a player toolbar at a caller-owned rectangle anchored to its stage.
pub fn player_toolbar_at<R>(ui: &mut Ui, rect: Rect, content: impl FnOnce(&mut Ui) -> R) -> R {
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
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(inner_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = Size::PLAYER_TOOLBAR_ITEM_SPACING;
    child.set_min_height(Size::CONTROL);
    child.set_max_height(Size::CONTROL);
    content(&mut child)
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
    fn player_toolbar_fills_available_width_up_to_the_player_limit() {
        egui::__run_test_ui(|ui| {
            ui.set_width(640.0);
            let (response, ()) = player_toolbar(ui, |_| ());
            assert_eq!(response.rect.width(), 640.0);
        });
        egui::__run_test_ui(|ui| {
            ui.set_width(Size::PAGE_WIDE_MAX);
            let (response, ()) = player_toolbar(ui, |_| ());
            assert_eq!(response.rect.width(), Size::PAGE_MEDIUM_MAX);
        });
    }

    #[test]
    fn player_rects_keep_the_toolbar_attached_in_windowed_and_fullscreen_modes() {
        let available = Rect::from_min_size(pos2(20.0, 30.0), vec2(1000.0, 700.0));
        let windowed = player_rects(available, false);
        assert_eq!(windowed.stage.width(), Size::PAGE_MEDIUM_MAX);
        assert_eq!(windowed.stage.width() / windowed.stage.height(), 16.0 / 9.0);
        assert_eq!(windowed.toolbar.width(), windowed.stage.width());
        assert_eq!(windowed.toolbar.top(), windowed.stage.bottom());
        assert_eq!(windowed.toolbar.height(), Size::PLAYER_TOOLBAR);
        assert_eq!(
            windowed.toolbar.bottom() - windowed.stage.top(),
            windowed.stage.height() + Size::PLAYER_TOOLBAR
        );

        let fullscreen = player_rects(available, true);
        assert_eq!(fullscreen.stage, available);
        assert_eq!(fullscreen.toolbar.width(), fullscreen.stage.width());
        assert_eq!(fullscreen.toolbar.bottom(), fullscreen.stage.bottom());
        assert_eq!(fullscreen.toolbar.height(), Size::PLAYER_TOOLBAR);
    }

    #[test]
    fn player_surface_keeps_stage_and_toolbar_equal_width_and_adjacent() {
        egui::__run_test_ui(|ui| {
            ui.set_width(Size::PAGE_WIDE_MAX);
            let output = player_surface(ui, |ui| ui.max_rect(), |ui| ui.max_rect());
            assert_eq!(output.rects.toolbar.width(), output.rects.stage.width());
            assert_eq!(output.rects.toolbar.top(), output.rects.stage.bottom());
            assert_eq!(
                output.rects.toolbar.bottom() - output.rects.stage.top(),
                output.rects.stage.height() + Size::PLAYER_TOOLBAR
            );
        });
    }

    #[test]
    fn player_surface_stays_inside_a_height_constrained_viewport() {
        egui::__run_test_ui(|ui| {
            ui.set_width(640.0);
            ui.set_height(300.0);
            let available = ui.available_rect_before_wrap();
            let output = player_surface(ui, |_| (), |_| ());

            assert!(output.response.rect.width() <= available.width());
            assert!(output.response.rect.height() <= available.height());
            assert_eq!(output.rects.toolbar.bottom(), available.bottom());
        });
    }
}
