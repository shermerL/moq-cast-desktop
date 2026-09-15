use egui::{
    Align, Color32, CornerRadius, Layout, Rect, RectAlign, Response, Sense, Shape, Slider, Stroke,
    Tooltip, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::{COLORS, ControlRole, Radius, Size, Spacing};

use super::common::{color, paint_focus, paint_surface, pointing_hand, resolve, sense};

/// Default volume for every new remote playback session.
pub const DEFAULT_PLAYER_VOLUME_PERCENT: u8 = 100;

/// Stable width of the mute button and volume slider in a player toolbar.
pub const PLAYER_VOLUME_CONTROL_WIDTH: f32 = 132.0;

const PLAYER_VOLUME_SLIDER_WIDTH: f32 = 84.0;
const PLAYER_VOLUME_RAIL_HEIGHT: f32 = 4.0;
const PLAYER_VOLUME_THUMB_RADIUS: f32 = 6.0;
const PLAYER_VOLUME_THUMB_RING_RADIUS: f32 = 9.0;
const PLAYER_VOLUME_ICON_SIZE: f32 = 20.0;

/// Session-scoped player volume, including the value restored after unmuting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerVolumeState {
    generation: Option<u64>,
    percent: u8,
    last_nonzero_percent: u8,
}

impl Default for PlayerVolumeState {
    fn default() -> Self {
        Self {
            generation: None,
            percent: DEFAULT_PLAYER_VOLUME_PERCENT,
            last_nonzero_percent: DEFAULT_PLAYER_VOLUME_PERCENT,
        }
    }
}

impl PlayerVolumeState {
    /// Resets volume to 100 percent when a different playback session appears.
    pub fn sync_generation(&mut self, generation: u64) -> bool {
        if self.generation == Some(generation) {
            return false;
        }
        self.generation = Some(generation);
        self.percent = DEFAULT_PLAYER_VOLUME_PERCENT;
        self.last_nonzero_percent = DEFAULT_PLAYER_VOLUME_PERCENT;
        true
    }

    /// Returns the current volume from zero through one hundred percent.
    pub fn percent(self) -> u8 {
        self.percent
    }

    /// Returns whether playback is currently muted.
    pub fn muted(self) -> bool {
        self.percent == 0
    }

    /// Sets and clamps the current volume, preserving the last nonzero value.
    pub fn set_percent(&mut self, percent: u8) -> bool {
        let percent = percent.min(100);
        if self.percent == percent {
            return false;
        }
        self.percent = percent;
        if percent > 0 {
            self.last_nonzero_percent = percent;
        }
        true
    }

    /// Toggles mute and returns the resulting volume percentage.
    pub fn toggle_mute(&mut self) -> u8 {
        if self.muted() {
            self.percent = self.last_nonzero_percent.max(1);
        } else {
            self.percent = 0;
        }
        self.percent
    }
}

/// Interaction result from one player volume control render.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct PlayerVolumeResponse {
    changed_percent: Option<u8>,
    active: bool,
    #[cfg(test)]
    mute_rect: Rect,
    #[cfg(test)]
    slider_rect: Rect,
    #[cfg(test)]
    slider_id: egui::Id,
}

impl PlayerVolumeResponse {
    /// Returns a new percentage when mute or slider input changed the volume.
    pub fn changed_percent(self) -> Option<u8> {
        self.changed_percent
    }

    /// Returns whether the control is hovered, focused, or being dragged.
    pub fn active(self) -> bool {
        self.active
    }
}

/// Renders a fixed-width mute button and volume slider for a player toolbar.
pub fn player_volume_control(
    ui: &mut Ui,
    state: &mut PlayerVolumeState,
    enabled: bool,
    mute_label: &str,
    unmute_label: &str,
    volume_label: &str,
    unavailable_label: &str,
) -> PlayerVolumeResponse {
    let (rect, _) = ui.allocate_exact_size(
        vec2(PLAYER_VOLUME_CONTROL_WIDTH, Size::CONTROL),
        Sense::hover(),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = Size::PLAYER_TOOLBAR_ITEM_SPACING;

    let mut changed = None;
    let muted = state.muted();
    let mute_accessible_label = if enabled {
        if muted { unmute_label } else { mute_label }
    } else {
        unavailable_label
    };
    let mute = child
        .add_enabled_ui(enabled, |ui| {
            player_volume_icon_button(ui, muted, mute_accessible_label)
        })
        .inner;
    let mute_clicked = mute.clicked();
    let mute_active =
        mute.contains_pointer() || mute.has_focus() || mute.is_pointer_button_down_on();
    if enabled {
        mute.clone().on_hover_text(mute_accessible_label);
    } else {
        mute.clone().on_disabled_hover_text(unavailable_label);
    }
    if mute_clicked {
        changed = Some(state.toggle_mute());
    }

    let mut percent = state.percent();
    let slider = child
        .add_enabled_ui(enabled, |ui| {
            player_volume_slider(ui, &mut percent, volume_label)
        })
        .inner;
    let slider_changed = slider.changed();
    let slider_active =
        slider.contains_pointer() || slider.has_focus() || slider.is_pointer_button_down_on();
    if enabled && slider_active {
        let mut tooltip = Tooltip::for_widget(&slider);
        tooltip.popup = tooltip.popup.align(RectAlign::TOP);
        tooltip.show(|ui| {
            ui.label(format!("{percent}%"));
        });
    } else if !enabled {
        slider.clone().on_disabled_hover_text(unavailable_label);
    }
    if slider_changed && state.set_percent(percent) {
        changed = Some(state.percent());
    }

    PlayerVolumeResponse {
        changed_percent: changed,
        active: mute_active || slider_active,
        #[cfg(test)]
        mute_rect: mute.rect,
        #[cfg(test)]
        slider_rect: slider.rect,
        #[cfg(test)]
        slider_id: slider.id,
    }
}

fn player_volume_icon_button(ui: &mut Ui, muted: bool, accessible_label: &str) -> Response {
    let enabled = ui.is_enabled();
    let (rect, response) =
        ui.allocate_exact_size(vec2(Size::CONTROL, Size::CONTROL), sense(enabled));
    let response = pointing_hand(response, enabled);
    let (interaction, visual) = resolve(&response, ControlRole::PlayerIcon, enabled, false, None);
    paint_surface(ui, rect, visual, Radius::MD as u8);
    paint_speaker_icon(
        ui,
        Rect::from_center_size(
            rect.center(),
            vec2(PLAYER_VOLUME_ICON_SIZE, PLAYER_VOLUME_ICON_SIZE),
        ),
        muted,
        color(visual.text, visual.opacity),
    );
    paint_focus(ui, rect, &response, interaction, Radius::MD);
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, muted, accessible_label));
    response
}

fn paint_speaker_icon(ui: &Ui, rect: Rect, muted: bool, icon_color: Color32) {
    let center = rect.center();
    let body = vec![
        pos2(center.x - 9.0, center.y - 3.0),
        pos2(center.x - 5.0, center.y - 3.0),
        pos2(center.x, center.y - 7.0),
        pos2(center.x, center.y + 7.0),
        pos2(center.x - 5.0, center.y + 3.0),
        pos2(center.x - 9.0, center.y + 3.0),
    ];
    let stroke = Stroke::new(1.8, icon_color);
    ui.painter().add(Shape::closed_line(body, stroke));
    if muted {
        ui.painter().line_segment(
            [
                pos2(center.x + 3.0, center.y - 4.0),
                pos2(center.x + 9.0, center.y + 4.0),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                pos2(center.x + 9.0, center.y - 4.0),
                pos2(center.x + 3.0, center.y + 4.0),
            ],
            stroke,
        );
        return;
    }

    let wave = (0..=8)
        .map(|step| {
            let angle = -0.85 + 1.7 * step as f32 / 8.0;
            pos2(center.x + angle.cos() * 8.0, center.y + angle.sin() * 8.0)
        })
        .collect();
    ui.painter().add(Shape::line(wave, stroke));
}

fn player_volume_slider(ui: &mut Ui, percent: &mut u8, accessible_label: &str) -> Response {
    let enabled = ui.is_enabled();
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = PLAYER_VOLUME_SLIDER_WIDTH;
            ui.spacing_mut().slider_rail_height = PLAYER_VOLUME_RAIL_HEIGHT;
            ui.spacing_mut().interact_size.y = Size::CONTROL;
            ui.visuals_mut().selection.bg_fill = COLORS.brand.into();
            let widgets = &mut ui.visuals_mut().widgets;
            for visual in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                visual.bg_fill = COLORS.player_bar.into();
                visual.fg_stroke = Stroke::NONE;
            }
            ui.add(
                Slider::new(percent, 0..=100)
                    .show_value(false)
                    .step_by(1.0)
                    .trailing_fill(true)
                    .handle_shape(egui::style::HandleShape::Rect {
                        aspect_ratio: 0.375,
                    }),
            )
        })
        .inner;

    response.widget_info(|| WidgetInfo::slider(enabled, *percent as f64, accessible_label));
    paint_player_volume_slider(ui, response.rect, &response, *percent, enabled);
    response
}

fn paint_player_volume_slider(
    ui: &Ui,
    rect: Rect,
    response: &Response,
    percent: u8,
    enabled: bool,
) {
    let opacity = if enabled { 1.0 } else { Size::DISABLED_ALPHA };
    let rail = Rect::from_center_size(
        rect.center(),
        vec2(PLAYER_VOLUME_SLIDER_WIDTH, PLAYER_VOLUME_RAIL_HEIGHT),
    );
    let thumb_x = rect.left()
        + PLAYER_VOLUME_THUMB_RADIUS
        + (rect.width() - PLAYER_VOLUME_THUMB_RADIUS * 2.0) * percent as f32 / 100.0;
    let thumb_center = pos2(thumb_x, rect.center().y);
    let remaining = Color32::from(COLORS.player_muted).gamma_multiply(0.32 * opacity);
    let fill = Color32::from(COLORS.brand).gamma_multiply(opacity);
    let thumb = Color32::from(if enabled {
        COLORS.player_text
    } else {
        COLORS.player_muted
    })
    .gamma_multiply(opacity);

    ui.painter()
        .rect_filled(rail, CornerRadius::same(2), remaining);
    let fill_width = player_volume_fill_width(percent, enabled);
    if fill_width > 0.0 {
        let fill_end = rail.left() + fill_width;
        let filled = Rect::from_min_max(rail.min, pos2(fill_end, rail.max.y));
        ui.painter()
            .rect_filled(filled, CornerRadius::same(2), fill);
    }
    if enabled && (response.has_focus() || response.is_pointer_button_down_on()) {
        ui.painter().circle_stroke(
            thumb_center,
            PLAYER_VOLUME_THUMB_RING_RADIUS,
            Stroke::new(Size::FOCUS, COLORS.brand),
        );
    }
    ui.painter()
        .circle_filled(thumb_center, PLAYER_VOLUME_THUMB_RADIUS, thumb);
}

fn player_volume_fill_width(percent: u8, enabled: bool) -> f32 {
    if enabled {
        PLAYER_VOLUME_SLIDER_WIDTH * percent.min(100) as f32 / 100.0
    } else {
        0.0
    }
}

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

    fn pointer_input(pos: egui::Pos2, pressed: bool) -> egui::RawInput {
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        }
    }

    fn key_input(key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        }
    }

    fn render_volume_fixture(
        context: &egui::Context,
        input: egui::RawInput,
        state: &mut PlayerVolumeState,
        enabled: bool,
    ) -> PlayerVolumeResponse {
        let mut output = None;
        let frame = context.run_ui(input, |ui| {
            output = Some(player_volume_control(
                ui,
                state,
                enabled,
                "Mute",
                "Unmute",
                "Playback volume",
                "No playable audio",
            ));
        });
        frame.drop_without_applying_deltas();
        output.expect("volume fixture renders")
    }

    #[test]
    fn player_volume_restores_the_last_nonzero_value_after_unmuting() {
        let mut volume = PlayerVolumeState::default();
        volume.sync_generation(1);
        assert!(volume.set_percent(37));
        assert_eq!(volume.toggle_mute(), 0);
        assert!(volume.muted());
        assert_eq!(volume.toggle_mute(), 37);
        assert!(!volume.muted());
    }

    #[test]
    fn slider_zero_preserves_the_value_used_by_unmute() {
        let mut volume = PlayerVolumeState::default();
        volume.sync_generation(1);
        volume.set_percent(62);
        volume.set_percent(0);
        assert_eq!(volume.toggle_mute(), 62);
    }

    #[test]
    fn a_new_playback_generation_resets_volume_to_full() {
        let mut volume = PlayerVolumeState::default();
        assert!(volume.sync_generation(4));
        volume.set_percent(28);
        assert!(!volume.sync_generation(4));
        assert_eq!(volume.percent(), 28);
        assert!(volume.sync_generation(5));
        assert_eq!(volume.percent(), DEFAULT_PLAYER_VOLUME_PERCENT);
        assert!(!volume.muted());
    }

    #[test]
    fn player_volume_keeps_fixed_button_gap_and_slider_hit_areas() {
        let context = egui::Context::default();
        let mut volume = PlayerVolumeState::default();
        let output = render_volume_fixture(&context, egui::RawInput::default(), &mut volume, true);

        assert_eq!(output.mute_rect.size(), vec2(Size::CONTROL, Size::CONTROL));
        assert_eq!(
            output.slider_rect.size(),
            vec2(PLAYER_VOLUME_SLIDER_WIDTH, Size::CONTROL)
        );
        assert_eq!(
            output.slider_rect.left() - output.mute_rect.right(),
            Size::PLAYER_TOOLBAR_ITEM_SPACING
        );
        assert_eq!(
            output.slider_rect.right() - output.mute_rect.left(),
            PLAYER_VOLUME_CONTROL_WIDTH
        );
    }

    #[test]
    fn player_volume_drag_updates_integer_percent_and_reports_activity() {
        let context = egui::Context::default();
        let mut volume = PlayerVolumeState::default();
        let initial = render_volume_fixture(&context, egui::RawInput::default(), &mut volume, true);
        let pointer = pos2(
            initial.slider_rect.center().x,
            initial.slider_rect.center().y,
        );
        let dragged =
            render_volume_fixture(&context, pointer_input(pointer, true), &mut volume, true);

        assert_eq!(dragged.changed_percent(), Some(50));
        assert_eq!(volume.percent(), 50);
        assert!(dragged.active());
    }

    #[test]
    fn focused_player_volume_keeps_native_arrow_adjustment_and_activity() {
        let context = egui::Context::default();
        let mut volume = PlayerVolumeState::default();
        let initial = render_volume_fixture(&context, egui::RawInput::default(), &mut volume, true);
        context.memory_mut(|memory| memory.request_focus(initial.slider_id));
        let focused =
            render_volume_fixture(&context, key_input(egui::Key::ArrowLeft), &mut volume, true);

        assert_eq!(focused.changed_percent(), Some(99));
        assert_eq!(volume.percent(), 99);
        assert!(focused.active());
    }

    #[test]
    fn disabled_player_volume_keeps_geometry_without_accepting_input() {
        let context = egui::Context::default();
        let mut volume = PlayerVolumeState::default();
        volume.set_percent(64);
        let initial =
            render_volume_fixture(&context, egui::RawInput::default(), &mut volume, false);
        let pointer = initial.slider_rect.center();
        let disabled =
            render_volume_fixture(&context, pointer_input(pointer, true), &mut volume, false);

        assert_eq!(disabled.changed_percent(), None);
        assert_eq!(volume.percent(), 64);
        assert!(disabled.active());
        assert_eq!(disabled.slider_rect, initial.slider_rect);
    }

    #[test]
    fn player_volume_fill_has_exact_zero_full_and_disabled_boundaries() {
        assert_eq!(player_volume_fill_width(0, true), 0.0);
        assert_eq!(
            player_volume_fill_width(100, true),
            PLAYER_VOLUME_SLIDER_WIDTH
        );
        assert_eq!(player_volume_fill_width(68, false), 0.0);
    }

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
