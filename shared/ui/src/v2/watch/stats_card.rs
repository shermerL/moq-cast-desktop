//! The card owns interaction state. Its one-second repaint deadline only reads the session sampler.
use crate::{PlayerInfoLocale, PlayerInfoSnapshot, v2::Palette};
use egui::{
    Align, Align2, Area, Button, Color32, CursorIcon, FontId, Id, Layout, Order, Pos2, Rect,
    Response, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};
use moqcast_playback_stats::{MarkKind, PlaybackStats, Sample, StatsSnapshot};
use std::time::Duration;

mod plot;

#[derive(Default)]
pub(super) struct StatsCard {
    pub(super) open: bool,
    offset: Option<Vec2>,
    grab_offset: Option<Vec2>,
    hidden_decoded: bool,
    hidden_ui: bool,
    selected: Option<f64>,
    cached: Option<(f64, StatsSnapshot)>,
}

#[derive(Clone, Copy)]
struct Labels(PlayerInfoLocale);

impl Labels {
    fn get(self, zh: &'static str, en: &'static str) -> &'static str {
        match self.0 {
            PlayerInfoLocale::Chinese => zh,
            PlayerInfoLocale::English => en,
        }
    }
}

fn card_rect(stage: Rect, offset: Option<Vec2>) -> Rect {
    let narrow = stage.width() <= 680.0;
    let bounds = stage.shrink(if narrow { 10.0 } else { 14.0 });
    let size = bounds
        .size()
        .max(Vec2::ZERO)
        .min(vec2(if narrow { 396.0 } else { 418.0 }, 444.0));
    let travel = (bounds.size() - size).max(Vec2::ZERO);
    let position = offset
        .unwrap_or(vec2(travel.x, 0.0))
        .clamp(Vec2::ZERO, travel);
    Rect::from_min_size(bounds.min + position, size)
}

struct CardHeader<'a> {
    owner: &'a PlaybackStats,
    data: &'a StatsSnapshot,
    t: Labels,
    p: Palette,
}

impl StatsCard {
    pub(super) fn close(&mut self) {
        self.open = false;
        self.grab_offset = None;
        self.selected = None;
        self.cached = None;
    }

    pub(super) fn show(
        &mut self,
        ui: &Ui,
        stage: Rect,
        info: PlayerInfoSnapshot<'_>,
        locale: PlayerInfoLocale,
        fullscreen: bool,
    ) {
        let Some(owner) = info.stats else {
            self.close();
            return;
        };
        let now = ui.input(|i| i.time);
        let mut cached = self.cached.take();
        if cached.as_ref().is_none_or(|(read, _)| now - read >= 1.0) {
            cached = owner
                .snapshot(info.generation)
                .map(|snapshot| (now, snapshot));
        }
        let Some((read, data)) = &cached else {
            self.close();
            return;
        };
        let rect = card_rect(stage, self.offset);
        let p = Palette::new(fullscreen || ui.visuals().dark_mode);
        let t = Labels(locale);
        let mut refresh = false;
        Area::new(Id::new(("watch-statistics", info.generation)))
            .order(Order::Middle)
            .movable(false)
            .sense(Sense::click())
            .fixed_pos(rect.min)
            .constrain(false)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                let rect = Rect::from_min_size(ui.min_rect().min, rect.size());
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                ui.set_clip_rect(rect.intersect(stage));
                ui.painter().rect(
                    rect,
                    12,
                    p.surface,
                    Stroke::new(1.0, p.border),
                    StrokeKind::Inside,
                );
                if fullscreen {
                    *ui.visuals_mut() = ui.ctx().style_of(egui::Theme::Dark).visuals.clone();
                }
                ui.visuals_mut().override_text_color = Some(p.text);
                ui.visuals_mut().weak_text_color = Some(p.muted);
                // The shell uses 16×10 pt button padding. The compact card has its own rows.
                ui.spacing_mut().interact_size.y = 22.0;
                ui.spacing_mut().button_padding = vec2(6.0, 3.0);
                ui.spacing_mut().item_spacing = vec2(5.0, 4.0);
                let inner = rect.shrink(11.0);
                let head = Rect::from_min_size(inner.min, vec2(inner.width(), 32.0));
                refresh = self.header(ui, head, stage, CardHeader { owner, data, t, p });
                let facts = Rect::from_min_size(
                    pos2(inner.left(), head.bottom() + 5.0),
                    vec2(inner.width(), 28.0),
                );
                paint_facts(ui, facts, info, data, t, p);
                let time_strip = Rect::from_min_size(
                    pos2(inner.left(), facts.bottom() + 2.0),
                    vec2(inner.width(), 24.0),
                );
                let footer =
                    Rect::from_min_max(pos2(inner.left(), inner.bottom() - 25.0), inner.max);
                let body = Rect::from_min_max(time_strip.left_bottom(), footer.right_top());
                let cursor = self.charts(ui, body, data, t, p);
                cursor_readout(ui, time_strip, data, cursor, t, p);
                self.events(ui, footer, data, t, p);
            });
        if self.open {
            if refresh {
                ui.ctx().request_repaint();
            } else {
                if data.enabled {
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(
                        (1.0 - (now - *read)).max(0.0),
                    ));
                }
                self.cached = cached;
            }
        }
    }
    fn header(&mut self, ui: &mut Ui, head: Rect, stage: Rect, header: CardHeader<'_>) -> bool {
        let CardHeader { owner, data, t, p } = header;
        let mut refresh = false;
        let mut header = ui.new_child(
            UiBuilder::new()
                .max_rect(head)
                .layout(Layout::right_to_left(Align::Center)),
        );
        header.spacing_mut().item_spacing.x = 5.0;
        let close = header
            .add(Button::new("×").frame(false).min_size(vec2(24.0, 24.0)))
            .on_hover_text(t.get("收起播放统计", "Close playback statistics"));
        if close.clicked() {
            self.close();
        }
        header
            .add_enabled(false, Button::new(t.get("导出", "Export")))
            .on_disabled_hover_text(t.get("统计导出尚未接入", "Statistics export is unavailable"));
        if header
            .add_enabled(data.enabled, Button::new(t.get("标记", "Mark")))
            .on_hover_text(t.get("标记此刻", "Mark this moment"))
            .clicked()
        {
            owner.mark(data.generation);
            refresh = true;
        }
        if header
            .button(if data.enabled {
                t.get("停止采样", "Stop sampling")
            } else {
                t.get("开始采样", "Start sampling")
            })
            .clicked()
        {
            owner.set_sampling(data.generation, !data.enabled);
            refresh = true;
        }
        let handle = header.available_rect_before_wrap();
        let drag = header
            .interact(handle, header.id().with("drag-title"), Sense::drag())
            .on_hover_cursor(CursorIcon::Grab);
        let grip = pos2(handle.left() + 4.0, handle.center().y);
        for dx in [0.0, 4.0] {
            for dy in [-4.0, 0.0, 4.0] {
                header
                    .painter()
                    .circle_filled(grip + vec2(dx, dy), 1.0, p.muted);
            }
        }
        header.painter().with_clip_rect(handle).text(
            grip + vec2(13.0, 0.0),
            Align2::LEFT_CENTER,
            t.get("播放统计", "Playback statistics"),
            FontId::proportional(13.0),
            p.text,
        );
        self.drag_title(ui, &drag, stage);

        refresh
    }

    fn drag_title(&mut self, ui: &Ui, handle: &Response, stage: Rect) {
        use egui::{Event, PointerButton::Primary};
        if !ui.input(|i| i.focused) {
            self.grab_offset = None;
            return;
        }
        // A complete gesture can arrive in one input batch, with neither a
        // dragged Response nor a retained press_origin. Hit-test the press, then
        // apply the absolute pointer position through the matching release.
        let events = ui.input(|i| {
            if self.grab_offset.is_none() && !i.pointer.primary_pressed() {
                return Vec::new();
            }
            i.events
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        Event::PointerMoved(_)
                            | Event::PointerButton {
                                button: Primary,
                                ..
                            }
                            | Event::WindowFocused(false)
                    )
                })
                .cloned()
                .collect()
        });
        let mut destination = None;
        for event in events {
            match event {
                Event::WindowFocused(false) => self.grab_offset = None,
                Event::PointerButton {
                    pressed: true, pos, ..
                } => {
                    let owns_press = handle.enabled()
                        && handle.interact_rect.contains(pos)
                        && ui.ctx().layer_id_at(pos) == Some(handle.layer_id)
                        && ui.memory(|m| m.allows_interaction(handle.layer_id));
                    self.grab_offset = owns_press.then(|| pos - card_rect(stage, self.offset).min);
                }
                Event::PointerMoved(pos) => {
                    if let Some(grab) = self.grab_offset {
                        destination = Some(pos - grab);
                    }
                }
                Event::PointerButton {
                    pressed: false,
                    pos,
                    ..
                } => {
                    if let Some(grab) = self.grab_offset.take() {
                        destination = Some(pos - grab);
                    }
                }
                _ => {}
            }
        }
        if let Some(pos) = destination {
            let origin = card_rect(stage, Some(Vec2::ZERO)).min;
            let next = card_rect(stage, Some(pos - origin)).min;
            if next != card_rect(stage, self.offset).min {
                self.offset = Some(next - origin);
                ui.ctx().request_repaint();
            }
        }
        if self.grab_offset.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
    }

    fn charts(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        data: &StatsSnapshot,
        t: Labels,
        p: Palette,
    ) -> Option<f64> {
        let mut scroller = ui.new_child(UiBuilder::new().max_rect(body));
        scroller.set_clip_rect(body.intersect(ui.clip_rect()));
        let colors = plot::Colors::new(ui.visuals().dark_mode);
        let mut cursor = None;
        ScrollArea::vertical()
            .id_salt("chart-body")
            .auto_shrink([false, false])
            .max_height(body.height())
            .show(&mut scroller, |ui| {
                let (blocks, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 300.0), Sense::hover());
                let frame_plot = plot_rect(blocks, 50.0, 151.0);
                let payload_plot = plot_rect(blocks, 212.0, 281.0);
                for (key, plot) in [("frames", frame_plot), ("payload", payload_plot)] {
                    let response = ui
                        .interact(plot, ui.id().with(key), Sense::click())
                        .on_hover_cursor(CursorIcon::Crosshair);
                    if ui.input(|i| i.focused)
                        && response.hovered()
                        && let Some(pos) = response.hover_pos()
                    {
                        let at =
                            data.now - 60.0 + 60.0 * ((pos.x - plot.left()) / plot.width()) as f64;
                        cursor = Some(at);
                        if response.clicked() {
                            self.selected = cursor;
                        }
                    }
                }
                self.selected = self.selected.filter(|at| *at >= data.now - 60.0);
                cursor = cursor.or(self.selected);
                let sample = match cursor {
                    Some(at) => sample_at(&data.samples, at),
                    None => data.samples.last(),
                };
                let fps_header = blocks.min + vec2(0.0, 6.0);
                label(
                    ui,
                    fps_header,
                    t.get("视频帧率", "Video frame rate"),
                    p.text,
                    13.0,
                );
                ui.painter().text(
                    pos2(blocks.right(), fps_header.y),
                    Align2::RIGHT_TOP,
                    t.get("帧/秒 · 近 60 秒", "fps · Last 60 s"),
                    FontId::proportional(11.0),
                    p.muted,
                );
                let legend_rect =
                    Rect::from_min_size(blocks.min + vec2(0.0, 24.0), vec2(blocks.width(), 24.0));
                let mut legend_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(legend_rect)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                legend_ui.spacing_mut().item_spacing.x = 5.0;
                series_legend(
                    &mut legend_ui,
                    t,
                    sample.and_then(|s| s.decoded_fps),
                    &mut self.hidden_decoded,
                    colors.decoded,
                    true,
                    p,
                );
                series_legend(
                    &mut legend_ui,
                    t,
                    sample.and_then(|s| s.ui_fps),
                    &mut self.hidden_ui,
                    colors.ui,
                    false,
                    p,
                );
                legend_ui.label(
                    RichText::new(t.get("SDK 解码输入 · 未接入", "Decoder input · Unavailable"))
                        .size(10.5)
                        .color(p.muted),
                );
                for r in [frame_plot, payload_plot] {
                    plot::background(ui.painter(), r, data, p, t);
                }
                plot::frame_rates(
                    ui.painter(),
                    frame_plot,
                    data,
                    !self.hidden_decoded,
                    !self.hidden_ui,
                    colors,
                    p,
                );
                label(
                    ui,
                    blocks.min + vec2(0.0, 165.0),
                    t.get("媒体负载码率", "Media payload bitrate"),
                    p.text,
                    13.0,
                );
                ui.painter().text(
                    pos2(blocks.right(), blocks.top() + 165.0),
                    Align2::RIGHT_TOP,
                    "Mbit/s",
                    FontId::proportional(11.0),
                    p.muted,
                );
                label(
                    ui,
                    blocks.min + vec2(0.0, 188.0),
                    t.get(
                        "视频负载 · 未提供     音频负载 · 未提供",
                        "Video · Unavailable      Audio · Unavailable",
                    ),
                    p.muted,
                    11.0,
                );
                ui.painter().text(
                    payload_plot.center(),
                    Align2::CENTER_CENTER,
                    t.get("有效载荷码率暂未提供", "Payload bitrate unavailable"),
                    FontId::proportional(12.0),
                    p.muted,
                );
                plot::time_labels(ui.painter(), payload_plot, p);
                for r in [frame_plot, payload_plot] {
                    plot::markers(ui.painter(), r, data, cursor, colors, p);
                }
            });

        cursor
    }

    fn events(&mut self, ui: &mut Ui, footer: Rect, data: &StatsSnapshot, t: Labels, p: Palette) {
        ui.painter().line_segment(
            [footer.left_top(), footer.right_top()],
            Stroke::new(1.0, p.border),
        );
        let mut events = ui.new_child(
            UiBuilder::new()
                .max_rect(footer)
                .layout(Layout::right_to_left(Align::Center)),
        );
        events.label(
            RichText::new(format!("{} {}", t.get("丢样", "Missed"), data.missed))
                .size(11.0)
                .color(p.muted),
        );
        let mut strip = events.new_child(
            UiBuilder::new()
                .max_rect(events.available_rect_before_wrap())
                .layout(Layout::left_to_right(Align::Center)),
        );
        ScrollArea::horizontal()
            .id_salt("watch-events")
            .auto_shrink([false, true])
            .show(&mut strip, |ui| {
                ui.horizontal(|ui| {
                    for mark in data.marks.iter().rev() {
                        let kind = match mark.kind {
                            MarkKind::DecoderChanged => t.get("重建", "Rebuilt"),
                            MarkKind::User => t.get("标记", "Mark"),
                        };
                        if ui
                            .add(
                                Button::selectable(
                                    self.selected == Some(mark.at),
                                    format!("{kind} −{:.0}s", data.now - mark.at),
                                )
                                .small(),
                            )
                            .clicked()
                        {
                            self.selected = Some(mark.at);
                            ui.ctx().request_repaint();
                        }
                    }
                });
            });
    }
}
fn paint_facts(
    ui: &mut Ui,
    facts: Rect,
    info: PlayerInfoSnapshot<'_>,
    data: &StatsSnapshot,
    t: Labels,
    p: Palette,
) {
    ui.painter().rect_filled(facts, 5, p.canvas);
    let missing = t.get("未知", "Unknown");
    let res = info
        .resolution
        .map(|(w, h)| format!("{w}×{h}"))
        .unwrap_or_else(|| missing.to_owned());
    let rebuilt = data
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::DecoderChanged);
    let decoder = info.decoder.unwrap_or(missing);
    let mut facts_text = format!(
        "{res} · {} · {decoder}",
        info.video_codec.unwrap_or(missing)
    );
    if rebuilt && info.decoder.is_some() {
        facts_text.push_str(t.get(" (已重建)", " (rebuilt)"));
    }
    if let Some(codec) = info.audio_codec {
        facts_text.push_str(&format!(" · {codec}"));
        if let Some(rate) = info.audio_sample_rate {
            facts_text.push_str(&format!(" {} kHz", rate as f64 / 1000.0));
        }
        if let Some(channels) = info.audio_channels {
            facts_text.push_str(&format!(" · {channels}ch"));
        }
    } else {
        facts_text.push_str(" · ");
        facts_text.push_str(crate::components::player_info::audio_state_label(
            t.0,
            info.audio_state,
        ));
    }
    ui.put(
        facts.shrink2(vec2(6.0, 0.0)),
        egui::Label::new(RichText::new(&facts_text).size(11.5).color(p.text)).truncate(),
    )
    .on_hover_text(&facts_text);
}
fn cursor_readout(
    ui: &mut Ui,
    time_strip: Rect,
    data: &StatsSnapshot,
    cursor: Option<f64>,
    t: Labels,
    p: Palette,
) {
    let at = cursor
        .or_else(|| data.samples.last().map(|s| s.at))
        .unwrap_or(data.now);
    let offset = (data.now - at).max(0.0);
    let sample = sample_at(&data.samples, at);
    let mut time_text = format!("{}  T−{offset:.1}s", t.get("时点", "Time"));
    if at >= 0.0 {
        time_text.push_str(&format!(
            "  ·  {:02}:{:04.1}",
            (at / 60.0) as u64,
            at % 60.0
        ));
    }
    if let Some(s) = sample {
        time_text.push_str(&format!("  ·  {:.1}s", s.at - s.start));
    }
    label(
        ui,
        time_strip.min + vec2(2.0, 5.0),
        &time_text,
        p.accent,
        11.5,
    );
    let status = if !data.enabled {
        t.get("采样已关闭", "Sampling off")
    } else if let Some(mark) = data.marks.iter().find(|m| Some(m.at) == cursor) {
        match mark.kind {
            MarkKind::User => t.get("标记", "Mark"),
            MarkKind::DecoderChanged => t.get("解码器重建", "Decoder rebuilt"),
        }
    } else if sample.is_none_or(|s| s.decoded_fps.is_none() && s.ui_fps.is_none()) {
        t.get("缺测", "Missing")
    } else {
        t.get("实测", "Measured")
    };
    ui.painter().text(
        time_strip.right_center(),
        Align2::RIGHT_CENTER,
        status,
        FontId::proportional(11.5),
        p.muted,
    );
}

fn plot_rect(blocks: Rect, top: f32, bottom: f32) -> Rect {
    Rect::from_min_max(
        pos2(blocks.left() + 24.0, blocks.top() + top),
        pos2(blocks.right() - 8.0, blocks.top() + bottom),
    )
}

fn sample_at(samples: &[Sample], at: f64) -> Option<&Sample> {
    samples.iter().find(|s| s.start < at && at <= s.at)
}

fn label(ui: &Ui, at: Pos2, text: &str, color: Color32, size: f32) {
    ui.painter().text(
        at,
        Align2::LEFT_TOP,
        text,
        FontId::proportional(size),
        color,
    );
}

fn series_legend(
    ui: &mut Ui,
    t: Labels,
    value: Option<f64>,
    hidden: &mut bool,
    color: Color32,
    decoded: bool,
    p: Palette,
) {
    let (name, tooltip) = if decoded {
        (
            t.get("解码输出", "Decoded"),
            t.get(
                "解码成功返回的帧数，计于调度丢帧前",
                "Frames returned by the decoder before scheduling",
            ),
        )
    } else {
        (
            t.get("UI 新接收", "UI received"),
            t.get(
                "UI 首次接收的唯一帧数，同一帧重复重绘只计一次",
                "Unique new frames accepted by the UI",
            ),
        )
    };
    let value = value
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| t.get("缺测", "Missing").into());
    let text = format!("{name} {value}");
    let galley = ui.painter().layout_no_wrap(
        text.clone(),
        FontId::proportional(11.0),
        if *hidden { p.muted } else { p.text },
    );
    let (r, response) = ui.allocate_exact_size(vec2(galley.size().x + 21.0, 22.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, !*hidden, name)
    });
    if response.clicked() {
        *hidden = !*hidden;
    }
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(r, 4, p.hover);
    }
    let swatch = [
        pos2(r.left() + 2.0, r.center().y),
        pos2(r.left() + 15.0, r.center().y),
    ];
    let stroke = Stroke::new(2.0, if *hidden { p.muted } else { color });
    if decoded {
        ui.painter()
            .extend(egui::Shape::dashed_line(&swatch, stroke, 3.0, 2.0));
    } else {
        ui.painter().line_segment(swatch, stroke);
    }
    ui.painter().galley(
        pos2(r.left() + 19.0, r.center().y - galley.size().y * 0.5),
        galley,
        p.text,
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tooltip);
}

pub(super) fn toolbar_button(
    ui: &mut Ui,
    label: &str,
    size: f32,
    p: Palette,
    open: bool,
    fullscreen: bool,
) -> Response {
    let with_text = !fullscreen && ui.ctx().content_rect().width() > 760.0;
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(12.5), p.text);
    let width = if with_text {
        size + galley.size().x + 4.0
    } else {
        size
    };
    let (r, response) = ui.allocate_exact_size(vec2(width, size), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, label));
    if open || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(r, 7, if open { p.soft } else { p.hover });
    }
    let c = pos2(r.left() + size / 2.0, r.center().y);
    let color = if open { p.accent } else { p.text };
    let stroke = Stroke::new(1.6, color);
    ui.painter().line(
        vec![
            c + vec2(-7.0, -7.0),
            c + vec2(-7.0, 7.0),
            c + vec2(8.0, 7.0),
        ],
        stroke,
    );
    ui.painter().line(
        vec![
            c + vec2(-4.0, 3.0),
            c + vec2(0.0, -2.0),
            c + vec2(3.0, 0.0),
            c + vec2(7.0, -5.0),
        ],
        stroke,
    );
    if with_text {
        ui.painter().galley(
            pos2(r.left() + size - 3.0, c.y - galley.size().y * 0.5),
            galley,
            p.text,
        );
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests;
