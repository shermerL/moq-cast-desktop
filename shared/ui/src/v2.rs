//! Opt-in S2 presentation primitives; existing platform components keep their theme.

use egui::{Color32, Context, Frame, Margin, Response, RichText, Stroke, Ui, Vec2};

/// Semantic colors for the S2 desktop shell.
#[derive(Clone, Copy)]
pub struct Palette {
    /// Main window background.
    pub canvas: Color32,
    /// Card background.
    pub surface: Color32,
    /// Primary text.
    pub text: Color32,
    /// Supporting text.
    pub muted: Color32,
    /// Card and control outline.
    pub border: Color32,
    /// Primary action color.
    pub accent: Color32,
    /// Indicator for a healthy service, not proof of a remote connection.
    pub success: Color32,
    /// Attention needed while part of a task remains usable.
    pub warning: Color32,
    /// Foreground for stopping media or disconnecting services.
    pub danger: Color32,
    /// Background for stopping media or disconnecting services.
    pub danger_soft: Color32,
    /// Tinted identity background.
    pub soft: Color32,
    /// Hovered control background.
    pub hover: Color32,
}

impl Palette {
    /// Resolves the palette from the active native theme.
    pub fn for_ui(ui: &Ui) -> Self {
        Self::new(ui.visuals().dark_mode)
    }

    /// Returns the light or dark S2 palette.
    pub fn new(dark: bool) -> Self {
        let rgb = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        if dark {
            Self {
                canvas: rgb(0x17191a),
                surface: rgb(0x222526),
                text: rgb(0xe5e8e8),
                muted: rgb(0xa4adaf),
                border: rgb(0x343b3d),
                accent: rgb(0x9ec7be),
                success: rgb(0x68c990),
                warning: rgb(0xe8ba69),
                danger: rgb(0xffb0a9),
                danger_soft: rgb(0x3e2827),
                soft: rgb(0x293e3a),
                hover: rgb(0x2b3334),
            }
        } else {
            Self {
                canvas: rgb(0xeff5f8),
                surface: Color32::WHITE,
                text: rgb(0x17232d),
                muted: rgb(0x65757f),
                border: rgb(0xdce5e9),
                accent: rgb(0x167c82),
                success: rgb(0x40a66d),
                warning: rgb(0x8b5b12),
                danger: rgb(0xae2b24),
                danger_soft: rgb(0xfce8e6),
                soft: rgb(0xe8f7f7),
                hover: rgb(0xf0f7f8),
            }
        }
    }

    /// Builds a dialog with consistent space around its body and actions.
    pub fn dialog(self) -> Frame {
        self.card().corner_radius(14).inner_margin(24)
    }

    /// Builds a rounded content card.
    pub fn card(self) -> Frame {
        Frame::new()
            .fill(self.surface)
            .stroke(Stroke::new(1.0, self.border))
            .corner_radius(12)
            .inner_margin(Margin::symmetric(24, 18))
    }
}

/// Installs both palettes without changing the user's theme preference.
pub fn install(context: &Context) {
    for (theme, dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let p = Palette::new(dark);
        context.style_mut_of(theme, |style| {
            style.visuals = if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            style.visuals.override_text_color = Some(p.text);
            style.visuals.panel_fill = p.canvas;
            style.visuals.window_fill = p.surface;
            style.visuals.window_stroke = Stroke::new(1.0, p.border);
            style.visuals.window_corner_radius = 12.into();
            style.visuals.selection.bg_fill = p.soft;
            style.visuals.selection.stroke = Stroke::new(1.0, p.accent);
            style.visuals.hyperlink_color = p.accent;
            style.visuals.faint_bg_color = p.soft;
            style.visuals.extreme_bg_color = p.surface;
            style.visuals.widgets.inactive.weak_bg_fill = p.surface;
            style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
            style.visuals.widgets.hovered.weak_bg_fill = p.surface.lerp_to_gamma(p.accent, 0.18);
            style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.5, p.accent);
            style.visuals.widgets.active.weak_bg_fill = p.surface.lerp_to_gamma(p.accent, 0.28);
            style.visuals.widgets.active.bg_stroke = Stroke::new(2.0, p.accent);
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style.spacing.item_spacing = egui::vec2(12.0, 12.0);
            style.spacing.button_padding = egui::vec2(16.0, 10.0);
            style.spacing.interact_size.y = 40.0;
        });
    }
}

fn action_width(ui: &Ui, label: &str, minimum: f32) -> f32 {
    let galley = egui::WidgetText::from(action_text(label, Palette::for_ui(ui).accent))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontSelection::Default,
        );
    minimum.max(galley.size().x + 2.0 * ui.spacing().button_padding.x)
}

fn action_text(label: &str, color: Color32) -> RichText {
    RichText::new(label)
        .size(14.0)
        .line_height(Some(20.0))
        .variation(b"wght", 500.0)
        .color(color)
}

/// Paints the bundled MoQCast PNG without theme tinting.
pub fn brand_mark(ui: &mut Ui) {
    let id = egui::Id::new("v2-moqcast-brand");
    let texture = ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let texture = ui.ctx().load_texture(
                "MoQCast logo",
                egui::ColorImage::from_rgba_unmultiplied(
                    [128, 128],
                    include_bytes!("../assets/brand/moqcast.rgba"),
                ),
                egui::TextureOptions::LINEAR,
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    ui.add(egui::Image::new((texture.id(), egui::vec2(32.0, 32.0))).alt_text("MoQCast"));
}

/// Display-only meaning of a status supplied by the platform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusTone {
    /// No active operation or special attention required.
    #[default]
    Neutral,
    /// Media is active or available.
    Active,
    /// An operation is awaiting completion.
    Busy,
    /// A service is functioning normally.
    Success,
    /// Some capability requires attention.
    Warning,
    /// The requested operation failed.
    Error,
}

impl StatusTone {
    /// Returns a theme-aware foreground for this meaning.
    pub fn color(self, palette: Palette) -> Color32 {
        match self {
            Self::Neutral => palette.muted,
            Self::Active | Self::Busy => palette.accent,
            Self::Success => palette.success,
            Self::Warning => palette.warning,
            Self::Error => palette.danger,
        }
    }

    /// Draws a compact indicator beside an accompanying status label.
    pub fn indicator(self, ui: &mut Ui) {
        let color = self.color(Palette::for_ui(ui));
        if self == Self::Busy {
            ui.add(egui::Spinner::new().size(12.0).color(color));
        } else {
            let (_, rect) = ui.allocate_space(egui::vec2(12.0, 12.0));
            if matches!(self, Self::Warning | Self::Error) {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "!",
                    egui::FontId::proportional(15.0),
                    color,
                );
            } else {
                ui.painter().circle_filled(rect.center(), 4.0, color);
            }
        }
    }
}

/// Shows a wrapping status message with a semantic indicator.
pub fn notice(ui: &mut Ui, tone: StatusTone, message: &str) {
    let p = Palette::for_ui(ui);
    Frame::new()
        .fill(p.hover)
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                tone.indicator(ui);
                ui.add(egui::Label::new(RichText::new(message).color(tone.color(p))).wrap());
            });
        });
}

/// Visual meaning of an action, independent of its enabled state.
#[derive(Clone, Copy, Default)]
pub enum ActionTone {
    /// Opens or starts the next step.
    #[default]
    Accent,
    /// Starts the primary task on this surface.
    Primary,
    /// Dismisses an informational surface without stopping work.
    Neutral,
    /// Stops active work or disconnects a service.
    Danger,
}

/// A labeled action with a minimum forty-point hit area.
pub struct Action<'a> {
    /// User-visible action and accessibility label.
    pub label: &'a str,
    /// Minimum requested size; text may require more width.
    pub size: Vec2,
    /// Whether the action may be activated.
    pub enabled: bool,
    /// Visual meaning of the operation.
    pub tone: ActionTone,
}

impl Action<'_> {
    /// Renders an action using the active S2 theme.
    pub fn show(self, ui: &mut Ui) -> Response {
        let p = Palette::for_ui(ui);
        let (foreground, fill, border) = match self.tone {
            ActionTone::Accent => (p.accent, p.surface, p.accent),
            ActionTone::Primary => (p.accent, p.soft, p.accent),
            ActionTone::Neutral => (p.text, p.hover, p.border),
            ActionTone::Danger => (p.danger, p.danger_soft, p.danger),
        };
        let size = egui::vec2(
            action_width(ui, self.label, self.size.x.max(40.0)),
            self.size.y.max(40.0),
        );
        ui.allocate_ui_with_layout(
            size,
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let widgets = &mut ui.style_mut().visuals.widgets;
                widgets.inactive.weak_bg_fill = fill;
                widgets.inactive.bg_stroke = Stroke::new(1.0, border);
                widgets.hovered.weak_bg_fill = fill.lerp_to_gamma(foreground, 0.18);
                widgets.hovered.bg_stroke = Stroke::new(1.5, foreground);
                widgets.active.weak_bg_fill = fill.lerp_to_gamma(foreground, 0.28);
                widgets.active.bg_stroke = Stroke::new(2.0, foreground);
                ui.add_enabled(
                    self.enabled,
                    egui::Button::new(action_text(self.label, foreground))
                        .min_size(size)
                        .wrap()
                        .corner_radius(6),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
            },
        )
        .inner
    }
}

/// Renders an accent action for opening or starting a task.
pub fn action(ui: &mut Ui, label: &str, size: Vec2, enabled: bool) -> Response {
    Action {
        label,
        size,
        enabled,
        tone: ActionTone::Accent,
    }
    .show(ui)
}

/// Renders a dialog title and a right-aligned close button without stopping work.
pub fn dialog_header(ui: &mut Ui, title: &str, close_label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(20.0).variation(b"wght", 600.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            icon_button(ui, Icon::Close, close_label)
        })
        .inner
    })
    .inner
}

/// Paints a neutral display identity glyph in an already allocated rectangle.
pub fn monitor(ui: &Ui, rect: egui::Rect, color: Color32) {
    let center = rect.center();
    let screen =
        egui::Rect::from_center_size(center - egui::vec2(0.0, 3.0), egui::vec2(28.0, 19.0));
    let stroke = Stroke::new(1.8, color);
    ui.painter()
        .rect_stroke(screen, 3, stroke, egui::StrokeKind::Inside);
    ui.painter().line_segment(
        [
            center + egui::vec2(0.0, 7.0),
            center + egui::vec2(0.0, 12.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(-7.0, 12.0),
            center + egui::vec2(7.0, 12.0),
        ],
        stroke,
    );
}

/// Window-wide layout metrics, resolved before applying page insets.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Whether secondary row status should move under the device name.
    pub compact: bool,
    /// Horizontal workspace padding.
    pub inset: i8,
    /// Header height including its contents.
    pub header: f32,
    /// Footer height including its controls.
    pub footer: f32,
    /// Minimum height of the local-device card including padding.
    pub local_card: f32,
    /// Vertical gap between workspace cards.
    pub gap: f32,
}

impl Metrics {
    /// Resolves the S2 breakpoint from the full content width, not a nested card.
    pub fn for_width(width: f32) -> Self {
        let compact = width < 920.0;
        Self {
            compact,
            inset: if compact { 16 } else { 24 },
            header: 56.0,
            footer: 62.0,
            local_card: 112.0,
            gap: if compact { 12.0 } else { 16.0 },
        }
    }
}

/// Display-only content for a Nearby row with separate details and watch actions.
pub struct NearbyRow<'a> {
    /// Stable device key supplied by the platform.
    pub id: egui::Id,
    /// User-facing device name, never an authenticated identity.
    pub name: &'a str,
    /// Supporting identity text shown in the wide layout.
    pub subtitle: &'a str,
    /// Current platform-projected availability.
    pub status: &'a str,
    /// Meaning of the platform-projected status.
    pub tone: StatusTone,
    /// This row owns the currently active playback.
    pub current: bool,
    /// Label of the watch or return action.
    pub watch_label: &'a str,
    /// Whether the platform currently permits the watch action.
    pub watch_enabled: bool,
    /// Accessible label for opening device details.
    pub details_label: &'a str,
    /// Uses the compact window layout.
    pub compact: bool,
}

/// Independent actions emitted by one Nearby row.
#[derive(Default)]
pub struct RowResponse {
    /// Open the device details without starting playback.
    pub details: bool,
    /// Start or return to playback using the platform's current snapshot.
    pub watch: bool,
}

struct RowRects {
    identity: egui::Rect,
    status: egui::Rect,
    watch: egui::Rect,
    details: egui::Rect,
}

fn row_rects(row: egui::Rect, compact: bool, action_width: f32) -> RowRects {
    let gap = if compact { 12.0 } else { 24.0 };
    let details = egui::Rect::from_center_size(
        egui::pos2(row.right() - 20.0, row.center().y),
        egui::vec2(40.0, 40.0),
    );
    let width = action_width.max(if compact { 108.0 } else { 128.0 });
    let watch = egui::Rect::from_min_size(
        egui::pos2(details.left() - gap - width, row.center().y - 21.0),
        egui::vec2(width, 42.0),
    );
    let status = egui::Rect::from_min_max(
        egui::pos2(
            watch.left() - gap - if compact { 0.0 } else { 128.0 },
            row.top(),
        ),
        egui::pos2(watch.left() - gap, row.bottom()),
    );
    let identity = egui::Rect::from_min_max(
        row.min,
        egui::pos2(
            if compact {
                watch.left() - gap
            } else {
                status.left() - gap
            },
            row.bottom(),
        ),
    );
    RowRects {
        identity,
        status,
        watch,
        details,
    }
}

fn label_at(ui: &mut Ui, rect: egui::Rect, label: egui::Label) -> Response {
    // Ui::place centers the galley even when the label's own text alignment is left.
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.add(label),
    )
    .inner
}

/// Renders an accessible, bounded row without changing the source-picker list API.
pub fn nearby_row(ui: &mut Ui, spec: NearbyRow<'_>) -> RowResponse {
    let p = Palette::for_ui(ui);
    let mut actions = RowResponse::default();
    let subtitle = if spec.compact {
        spec.status
    } else {
        spec.subtitle
    };
    ui.push_id(spec.id, |ui| {
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), if spec.compact { 76.0 } else { 88.0 }),
            egui::Sense::hover(),
        );
        let rects = row_rects(row, spec.compact, action_width(ui, spec.watch_label, 0.0));
        if spec.current {
            ui.painter().rect_filled(row.shrink(2.0), 6, p.soft);
        }
        ui.painter().line_segment(
            [row.left_bottom(), row.right_bottom()],
            Stroke::new(1.0, p.border),
        );
        let response = ui.interact(
            rects.identity,
            ui.id().with("identity"),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{}: {}", spec.details_label, spec.name),
            )
        });
        if response.enabled() && (response.hovered() || response.is_pointer_button_down_on()) {
            let amount = if response.is_pointer_button_down_on() {
                0.28
            } else {
                0.18
            };
            ui.painter().rect_filled(
                rects.identity.shrink(3.0),
                6,
                p.surface.lerp_to_gamma(p.accent, amount),
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rects.identity.shrink(2.0),
                6,
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        actions.details = response.clicked();
        if response.has_focus() {
            response.scroll_to_me(None);
        }
        monitor(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(row.left() + 16.0, row.center().y),
                egui::vec2(32.0, 40.0),
            ),
            p.muted,
        );
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(row.left() + 56.0, row.center().y - 25.0),
            egui::pos2(rects.identity.right() - 4.0, row.center().y + 2.0),
        );
        label_at(
            ui,
            name_rect,
            egui::Label::new(
                RichText::new(spec.name)
                    .size(17.0)
                    .variation(b"wght", 600.0),
            )
            .selectable(false)
            .truncate()
            .halign(egui::Align::Min),
        );
        let subtitle_rect = egui::Rect::from_min_max(
            egui::pos2(name_rect.left(), row.center().y + 5.0),
            egui::pos2(name_rect.right(), row.center().y + 25.0),
        );
        label_at(
            ui,
            subtitle_rect,
            egui::Label::new(RichText::new(subtitle).size(13.0).color(if spec.compact {
                spec.tone.color(p)
            } else {
                p.muted
            }))
            .selectable(false)
            .truncate()
            .halign(egui::Align::Min),
        );
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("{}\n{}", spec.name, subtitle));
        if !spec.compact {
            if spec.tone == StatusTone::Busy {
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                        egui::pos2(rects.status.left(), row.center().y - 6.0),
                        egui::vec2(12.0, 12.0),
                    )),
                    |ui| spec.tone.indicator(ui),
                );
            }
            let mut status_rect = rects.status;
            if spec.tone == StatusTone::Busy {
                status_rect.min.x += 18.0;
            }
            label_at(
                ui,
                status_rect,
                egui::Label::new(
                    RichText::new(spec.status)
                        .size(15.0)
                        .color(spec.tone.color(p)),
                )
                .selectable(false)
                .truncate()
                .halign(egui::Align::Min),
            );
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(rects.watch), |ui| {
            let response = Action {
                label: spec.watch_label,
                size: rects.watch.size(),
                enabled: spec.watch_enabled,
                tone: ActionTone::Primary,
            }
            .show(ui);
            if response.has_focus() {
                response.scroll_to_me(None);
            }
            actions.watch = response.clicked();
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(rects.details), |ui| {
            let response = icon_button(ui, Icon::Information, spec.details_label);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spec.details_label)
            });
            if response.has_focus() {
                response.scroll_to_me(None);
            }
            actions.details |= response.clicked();
            response.on_hover_text(spec.details_label);
        });
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_workspace_fills_width_and_reserves_controls() {
        for size in [egui::vec2(420.0, 400.0), egui::vec2(1000.0, 700.0)] {
            let area = egui::Rect::from_min_size(egui::pos2(16.0, 56.0), size);
            let rects = player_rects(area, false);
            assert_eq!(rects.stage.left_top(), area.left_top());
            assert_eq!(rects.stage.width(), area.width());
            assert_eq!(rects.toolbar.width(), area.width());
            assert_eq!(rects.toolbar.height(), PLAYER_TOOLBAR_HEIGHT);
            assert_eq!(rects.stage.bottom(), rects.toolbar.top());
            assert_eq!(rects.toolbar.right_bottom(), area.right_bottom());
        }
    }

    #[test]
    fn fullscreen_controls_overlay_the_full_stage() {
        let area = egui::Rect::from_min_size(egui::pos2(12.0, 34.0), egui::vec2(800.0, 600.0));
        let rects = player_rects(area, true);
        assert_eq!(rects.stage, area);
        assert_eq!(rects.toolbar.height(), PLAYER_TOOLBAR_HEIGHT);
        assert_eq!(rects.toolbar.right_bottom(), area.right_bottom());
    }

    #[test]
    fn small_player_workspace_never_inverts_rectangles() {
        for height in [0.0, 1.0, 32.0, PLAYER_TOOLBAR_HEIGHT] {
            let area = egui::Rect::from_min_size(egui::pos2(12.0, 34.0), egui::vec2(320.0, height));
            for fullscreen in [false, true] {
                let rects = player_rects(area, fullscreen);
                for rect in [rects.stage, rects.toolbar] {
                    assert!(rect.height() >= 0.0);
                    assert!(area.contains_rect(rect));
                }
            }
        }
    }

    #[test]
    fn identity_text_starts_at_its_inset_instead_of_centering_in_the_column() {
        let ctx = Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let rect =
                    egui::Rect::from_min_size(egui::pos2(100.0, 60.0), egui::vec2(300.0, 30.0));
                label_at(
                    ui,
                    rect,
                    egui::Label::new("Device name")
                        .truncate()
                        .halign(egui::Align::Min),
                );
            });
        });
        let text_left = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == "Device name" => {
                    Some(text.pos.x)
                }
                _ => None,
            })
            .expect("the label must paint its text");
        output.textures_delta.clear();
        assert!(
            (text_left - 100.0).abs() < 1.0,
            "painted text starts at {text_left}, expected 100"
        );
    }

    #[test]
    fn context_card_padding_does_not_accumulate_with_parent_spacing() {
        for width in [680.0, 920.0, 1040.0] {
            let ctx = Context::default();
            install(&ctx);
            crate::install_ui_font(&ctx, std::borrow::Cow::Borrowed(crate::NOTO_SANS_SC));
            let m = Metrics::for_width(width);
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 520.0),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |root| {
                    egui::CentralPanel::default()
                        .frame(Frame::new().inner_margin(m.inset))
                        .show(root, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let top = ui.cursor().top();
                            let bound = ui.available_rect_before_wrap();
                            let response = context_card(
                                ui,
                                ContextCard {
                                    heading: "本机",
                                    name: "很长的设备名称，用于检查小窗口的边界",
                                    identity: "Current run ID 0123456789abcdef",
                                    status: "Idle",
                                    tone: StatusTone::Neutral,
                                    action: "共享屏幕 / 窗口",
                                },
                                m,
                            );
                            assert!(bound.contains_rect(response.rect));
                            let height = ui.cursor().top() - top;
                            assert!(
                                height <= m.local_card + 4.0,
                                "height {height} at width {width}"
                            );
                        });
                });
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn nearby_status_remains_visible_without_metadata_in_both_layouts() {
        for compact in [true, false] {
            for theme in [egui::Theme::Light, egui::Theme::Dark] {
                let ctx = Context::default();
                ctx.set_theme(theme);
                install(&ctx);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(740.0, 100.0),
                        )),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            nearby_row(
                                ui,
                                NearbyRow {
                                    id: egui::Id::new("peer"),
                                    name: "Device",
                                    subtitle: "",
                                    status: "Not sharing",
                                    tone: StatusTone::Neutral,
                                    current: false,
                                    watch_label: "Watch",
                                    watch_enabled: false,
                                    details_label: "Details",
                                    compact,
                                },
                            );
                        });
                    },
                );
                let status_count = output
                    .shapes
                    .iter()
                    .filter(|clipped| {
                        matches!(&clipped.shape, egui::epaint::Shape::Text(text)
                        if text.galley.job.text == "Not sharing")
                    })
                    .count();
                output.textures_delta.clear();
                assert_eq!(status_count, 1, "compact={compact}, theme={theme:?}");
            }
        }
    }

    #[test]
    fn nearby_actions_fit_small_and_large_workspaces_without_overlap() {
        for width in [680.0, 919.0, 920.0, 1040.0, 1440.0] {
            let m = Metrics::for_width(width);
            let card_padding = 48.0;
            let row = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(
                    width.min(1240.0) - 2.0 * f32::from(m.inset) - card_padding,
                    88.0,
                ),
            );
            let r = row_rects(row, m.compact, 128.0);
            assert!(r.identity.width() >= 120.0);
            assert!(r.identity.right() < r.watch.left());
            assert!(r.watch.right() < r.details.left());
            assert!(row.contains_rect(r.watch));
            assert!(row.contains_rect(r.details));
            assert!(r.details.height() >= 40.0);
        }
    }

    #[test]
    fn device_details_never_emit_watch_and_disabled_watch_does_not_activate() {
        for (point, enabled, expect_details, expect_watch) in [
            (egui::pos2(100.0, 44.0), true, true, false),
            (egui::pos2(612.0, 44.0), true, false, true),
            (egui::pos2(720.0, 44.0), true, true, false),
            (egui::pos2(612.0, 44.0), false, false, false),
        ] {
            let ctx = Context::default();
            install(&ctx);
            let mut result = RowResponse::default();
            for events in [
                vec![],
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            ] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(740.0, 100.0),
                    )),
                    events,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |root| {
                    egui::CentralPanel::default()
                        .frame(Frame::NONE)
                        .show(root, |ui| {
                            result = nearby_row(
                                ui,
                                NearbyRow {
                                    id: egui::Id::new("peer"),
                                    name: "Long device name",
                                    subtitle: "Current run ID",
                                    status: "Available",
                                    tone: StatusTone::Active,
                                    current: false,
                                    watch_label: "Watch",
                                    watch_enabled: enabled,
                                    details_label: "Details",
                                    compact: false,
                                },
                            );
                        });
                });
                output.textures_delta.clear();
            }
            assert_eq!(
                result.details, expect_details,
                "details at {point:?}, enabled={enabled}"
            );
            assert_eq!(
                result.watch, expect_watch,
                "watch at {point:?}, enabled={enabled}"
            );
        }
    }
}

/// Local-device content projected by a platform without media ownership in the UI library.
pub struct ContextCard<'a> {
    /// Local-device section label.
    pub heading: &'a str,
    /// Local device name.
    pub name: &'a str,
    /// Current-run identifier text.
    pub identity: &'a str,
    /// Actual media task state.
    pub status: &'a str,
    /// Meaning of the platform-projected status.
    pub tone: StatusTone,
    /// Open sharing setup or return to playback.
    pub action: &'a str,
}

/// Renders the local-device context with one bounded primary action.
pub fn context_card(ui: &mut Ui, content: ContextCard<'_>, metrics: Metrics) -> Response {
    let p = Palette::for_ui(ui);
    let mut response = None;
    p.card()
        .inner_margin(Margin::symmetric(if metrics.compact { 18 } else { 24 }, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(metrics.local_card - if metrics.compact { 32.0 } else { 38.0 });
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(content.heading).size(14.0).color(p.muted));
            ui.add_space(6.0);
            let (row, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
            let badge = egui::Rect::from_min_size(row.min, egui::vec2(40.0, 56.0));

            monitor(ui, badge, p.accent);
            let action_width = action_width(
                ui,
                content.action,
                if metrics.compact { 148.0 } else { 174.0 },
            );
            let action_rect = egui::Rect::from_center_size(
                egui::pos2(row.right() - action_width / 2.0, row.center().y),
                egui::vec2(action_width, 46.0),
            );
            let name_left = badge.right() + if metrics.compact { 16.0 } else { 22.0 };
            let name_right = action_rect.left() - 16.0;
            let name = egui::Rect::from_min_max(
                egui::pos2(name_left, row.center().y - 28.0),
                egui::pos2(name_right, row.center().y + 3.0),
            );
            label_at(
                ui,
                name,
                egui::Label::new(
                    RichText::new(content.name)
                        .size(16.0)
                        .variation(b"wght", 600.0),
                )
                .truncate()
                .halign(egui::Align::Min),
            )
            .on_hover_text(content.name);
            let meta = egui::Rect::from_min_max(
                egui::pos2(name_left, row.center().y + 8.0),
                egui::pos2(name_right, row.center().y + 28.0),
            );
            label_at(
                ui,
                meta,
                egui::Label::new(
                    RichText::new(content.status)
                        .size(13.0)
                        .color(content.tone.color(p)),
                )
                .truncate()
                .halign(egui::Align::Min),
            )
            .on_hover_text(content.identity);
            ui.scope_builder(egui::UiBuilder::new().max_rect(action_rect), |ui| {
                response = Some(action(ui, content.action, action_rect.size(), true));
            });
        });
    response.expect("the context card always renders its action")
}

/// Small native line icons used by the desktop shell.
#[derive(Clone, Copy)]
pub enum Icon {
    /// Dismisses the current dialog.
    Close,
    /// Opens application preferences.
    Settings,
    /// Opens read-only device details.
    Information,
    /// Changes the native window fullscreen state.
    Fullscreen,
}

/// Renders a line icon with a forty-point target and a descriptive accessibility label.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let response = ui.add(
        egui::Button::new("")
            .min_size(egui::vec2(40.0, 40.0))
            .corner_radius(6),
    );
    let p = Palette::for_ui(ui);
    let center = response.rect.center();
    let interacting = response.enabled()
        && (response.hovered() || response.is_pointer_button_down_on() || response.has_focus());
    let ink = if interacting { p.accent } else { p.muted };
    let stroke = Stroke::new(if interacting { 2.0 } else { 1.7 }, ink);
    match icon {
        Icon::Close => {
            let id = egui::Id::new("v2-tabler-close");
            let texture = ui
                .ctx()
                .data_mut(|data| data.get_temp::<egui::TextureHandle>(id))
                .unwrap_or_else(|| {
                    let texture = ui.ctx().load_texture(
                        "Tabler X",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [72, 72],
                            include_bytes!("../assets/icons/tabler-x.rgba"),
                        ),
                        Default::default(),
                    );
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(id, texture.clone()));
                    texture
                });
            ui.painter().image(
                texture.id(),
                egui::Rect::from_center_size(center, egui::vec2(24.0, 24.0)),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                if interacting { p.accent } else { p.text },
            );
        }
        Icon::Information => {
            ui.painter().circle_stroke(center, 9.0, stroke);
            ui.painter()
                .circle_filled(center - egui::vec2(0.0, 4.0), 1.1, ink);
            ui.painter().line_segment(
                [center - egui::vec2(0.0, 0.5), center + egui::vec2(0.0, 5.0)],
                stroke,
            );
        }
        Icon::Fullscreen => {
            let r = egui::Rect::from_center_size(center, egui::vec2(18.0, 18.0));
            for (corner, x, y) in [
                (r.left_top(), 1.0, 1.0),
                (r.right_top(), -1.0, 1.0),
                (r.left_bottom(), 1.0, -1.0),
                (r.right_bottom(), -1.0, -1.0),
            ] {
                ui.painter()
                    .line_segment([corner + egui::vec2(x * 6.0, 0.0), corner], stroke);
                ui.painter()
                    .line_segment([corner, corner + egui::vec2(0.0, y * 6.0)], stroke);
            }
        }
        Icon::Settings => {
            ui.painter().circle_stroke(center, 7.0, stroke);
            ui.painter().circle_stroke(center, 2.5, stroke);
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                ui.painter().line_segment(
                    [center + direction * 7.0, center + direction * 10.0],
                    stroke,
                );
            }
        }
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// Height shared by the windowed and fullscreen player controls.
pub const PLAYER_TOOLBAR_HEIGHT: f32 = 64.0;

/// Divides the full video workspace into a stage and attached or overlaid controls.
pub fn player_rects(available: egui::Rect, fullscreen: bool) -> crate::PlayerRects {
    let mut stage = available;
    if !fullscreen {
        stage.max.y = (available.bottom() - PLAYER_TOOLBAR_HEIGHT).max(available.top());
    }
    let top = if fullscreen {
        (available.bottom() - PLAYER_TOOLBAR_HEIGHT).max(available.top())
    } else {
        stage.bottom()
    };
    let toolbar =
        egui::Rect::from_min_max(egui::pos2(available.left(), top), available.right_bottom());
    crate::PlayerRects { stage, toolbar }
}

/// Draws themed controls below an edge-to-edge black video canvas.
pub fn player_toolbar<R>(ui: &mut Ui, rect: egui::Rect, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::for_ui(ui);
    ui.painter().rect_filled(rect, 0, p.surface);
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, p.border),
    );
    let inset = if rect.width() < 750.0 { 14.0 } else { 22.0 };
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(inset, 12.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        contents,
    )
    .inner
}

/// Width of the v2 mute button, gap, and volume slider.
pub const PLAYER_VOLUME_CONTROL_WIDTH: f32 = 148.0;

/// Localized labels and availability for the watched stream's audio control.
pub struct VolumeLabels<'a> {
    /// Label when sound can be muted.
    pub mute: &'a str,
    /// Label when the previous volume can be restored.
    pub unmute: &'a str,
    /// Accessible slider name.
    pub volume: &'a str,
    /// Reason sound cannot be controlled.
    pub unavailable: &'a str,
    /// Whether this session has playable audio.
    pub enabled: bool,
}

/// Uses the shared session-scoped mute state with the v2 theme.
pub fn volume_control(
    ui: &mut Ui,
    state: &mut crate::PlayerVolumeState,
    labels: VolumeLabels<'_>,
) -> Option<u8> {
    let mut changed = None;
    ui.add_enabled_ui(labels.enabled, |ui| {
        let p = Palette::for_ui(ui);
        let label = if !labels.enabled {
            labels.unavailable
        } else if state.muted() {
            labels.unmute
        } else {
            labels.mute
        };
        let response = ui.add(
            egui::Button::new("")
                .min_size(egui::vec2(40.0, 40.0))
                .frame(true)
                .frame_when_inactive(false),
        );
        let interacting = response.enabled()
            && (response.hovered() || response.is_pointer_button_down_on() || response.has_focus());
        crate::components::player::paint_speaker_icon(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(20.0, 20.0)),
            state.muted(),
            if interacting { p.accent } else { p.muted },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                labels.enabled,
                state.muted(),
                label,
            )
        });
        if response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label)
            .on_disabled_hover_text(label)
            .clicked()
        {
            changed = Some(state.toggle_mute());
        }
        let mut percent = state.percent();
        ui.spacing_mut().slider_width = 100.0;
        ui.visuals_mut().selection.bg_fill = p.accent;
        let response = ui.add(
            egui::Slider::new(&mut percent, 0..=100)
                .show_value(false)
                .trailing_fill(true),
        );
        response.widget_info(|| {
            egui::WidgetInfo::slider(labels.enabled, percent as f64, labels.volume)
        });
        if response.changed() && state.set_percent(percent) {
            changed = Some(percent);
        }
        response
            .on_hover_text(format!("{}: {}%", labels.volume, percent))
            .on_disabled_hover_text(labels.unavailable);
    });
    changed
}

/// Reuses playback information measurement with a theme-aware button and popup.
pub fn player_info_control(
    ui: &mut Ui,
    state: &mut crate::PlayerInfoState,
    locale: crate::PlayerInfoLocale,
    now: f64,
    snapshot: crate::PlayerInfoSnapshot<'_>,
    enabled: bool,
) -> crate::PlayerInfoResponse {
    let button = ui
        .add_enabled_ui(enabled, |ui| {
            icon_button(
                ui,
                Icon::Information,
                match locale {
                    crate::PlayerInfoLocale::Chinese => "播放信息",
                    crate::PlayerInfoLocale::English => "Playback information",
                },
            )
        })
        .inner;
    crate::components::player_info::player_info_from_button(
        ui,
        state,
        locale,
        now,
        snapshot,
        button,
        Some(Palette::for_ui(ui)),
    )
}

/// A compact settings row with a stable label column and trailing controls.
pub fn settings_row<R>(ui: &mut Ui, label: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::for_ui(ui);
    let stacked = ui.available_width() < 360.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), if stacked { 84.0 } else { 56.0 }),
        egui::Sense::hover(),
    );
    let control = if stacked {
        egui::Rect::from_min_max(
            rect.min + egui::vec2(0.0, 32.0),
            rect.right_bottom() - egui::vec2(0.0, 12.0),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(rect.left() + 116.0, rect.top() + 8.0),
            rect.right_bottom() - egui::vec2(0.0, 8.0),
        )
    };
    let label_rect = if stacked {
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 24.0))
    } else {
        egui::Rect::from_min_max(rect.min, egui::pos2(control.left() - 12.0, rect.bottom()))
    };
    label_at(
        ui,
        label_rect,
        egui::Label::new(RichText::new(label).size(14.0).variation(b"wght", 500.0)).truncate(),
    );
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, p.border),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(control)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        contents,
    )
    .inner
}

/// A recoverable state shown in the video canvas instead of a page card.
pub struct CanvasMessage<'a> {
    /// Short description of the current state.
    pub title: &'a str,
    /// Failure details, scrollable when longer than the canvas allows.
    pub detail: &'a str,
    /// A platform-owned recovery action.
    pub action: &'a str,
}

/// Centers the whole state while keeping its recovery action outside the detail scroll area.
pub fn canvas_message(ui: &mut Ui, message: CanvasMessage<'_>) -> Response {
    let available = ui.available_rect_before_wrap();
    ui.painter().rect_filled(available, 0, Color32::BLACK);
    let rect = egui::Rect::from_center_size(
        available.center(),
        egui::vec2(
            (available.width() - 48.0).min(480.0),
            (available.height() - 32.0).min(260.0),
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.visuals_mut().dark_mode = true;
            let p = Palette::new(true);
            ui.label(
                RichText::new(message.title)
                    .size(18.0)
                    .variation(b"wght", 600.0)
                    .color(p.text),
            );
            ui.add_space(12.0);
            egui::ScrollArea::vertical()
                .id_salt("canvas-message-detail")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .max_height((rect.height() - 110.0).max(24.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(message.detail).size(13.0).color(p.muted))
                            .wrap(),
                    );
                });
            ui.add_space(18.0);
            Action {
                label: message.action,
                size: egui::vec2(180.0, 44.0),
                enabled: true,
                tone: ActionTone::Neutral,
            }
            .show(ui)
        },
    )
    .inner
}

#[cfg(test)]
mod responsive_settings_tests {
    use super::*;
    #[test]
    fn narrow_settings_stack_actions_below_the_label() {
        for width in [240.0, 359.0, 480.0] {
            egui::__run_test_ui(|ui| {
                ui.set_width(width);
                let top = ui.cursor().top();
                let mut buttons = Vec::new();
                settings_row(ui, "Logs", |ui| {
                    for label in ["Export logs", "Open folder"] {
                        buttons.push(
                            Action {
                                label,
                                size: egui::vec2(100.0, 40.0),
                                enabled: true,
                                tone: ActionTone::Neutral,
                            }
                            .show(ui)
                            .rect,
                        );
                    }
                });
                for rect in &buttons {
                    assert!(rect.left() >= ui.min_rect().left());
                    assert!(rect.right() <= ui.max_rect().right());
                    if width < 360.0 {
                        assert!(rect.top() >= top + 32.0);
                    } else {
                        assert!(rect.top() >= top + 8.0);
                    }
                }
                assert!(!buttons[0].intersects(buttons[1]));
            });
        }
    }
}

/// Labels and optional page title for the shared desktop header.
pub struct Header<'a> {
    /// None displays the home brand instead of a back action.
    pub title: Option<&'a str>,
    /// Accessible, localized back action.
    pub back: &'a str,
    /// Accessible, localized settings action.
    pub settings: &'a str,
}

/// Header actions for the platform to handle.
pub struct HeaderResponse {
    /// Navigate to Nearby without stopping media.
    pub back: bool,
    /// Open the settings panel.
    pub settings: bool,
}

/// Draws the common brand, navigation, page title, and settings entry.
pub fn header(ui: &mut Ui, spec: Header<'_>) -> HeaderResponse {
    let p = Palette::for_ui(ui);
    let row = ui.available_rect_before_wrap();
    let mut response = HeaderResponse {
        back: false,
        settings: false,
    };
    ui.horizontal_centered(|ui| {
        if spec.title.is_none() {
            brand_mark(ui);
            ui.label(
                RichText::new("MoQCast")
                    .size(16.0)
                    .variation(b"wght", 600.0)
                    .color(p.text),
            );
        } else {
            response.back = Action {
                label: spec.back,
                size: egui::vec2(112.0, 40.0),
                enabled: true,
                tone: ActionTone::Neutral,
            }
            .show(ui)
            .clicked();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            response.settings = icon_button(ui, Icon::Settings, spec.settings).clicked();
        });
    });
    if let Some(title) = spec.title {
        let center = egui::Rect::from_center_size(
            row.center(),
            egui::vec2((row.width() - 288.0).max(0.0), 40.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(center)
                .layout(egui::Layout::centered_and_justified(
                    egui::Direction::LeftToRight,
                )),
            |ui| {
                ui.add(egui::Label::new(RichText::new(title).size(15.0).color(p.muted)).truncate());
            },
        );
    }
    ui.painter().line_segment(
        [row.left_bottom(), row.right_bottom()],
        Stroke::new(1.0, p.border),
    );
    response
}

/// Scrolls only source rows with an auto-fading, draggable scrollbar.
pub fn source_list<R>(
    ui: &mut Ui,
    max_height: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.scope(|ui| {
        let scroll = &mut ui.style_mut().spacing.scroll;
        *scroll = egui::style::ScrollStyle::floating();
        // Reserve a stable gutter so the fading handle never covers source text.
        scroll.floating_allocated_width = 8.0;
        egui::ScrollArea::vertical()
            .id_salt("capture-source-list")
            .max_height(max_height)
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, contents)
    })
    .inner
}

/// A capture source row; the platform owns the selected source and its validation.
pub fn source_row(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::selectable(
            selected,
            RichText::new(label).size(14.0).variation(b"wght", 500.0),
        )
        .min_size(egui::vec2(ui.available_width(), 48.0))
        .truncate(),
    )
    .on_hover_text(label)
}

#[cfg(test)]
mod controls_contract_tests {
    use super::*;
    #[test]
    fn volume_width_matches_the_actual_controls() {
        egui::__run_test_ui(|ui| {
            install(ui.ctx());
            let mut state = crate::PlayerVolumeState::default();
            let output = ui.horizontal(|ui| {
                let start = ui.cursor().left();
                volume_control(
                    ui,
                    &mut state,
                    VolumeLabels {
                        mute: "Mute",
                        unmute: "Unmute",
                        volume: "Volume",
                        unavailable: "No audio",
                        enabled: true,
                    },
                );
                ui.min_rect().right() - start
            });
            assert!(
                (output.inner - PLAYER_VOLUME_CONTROL_WIDTH).abs() < 0.1,
                "{}",
                output.inner
            );
        });
    }
}
