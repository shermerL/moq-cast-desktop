use std::{fmt::Debug, hash::Hash};

use egui::{
    Align, Id, Label, Layout, Rect, Response, ScrollArea, Ui, UiBuilder, WidgetInfo, WidgetType,
    containers::scroll_area::ScrollBarVisibility, vec2,
};

use crate::{COLORS, ControlRole, Interaction, Size, Spacing, TypographyRole, typography};

use super::{
    common::{effective_enabled, paint_focus, paint_surface, pointing_hand, resolve, sense},
    state_panel::{BadgeTone, status_badge},
};

/// Display-only configuration for a settings row.
#[derive(Clone, Copy, Debug)]
pub struct SettingRowSpec<'a> {
    title: &'a str,
    description: Option<&'a str>,
}

/// Display-only configuration for a compact key/value row.
#[derive(Clone, Copy, Debug)]
pub struct DetailRowSpec<'a> {
    label: &'a str,
}

impl<'a> DetailRowSpec<'a> {
    /// Creates a compact row with a leading label.
    pub fn new(label: &'a str) -> Self {
        Self { label }
    }
}

impl<'a> SettingRowSpec<'a> {
    /// Creates a settings row with a primary label.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            description: None,
        }
    }

    /// Adds supporting copy below the primary label.
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }
}

/// Display-only configuration for a selectable device row.
#[derive(Clone, Copy, Debug)]
pub struct DeviceRowSpec<'a> {
    id: Id,
    title: &'a str,
    detail: Option<&'a str>,
    selected: bool,
    enabled: bool,
    preview: Option<Interaction>,
}

/// Display-only badge attached to a device-list item.
#[derive(Clone, Copy, Debug)]
pub struct DeviceBadgeSpec<'a> {
    label: &'a str,
    tone: BadgeTone,
}

impl<'a> DeviceBadgeSpec<'a> {
    /// Creates a badge with a semantic tone.
    pub fn new(label: &'a str, tone: BadgeTone) -> Self {
        Self { label, tone }
    }
}

/// Pure presentation data for one item in a shared device list.
#[derive(Clone, Debug)]
pub struct DeviceListItemSpec<'a, I> {
    id: I,
    title: &'a str,
    subtitle: Option<&'a str>,
    badge: Option<DeviceBadgeSpec<'a>>,
    selected: bool,
    enabled: bool,
    preview: Option<Interaction>,
}

impl<'a, I> DeviceListItemSpec<'a, I> {
    /// Creates an enabled device item with a stable caller-owned ID.
    pub fn new(id: I, title: &'a str) -> Self {
        Self {
            id,
            title,
            subtitle: None,
            badge: None,
            selected: false,
            enabled: true,
            preview: None,
        }
    }

    /// Adds supporting device state or identity copy.
    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    /// Adds a compact capability or status badge.
    pub fn badge(mut self, badge: DeviceBadgeSpec<'a>) -> Self {
        self.badge = Some(badge);
        self
    }

    /// Sets the item's selected state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets whether the item accepts pointer or keyboard activation.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Forces a deterministic interaction state for review fixtures.
    pub fn preview_interaction(mut self, state: Interaction) -> Self {
        self.preview = Some(state);
        self
    }
}

/// Display-only configuration for a shared device list.
#[derive(Clone, Copy, Debug)]
pub struct DeviceListSpec<'a, I> {
    id: Id,
    items: &'a [DeviceListItemSpec<'a, I>],
    viewport_height: Option<f32>,
}

impl<'a, I> DeviceListSpec<'a, I> {
    /// Creates a device list with a stable scroll-area ID and item slice.
    pub fn new(id: Id, items: &'a [DeviceListItemSpec<'a, I>]) -> Self {
        Self {
            id,
            items,
            viewport_height: None,
        }
    }

    /// Fills a caller-owned viewport with this logical height.
    pub fn viewport_height(mut self, height: f32) -> Self {
        self.viewport_height = Some(height.max(Size::DEVICE_ROW));
        self
    }
}

impl<'a> DeviceRowSpec<'a> {
    /// Creates an enabled device row with a stable ID and title.
    pub fn new(id: Id, title: &'a str) -> Self {
        Self {
            id,
            title,
            detail: None,
            selected: false,
            enabled: true,
            preview: None,
        }
    }

    /// Adds supporting device state or metadata.
    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Sets the row's selected state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets whether the row accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Forces a deterministic state for catalogs and visual tests.
    pub fn preview_interaction(mut self, state: Interaction) -> Self {
        self.preview = Some(state);
        self
    }
}

/// Renders a responsive settings row with a caller-owned trailing control.
pub fn setting_row<R>(
    ui: &mut Ui,
    spec: SettingRowSpec<'_>,
    trailing: impl FnOnce(&mut Ui) -> R,
) -> R {
    let stacked = ui.available_width() < Size::SETTINGS_BREAKPOINT;
    let height = if stacked {
        Size::SETTING_ROW + Spacing::SM + Size::CONTROL
    } else {
        Size::SETTING_ROW
    };
    let rect = ui.allocate_space(vec2(ui.available_width(), height)).1;
    let inner = rect.shrink2(vec2(Size::ROW_HORIZONTAL_INSET, 0.0));
    let label_height = Size::SETTING_ROW;

    if stacked {
        let label_rect = egui::Rect::from_min_size(inner.min, vec2(inner.width(), label_height));
        paint_setting_label(ui, label_rect, spec);
        let trailing_rect = egui::Rect::from_min_size(
            egui::pos2(inner.left(), label_rect.bottom() + Spacing::SM),
            vec2(inner.width(), Size::CONTROL),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(trailing_rect)
                .layout(Layout::left_to_right(Align::Center)),
            trailing,
        )
        .inner
    } else {
        let trailing_width = Size::SETTING_CONTROL_MAX.min(inner.width() / 2.0);
        let label_width = (inner.width() - trailing_width - Spacing::LG).max(1.0);
        let label_rect = egui::Rect::from_min_size(inner.min, vec2(label_width, label_height));
        let trailing_rect = egui::Rect::from_min_size(
            egui::pos2(inner.right() - trailing_width, inner.top()),
            vec2(trailing_width, label_height),
        );
        paint_setting_label(ui, label_rect, spec);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(trailing_rect)
                .layout(Layout::right_to_left(Align::Center)),
            trailing,
        )
        .inner
    }
}

fn paint_setting_label(ui: &mut Ui, rect: egui::Rect, spec: SettingRowSpec<'_>) {
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = Spacing::XS;
                ui.set_max_width(rect.width());
                ui.label(typography(
                    spec.title,
                    TypographyRole::Row,
                    COLORS.text.into(),
                ));
                if let Some(description) = spec.description {
                    ui.label(typography(
                        description,
                        TypographyRole::Help,
                        COLORS.muted.into(),
                    ));
                }
            });
        },
    );
}

/// Renders a fixed-height key/value row that never switches to a stacked layout.
pub fn detail_row<R>(
    ui: &mut Ui,
    spec: DetailRowSpec<'_>,
    trailing: impl FnOnce(&mut Ui) -> R,
) -> (egui::Rect, R) {
    let rect = ui
        .allocate_space(vec2(ui.available_width(), Size::DETAIL_ROW))
        .1;
    let inner = rect.shrink2(vec2(Size::ROW_HORIZONTAL_INSET, 0.0));
    let trailing_width = (inner.width() / 2.0).min(Size::SETTING_CONTROL_MAX);
    let label_width = (inner.width() - trailing_width - Spacing::SM).max(1.0);
    let label_rect = egui::Rect::from_min_size(inner.min, vec2(label_width, inner.height()));
    let trailing_rect = egui::Rect::from_min_size(
        egui::pos2(inner.right() - trailing_width, inner.top()),
        vec2(trailing_width, inner.height()),
    );

    let mut label_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(label_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    label_ui.label(typography(
        spec.label,
        TypographyRole::Meta,
        COLORS.muted.into(),
    ));
    let mut trailing_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(trailing_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    (rect, trailing(&mut trailing_ui))
}

/// Renders a selectable device row with a caller-owned trailing control.
pub fn device_row<R>(
    ui: &mut Ui,
    spec: DeviceRowSpec<'_>,
    trailing: impl FnOnce(&mut Ui) -> R,
) -> (Response, R) {
    let output = render_device_row(ui, spec, trailing);
    (output.response, output.inner)
}

struct DeviceRowOutput<R> {
    response: Response,
    inner: R,
    content_rect: Rect,
    #[cfg(test)]
    title_rect: Rect,
    #[cfg(test)]
    detail_rect: Option<Rect>,
}

fn render_device_row<R>(
    ui: &mut Ui,
    spec: DeviceRowSpec<'_>,
    trailing: impl FnOnce(&mut Ui) -> R,
) -> DeviceRowOutput<R> {
    let rect = ui
        .allocate_space(vec2(ui.available_width(), Size::DEVICE_ROW))
        .1;
    let enabled = effective_enabled(spec.enabled, spec.preview);
    let mut response = pointing_hand(ui.interact(rect, spec.id, sense(enabled)), enabled);
    let (state, visual) = resolve(
        &response,
        ControlRole::Secondary,
        enabled,
        spec.selected,
        spec.preview,
    );
    paint_surface(ui, rect, visual, crate::Radius::MD as u8);
    let content_rect = rect.shrink2(vec2(Size::ROW_HORIZONTAL_INSET, Spacing::MD));
    let mut content_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(content_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    content_ui.set_clip_rect(content_rect.intersect(ui.clip_rect()));
    let inner = trailing(&mut content_ui);
    let text_rect = content_ui.available_rect_before_wrap();
    let mut text_ui = content_ui.new_child(
        UiBuilder::new().max_rect(text_rect).layout(
            Layout::top_down(Align::Min)
                .with_main_align(Align::Center)
                .with_cross_justify(true),
        ),
    );
    text_ui.set_clip_rect(text_rect.intersect(content_ui.clip_rect()));
    text_ui.spacing_mut().item_spacing.y = Spacing::XS;
    let title_response = pointing_hand(
        text_ui.add(
            Label::new(typography(
                spec.title,
                TypographyRole::Row,
                visual.text.into(),
            ))
            .sense(sense(enabled))
            .truncate(),
        ),
        enabled,
    );
    #[cfg(test)]
    let title_rect = title_response.rect;
    response |= title_response;
    #[cfg(test)]
    let mut detail_rect = None;
    if let Some(detail) = spec.detail {
        let detail_response = pointing_hand(
            text_ui.add(
                Label::new(typography(
                    detail,
                    TypographyRole::Meta,
                    COLORS.muted.into(),
                ))
                .sense(sense(enabled))
                .truncate(),
            ),
            enabled,
        );
        #[cfg(test)]
        {
            detail_rect = Some(detail_response.rect);
        }
        response |= detail_response;
    }
    paint_focus(ui, rect, &response, state, crate::Radius::MD);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            enabled,
            spec.selected,
            spec.title,
        )
    });
    DeviceRowOutput {
        response,
        inner,
        content_rect: text_rect,
        #[cfg(test)]
        title_rect,
        #[cfg(test)]
        detail_rect,
    }
}

/// Renders a scrollable device list and returns the activated stable item ID.
pub fn device_list<I>(ui: &mut Ui, spec: DeviceListSpec<'_, I>) -> Option<I>
where
    I: Clone + Debug + Hash,
{
    render_device_list(ui, spec).activated
}

#[cfg_attr(not(test), allow(dead_code))]
struct DeviceListOutput<I> {
    activated: Option<I>,
    row_rects: Vec<Rect>,
    content_rects: Vec<Rect>,
    #[cfg(test)]
    hit_rects: Vec<DeviceListHitRects>,
    content_rect: Rect,
    content_cursor_y: f32,
    viewport_rect: Rect,
    content_size: egui::Vec2,
}

#[cfg(test)]
struct DeviceListHitRects {
    title: Rect,
    detail: Option<Rect>,
    badge: Option<Rect>,
}

struct DeviceListContent<I> {
    activated: Option<I>,
    row_rects: Vec<Rect>,
    content_rects: Vec<Rect>,
    #[cfg(test)]
    hit_rects: Vec<DeviceListHitRects>,
    content_cursor_y: f32,
}

fn render_device_list<I>(ui: &mut Ui, spec: DeviceListSpec<'_, I>) -> DeviceListOutput<I>
where
    I: Clone + Debug + Hash,
{
    let viewport_height = spec.viewport_height.unwrap_or(Size::DEVICE_LIST_MAX_HEIGHT);
    let shrink_vertically = spec.viewport_height.is_none();
    let output = ScrollArea::vertical()
        .id_salt(spec.id)
        .max_height(viewport_height)
        .min_scrolled_height(viewport_height)
        .auto_shrink([false, shrink_vertically])
        .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let item_spacing_y = ui.spacing().item_spacing.y;
            ui.spacing_mut().item_spacing.y = Spacing::NONE;
            let mut activated = None;
            let mut row_rects = Vec::with_capacity(spec.items.len());
            let mut content_rects = Vec::with_capacity(spec.items.len());
            #[cfg(test)]
            let mut hit_rects = Vec::with_capacity(spec.items.len());

            for (index, item) in spec.items.iter().enumerate() {
                if index > 0 {
                    ui.add_space(Spacing::SM);
                }
                let badge = item.badge;
                let mut row_spec = DeviceRowSpec::new(spec.id.with(&item.id), item.title)
                    .selected(item.selected)
                    .enabled(item.enabled);
                if let Some(subtitle) = item.subtitle {
                    row_spec = row_spec.detail(subtitle);
                }
                if let Some(preview) = item.preview {
                    row_spec = row_spec.preview_interaction(preview);
                }
                let enabled = effective_enabled(item.enabled, item.preview);
                let output = render_device_row(ui, row_spec, |ui| {
                    badge.map(|badge| status_badge(ui, badge.label, badge.tone))
                });
                #[cfg(test)]
                let badge_rect = output.inner.as_ref().map(|response| response.rect);
                let badge_clicked = output.inner.is_some_and(|response| {
                    pointing_hand(response.interact(sense(enabled)), enabled).clicked()
                });
                let keyboard_activated = enabled
                    && output.response.has_focus()
                    && ui.input(|input| {
                        input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
                    });
                if enabled && (output.response.clicked() || badge_clicked || keyboard_activated) {
                    activated = Some(item.id.clone());
                }
                row_rects.push(output.response.rect);
                content_rects.push(output.content_rect);
                #[cfg(test)]
                hit_rects.push(DeviceListHitRects {
                    title: output.title_rect,
                    detail: output.detail_rect,
                    badge: badge_rect,
                });
            }
            let content_cursor_y = ui.next_widget_position().y;
            ui.spacing_mut().item_spacing.y = item_spacing_y;
            DeviceListContent {
                activated,
                row_rects,
                content_rects,
                #[cfg(test)]
                hit_rects,
                content_cursor_y,
            }
        });
    let content = output.inner;
    let content_rect = content
        .row_rects
        .iter()
        .copied()
        .reduce(Rect::union)
        .unwrap_or(Rect::NOTHING);
    DeviceListOutput {
        activated: content.activated,
        row_rects: content.row_rects,
        content_rects: content.content_rects,
        #[cfg(test)]
        hit_rects: content.hit_rects,
        content_rect,
        content_cursor_y: content.content_cursor_y,
        viewport_rect: output.inner_rect,
        content_size: output.content_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLICK_LIST_ID: &str = "device-list-click-fixture";
    const CLICK_ITEM_ID: &str = "peer-a";

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

    fn render_click_fixture(
        context: &egui::Context,
        input: egui::RawInput,
        enabled: bool,
    ) -> DeviceListOutput<&'static str> {
        let items = [DeviceListItemSpec::new(CLICK_ITEM_ID, "Conference display")
            .subtitle("Connected · A1B2")
            .badge(DeviceBadgeSpec::new("Watch", BadgeTone::Info))
            .enabled(enabled)];
        let mut output = None;
        let frame = context.run_ui(input, |ui| {
            ui.set_width(Size::NEARBY_LIST);
            output = Some(render_device_list(
                ui,
                DeviceListSpec::new(Id::new(CLICK_LIST_ID), &items),
            ));
        });
        frame.drop_without_applying_deltas();
        output.expect("device list fixture renders")
    }

    fn click_fixture(
        context: &egui::Context,
        pos: egui::Pos2,
        enabled: bool,
    ) -> DeviceListOutput<&'static str> {
        render_click_fixture(context, pointer_input(pos, true), enabled);
        render_click_fixture(context, pointer_input(pos, false), enabled)
    }

    #[test]
    fn detail_rows_keep_a_fixed_compact_height_in_narrow_columns() {
        egui::__run_test_ui(|ui| {
            ui.set_width(Size::NEARBY_LIST);
            let (rect, trailing) = detail_row(ui, DetailRowSpec::new("Connection"), |ui| {
                ui.label("Connected").rect
            });
            assert_eq!(rect.height(), Size::DETAIL_ROW);
            assert_eq!(rect.width(), Size::NEARBY_LIST);
            assert!(trailing.center().y >= rect.top());
            assert!(trailing.center().y <= rect.bottom());
        });
    }

    #[test]
    fn settings_controls_stay_inside_wide_and_stacked_rows() {
        for width in [720.0, 620.0] {
            egui::__run_test_ui(|ui| {
                ui.set_width(width);
                let row_bounds = ui.available_rect_before_wrap();
                let response = setting_row(ui, SettingRowSpec::new("Language"), |ui| {
                    ui.allocate_exact_size(
                        vec2(Size::SETTING_CONTROL_MAX, Size::CONTROL),
                        egui::Sense::hover(),
                    )
                    .1
                });
                assert!(response.rect.left() >= row_bounds.left());
                assert!(response.rect.right() <= row_bounds.left() + width);
            });
        }
    }

    #[test]
    fn adjacent_device_rows_consume_their_full_rect_without_overlap() {
        for (case, width) in [Size::NEARBY_LIST, 240.0].into_iter().enumerate() {
            egui::__run_test_ui(|ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = Spacing::SM;
                let (first, ()) = device_row(
                    ui,
                    DeviceRowSpec::new(Id::new(("device-row", case, 1)), "First device")
                        .detail("Available"),
                    |_| {},
                );
                let (second, ()) = device_row(
                    ui,
                    DeviceRowSpec::new(Id::new(("device-row", case, 2)), "Second device")
                        .detail("Available"),
                    |_| {},
                );

                assert_eq!(first.rect.height(), Size::DEVICE_ROW);
                assert_eq!(second.rect.height(), Size::DEVICE_ROW);
                assert_eq!(second.rect.top() - first.rect.bottom(), Spacing::SM);
                assert_eq!(ui.min_rect().bottom(), second.rect.bottom());
            });
        }
    }

    #[test]
    fn device_list_keeps_standard_and_narrow_rows_disjoint_without_tail_spacing() {
        for (case, width) in [Size::NEARBY_LIST, 240.0].into_iter().enumerate() {
            egui::__run_test_ui(|ui| {
                ui.set_width(width);
                let items = [
                    DeviceListItemSpec::new("peer-a", "First device")
                        .subtitle("Discovered")
                        .badge(DeviceBadgeSpec::new("Found", BadgeTone::Neutral))
                        .preview_interaction(Interaction::Hovered),
                    DeviceListItemSpec::new("peer-b", "Second device")
                        .subtitle("Connected · A1B2")
                        .badge(DeviceBadgeSpec::new("Connected", BadgeTone::Info))
                        .selected(true),
                    DeviceListItemSpec::new("peer-c", "Third device")
                        .subtitle("Unavailable · C3D4")
                        .badge(DeviceBadgeSpec::new("Unavailable", BadgeTone::Warning))
                        .enabled(false)
                        .preview_interaction(Interaction::Focused),
                ];
                let output = render_device_list(
                    ui,
                    DeviceListSpec::new(Id::new(("device-list-geometry", case)), &items),
                );

                assert_eq!(output.row_rects.len(), items.len());
                for pair in output.row_rects.windows(2) {
                    assert!(!pair[0].intersects(pair[1]));
                    assert_eq!(pair[1].top() - pair[0].bottom(), Spacing::SM);
                }
                for (row, content) in output.row_rects.iter().zip(&output.content_rects) {
                    assert!(row.contains(content.min));
                    assert!(row.contains(content.max));
                    assert!(content.width() >= 0.0);
                }
                let last = output.row_rects.last().expect("fixture is not empty");
                assert_eq!(output.content_rect.bottom(), last.bottom());
                assert_eq!(output.content_cursor_y, last.bottom());
                assert!(output.content_size.y <= output.viewport_rect.height());
            });
        }
    }

    #[test]
    fn device_list_activates_the_same_item_across_the_entire_display_row() {
        for target in ["title", "subtitle", "row whitespace", "status badge"] {
            let context = egui::Context::default();
            let initial = render_click_fixture(&context, egui::RawInput::default(), true);
            let pos = match target {
                "title" => initial.hit_rects[0].title.center(),
                "subtitle" => initial.hit_rects[0]
                    .detail
                    .expect("fixture has a subtitle")
                    .center(),
                "row whitespace" => egui::pos2(
                    initial.row_rects[0].left() + Spacing::XS,
                    initial.row_rects[0].center().y,
                ),
                "status badge" => initial.hit_rects[0]
                    .badge
                    .expect("fixture has a status badge")
                    .center(),
                _ => unreachable!(),
            };
            let clicked = click_fixture(&context, pos, true);
            assert_eq!(clicked.activated, Some(CLICK_ITEM_ID), "{target}");
        }
    }

    #[test]
    fn disabled_device_list_items_ignore_pointer_and_keyboard_activation() {
        let context = egui::Context::default();
        let initial = render_click_fixture(&context, egui::RawInput::default(), false);
        let clicked = click_fixture(&context, initial.hit_rects[0].title.center(), false);
        assert!(clicked.activated.is_none());

        context.memory_mut(|memory| {
            memory.request_focus(Id::new(CLICK_LIST_ID).with(CLICK_ITEM_ID));
        });
        let keyed = render_click_fixture(&context, key_input(egui::Key::Enter), false);
        assert!(keyed.activated.is_none());
    }

    #[test]
    fn focused_device_list_items_keep_keyboard_activation() {
        let context = egui::Context::default();
        render_click_fixture(&context, egui::RawInput::default(), true);
        context.memory_mut(|memory| {
            memory.request_focus(Id::new(CLICK_LIST_ID).with(CLICK_ITEM_ID));
        });

        let output = render_click_fixture(&context, key_input(egui::Key::Space), true);
        assert_eq!(output.activated, Some(CLICK_ITEM_ID));
    }

    #[test]
    fn long_and_duplicate_device_names_stay_clipped_inside_fixed_rows() {
        egui::__run_test_ui(|ui| {
            ui.set_width(240.0);
            let items = [
                DeviceListItemSpec::new(
                    "peer-cn",
                    "会议室里用于演示和跨设备联调的超长中文设备名称",
                )
                .subtitle("同名设备 · 7F2A")
                .badge(DeviceBadgeSpec::new("可观看", BadgeTone::Info)),
                DeviceListItemSpec::new(
                    "peer-en",
                    "Conference room presentation and interoperability workstation",
                )
                .subtitle("Same name · 9C4D")
                .badge(DeviceBadgeSpec::new("Watch", BadgeTone::Info)),
            ];
            let output =
                render_device_list(ui, DeviceListSpec::new(Id::new("long-device-list"), &items));

            for (row, content) in output.row_rects.iter().zip(&output.content_rects) {
                assert_eq!(row.height(), Size::DEVICE_ROW);
                assert!(row.contains(content.min));
                assert!(row.contains(content.max));
            }
        });
    }

    #[test]
    fn long_device_lists_scroll_and_keep_the_last_row_tail_free() {
        egui::__run_test_ui(|ui| {
            ui.set_width(Size::NEARBY_LIST);
            let titles = (1..=7)
                .map(|index| format!("Review device {index}"))
                .collect::<Vec<_>>();
            let items = titles
                .iter()
                .enumerate()
                .map(|(index, title)| {
                    DeviceListItemSpec::new(index, title.as_str())
                        .subtitle("Review fixture")
                        .badge(DeviceBadgeSpec::new(
                            if index % 2 == 0 { "Found" } else { "Watch" },
                            if index % 2 == 0 {
                                BadgeTone::Neutral
                            } else {
                                BadgeTone::Info
                            },
                        ))
                })
                .collect::<Vec<_>>();
            let output = render_device_list(
                ui,
                DeviceListSpec::new(Id::new("scroll-device-list"), &items),
            );

            assert_eq!(output.row_rects.len(), 7);
            assert!(output.content_size.y > output.viewport_rect.height());
            assert!(output.viewport_rect.height() <= Size::DEVICE_LIST_MAX_HEIGHT);
            for pair in output.row_rects.windows(2) {
                assert!(!pair[0].intersects(pair[1]));
                assert_eq!(pair[1].top() - pair[0].bottom(), Spacing::SM);
            }
            let first = output.row_rects.first().expect("fixture is not empty");
            let last = output.row_rects.last().expect("fixture is not empty");
            assert_eq!(output.content_rect.top(), first.top());
            assert_eq!(output.content_rect.bottom(), last.bottom());
            assert_eq!(output.content_cursor_y, last.bottom());
        });
    }

    #[test]
    fn device_list_can_fill_a_caller_owned_remaining_height() {
        egui::__run_test_ui(|ui| {
            ui.set_width(Size::NEARBY_LIST);
            let titles = (1..=8)
                .map(|index| format!("Device {index}"))
                .collect::<Vec<_>>();
            let items = titles
                .iter()
                .enumerate()
                .map(|(index, title)| DeviceListItemSpec::new(index, title.as_str()))
                .collect::<Vec<_>>();
            let viewport_height = Size::DEVICE_ROW * 3.0 + Spacing::SM * 2.0;
            let output = render_device_list(
                ui,
                DeviceListSpec::new(Id::new("remaining-height-device-list"), &items)
                    .viewport_height(viewport_height),
            );

            assert_eq!(output.viewport_rect.height(), viewport_height);
            assert!(output.content_size.y > output.viewport_rect.height());
            assert!(
                output
                    .row_rects
                    .iter()
                    .all(|row| row.height() == Size::DEVICE_ROW)
            );
        });
    }

    #[test]
    fn empty_device_lists_do_not_activate_or_allocate_rows() {
        egui::__run_test_ui(|ui| {
            ui.set_width(Size::NEARBY_LIST);
            let items: [DeviceListItemSpec<'_, u8>; 0] = [];
            let output = render_device_list(
                ui,
                DeviceListSpec::new(Id::new("empty-device-list"), &items),
            );
            assert!(output.activated.is_none());
            assert!(output.row_rects.is_empty());
            assert!(output.content_rects.is_empty());
        });
    }
}
