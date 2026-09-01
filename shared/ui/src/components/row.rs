use egui::{Align, Id, Layout, Response, Ui, UiBuilder, WidgetInfo, WidgetType, vec2};

use crate::{COLORS, ControlRole, Interaction, Size, Spacing, TypographyRole, typography};

use super::common::{paint_focus, paint_surface, resolve, sense};

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
    let rect = ui
        .allocate_space(vec2(ui.available_width(), Size::DEVICE_ROW))
        .1;
    let response = ui.interact(rect, spec.id, sense(spec.enabled));
    let (state, visual) = resolve(
        &response,
        ControlRole::Secondary,
        spec.enabled,
        spec.selected,
        spec.preview,
    );
    paint_surface(ui, rect, visual, crate::Radius::MD as u8);
    let inner = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(Size::ROW_HORIZONTAL_INSET, Spacing::MD)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.vertical(|ui| {
                    ui.label(typography(
                        spec.title,
                        TypographyRole::Row,
                        visual.text.into(),
                    ));
                    if let Some(detail) = spec.detail {
                        ui.label(typography(
                            detail,
                            TypographyRole::Meta,
                            COLORS.muted.into(),
                        ));
                    }
                });
                let available = ui.available_rect_before_wrap();
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(available)
                        .layout(Layout::right_to_left(Align::Center)),
                    trailing,
                )
                .inner
            },
        )
        .inner;
    paint_focus(ui, rect, &response, state, crate::Radius::MD);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            spec.enabled,
            spec.selected,
            spec.title,
        )
    });
    (response, inner)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
