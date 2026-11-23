//! Controls shared by the three native LAN watch surfaces.
use super::{Icon, Palette, toolbar_icon_button};
use crate::{
    PlayerInfoLocale, PlayerInfoSnapshot, PlayerInfoState, PlayerVolumeState, player_volume_control,
};
use egui::{Align, Color32, Layout, Rect, RichText, Stroke, Ui, UiBuilder, vec2};
use std::time::Duration;

/// Observable watch phase, including whether a frame is actually present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchPhase {
    /// No presentable frame yet.
    Waiting,
    /// An active session with a presentable video frame.
    Live,
    /// Stop is in progress.
    Stopping,
}

impl WatchPhase {
    fn allows_audio_control(self, audio: crate::PlayerAudioState) -> bool {
        self != Self::Stopping
            && matches!(
                audio,
                crate::PlayerAudioState::Ready | crate::PlayerAudioState::Muted
            )
    }
}

/// Media commands are dispatched by the platform that owns the session.
pub enum WatchCommand {
    /// Stop the watch session.
    Stop,
    /// Change local playback gain.
    Volume {
        /// The watch session that received this input.
        generation: u64,
        /// Linear gain in percent.
        percent: u8,
    },
}

const HIDE_AFTER: f64 = 2.5;

#[derive(Default)]
struct HideTimer {
    last_activity: Option<f64>,
    pinned: bool,
}

impl HideTimer {
    fn visible(&mut self, now: f64, activity: bool, pinned: bool) -> bool {
        if activity || pinned || self.pinned || self.last_activity.is_none() {
            self.last_activity = Some(now);
        }
        self.pinned = pinned;
        now - self.last_activity.unwrap_or(now) < HIDE_AFTER
    }
}

/// Interaction state scoped to a playback generation. Switching pages preserves volume.
#[derive(Default)]
pub struct WatchChrome {
    timer: HideTimer,
    info: PlayerInfoState,
    volume: PlayerVolumeState,
    controls_active: bool,
}

impl WatchChrome {
    /// Forget the previous fullscreen deadline when the native viewport leaves fullscreen.
    pub fn leave_fullscreen(&mut self) {
        self.timer = HideTimer::default();
        self.controls_active = false;
    }

    /// Draw the controls over an already painted stage and return only deliberate media actions.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        available: Rect,
        fullscreen: bool,
        phase: WatchPhase,
        locale: PlayerInfoLocale,
        mut snapshot: PlayerInfoSnapshot<'_>,
    ) -> Option<WatchCommand> {
        if self.volume.sync_generation(snapshot.generation) {
            self.leave_fullscreen();
        }
        self.info.sync_generation(snapshot.generation);
        let zh = matches!(locale, PlayerInfoLocale::Chinese);
        let text = |zh_label, en_label| if zh { zh_label } else { en_label };
        let (now, activity, escape, pointer) = ui.input(|i| {
            (
                i.time,
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { .. }
                            | egui::Event::PointerMoved(_)
                            | egui::Event::PointerButton { .. }
                            | egui::Event::MouseWheel { .. }
                            | egui::Event::Touch { .. }
                    )
                }),
                i.key_pressed(egui::Key::Escape),
                i.pointer.hover_pos(),
            )
        });
        let bar = super::player_rects(available, fullscreen).toolbar;
        let title = format!(
            "{} · {}",
            snapshot
                .source
                .unwrap_or(text("来源不可用", "Source unavailable")),
            text("按 Esc 退出全屏", "Press Esc to exit fullscreen")
        );
        let title_width = ui
            .painter()
            .layout_no_wrap(
                title.clone(),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            )
            .size()
            .x
            + 32.0;
        let top = Rect::from_center_size(
            egui::pos2(available.center().x, available.top() + 32.0),
            vec2(title_width.min((available.width() - 24.0).max(0.0)), 32.0),
        );
        if fullscreen {
            if escape {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            let pinned = phase != WatchPhase::Live
                || self.info.open()
                || self.controls_active
                || pointer.is_some_and(|p| bar.contains(p) || top.contains(p));
            if !self.timer.visible(now, activity, pinned) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                return None;
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            if !pinned {
                let remaining =
                    (HIDE_AFTER - (now - self.timer.last_activity.unwrap_or(now))).max(0.0);
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(remaining));
            }
            ui.painter().rect(
                top,
                14,
                Color32::from_rgb(21, 26, 29),
                Stroke::new(1.0, Color32::from_rgb(44, 53, 59)),
                egui::StrokeKind::Inside,
            );
            ui.put(
                top.shrink2(vec2(12.0, 0.0)),
                egui::Label::new(RichText::new(title).color(Color32::WHITE)).truncate(),
            );
        } else {
            self.leave_fullscreen();
        }
        let mut command = None;
        let enabled = phase != WatchPhase::Stopping;
        let compact = available.width() < 680.0;
        let mut active = false;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let mut palette = if fullscreen {
                    let mut dark = Palette::new(true);
                    dark.surface = Color32::from_rgb(27, 30, 32);
                    let visuals = ui.visuals_mut();
                    visuals.dark_mode = true;
                    visuals.override_text_color = Some(dark.text);
                    visuals.window_fill = dark.surface;
                    visuals.window_stroke = Stroke::new(1.0, dark.border);
                    dark
                } else {
                    Palette::for_ui(ui)
                };
                if fullscreen {
                    palette.surface = Color32::from_rgb(27, 30, 32);
                }
                ui.painter()
                    .rect_filled(bar, if fullscreen { 14 } else { 0 }, palette.surface);
                if fullscreen {
                    ui.painter().rect_stroke(
                        bar,
                        14,
                        Stroke::new(1.0, palette.border),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    ui.painter().line_segment(
                        [bar.left_top(), bar.right_top()],
                        Stroke::new(1.0, palette.border),
                    );
                }
                let btn_size = if fullscreen { 36.0 } else { 40.0 };
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(bar.shrink2(vec2(if fullscreen { 12.0 } else { 16.0 }, 0.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let label = match phase {
                            WatchPhase::Live => "LIVE",
                            WatchPhase::Waiting => text("等待画面", "Waiting for video"),
                            WatchPhase::Stopping => text("正在停止", "Stopping"),
                        };
                        if phase == WatchPhase::Live {
                            let (r, _) =
                                ui.allocate_exact_size(vec2(7.0, 7.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                r.center(),
                                3.5,
                                Color32::from_rgb(217, 45, 32),
                            );
                        }
                        let mut badge_text = RichText::new(label)
                            .size(if phase == WatchPhase::Live {
                                12.5
                            } else {
                                13.0
                            })
                            .color(if phase == WatchPhase::Live {
                                Color32::from_rgb(217, 45, 32)
                            } else {
                                palette.muted
                            });
                        if phase == WatchPhase::Live {
                            badge_text = badge_text.variation(b"wght", 600.0);
                        }
                        ui.label(badge_text);
                        divider(ui, palette);
                        let playable = phase.allows_audio_control(snapshot.audio_state);
                        let vol = player_volume_control(
                            ui,
                            &mut self.volume,
                            playable,
                            text("静音", "Mute"),
                            text("取消静音", "Unmute"),
                            text("播放音量", "Playback volume"),
                            text("当前没有可播放音频", "No playable audio"),
                        );
                        active |= vol.active();
                        if let Some(percent) = vol.changed_percent() {
                            command = Some(WatchCommand::Volume {
                                generation: snapshot.generation,
                                percent,
                            });
                        }
                        if playable {
                            snapshot.audio_state = if self.volume.muted() {
                                crate::PlayerAudioState::Muted
                            } else {
                                crate::PlayerAudioState::Ready
                            };
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let stop_label = if compact {
                                text("停止", "Stop")
                            } else {
                                text("停止观看", "Stop watching")
                            };
                            let stop = ui
                                .add_enabled_ui(enabled, |ui| {
                                    stop_button(ui, stop_label, palette, btn_size)
                                })
                                .inner;
                            active |= stop.has_focus();
                            if stop.clicked() {
                                command = Some(WatchCommand::Stop);
                            }
                            divider(ui, palette);
                            let fs = toolbar_icon_button(
                                ui,
                                if fullscreen {
                                    Icon::ExitFullscreen
                                } else {
                                    Icon::Fullscreen
                                },
                                if fullscreen {
                                    text("退出全屏", "Exit fullscreen")
                                } else {
                                    text("全屏", "Fullscreen")
                                },
                                btn_size,
                                palette,
                                false,
                            );
                            active |= fs.has_focus();
                            if fs.clicked() {
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                        !fullscreen,
                                    ));
                            }
                            let button = toolbar_icon_button(
                                ui,
                                Icon::Information,
                                text("播放信息", "Playback information"),
                                btn_size,
                                palette,
                                self.info.open(),
                            );
                            active |= crate::components::player_info::player_info_from_button(
                                ui,
                                &mut self.info,
                                locale,
                                now,
                                snapshot,
                                button,
                                Some(palette),
                            )
                            .active();
                        });
                    },
                );
            },
        );
        if fullscreen && active != self.controls_active {
            ui.ctx().request_repaint();
        }
        self.controls_active = active;
        command
    }
}

fn stop_button(ui: &mut Ui, label: &str, palette: Palette, height: f32) -> egui::Response {
    let painter = ui.painter();
    let color = if ui.is_enabled() {
        palette.danger
    } else {
        palette.muted
    };
    let text = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(14.0), color);
    let (rect, response) =
        ui.allocate_exact_size(vec2(text.size().x + 41.0, height), egui::Sense::click());
    let stroke = if response.has_focus() { 2.0 } else { 1.0 };
    ui.painter().rect(
        rect,
        7,
        if response.hovered() {
            palette.danger_soft
        } else {
            palette.surface
        },
        Stroke::new(stroke, color),
        egui::StrokeKind::Inside,
    );
    let square = Rect::from_center_size(
        egui::pos2(rect.left() + 16.5, rect.center().y),
        vec2(9.0, 9.0),
    );
    ui.painter().rect_filled(square, 2, color);
    ui.painter().galley(
        egui::pos2(square.right() + 8.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn divider(ui: &mut Ui, palette: Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0, palette.border);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_controls_follow_audio_before_the_first_video_frame() {
        use crate::PlayerAudioState::*;
        for playable in [Ready, Muted] {
            assert!(WatchPhase::Waiting.allows_audio_control(playable));
            assert!(!WatchPhase::Stopping.allows_audio_control(playable));
        }
        for unavailable in [Pending, NoAudio, Failed] {
            assert!(!WatchPhase::Waiting.allows_audio_control(unavailable));
        }
    }

    #[test]
    fn fullscreen_idle_pins_and_wakeup_use_a_fresh_deadline() {
        let mut timer = HideTimer::default();
        assert!(timer.visible(3.0, false, false));
        assert!(timer.visible(5.49, false, false));
        assert!(!timer.visible(5.5, false, false));
        assert!(timer.visible(6.0, true, false));
        assert!(timer.visible(20.0, false, true));
        assert!(timer.visible(40.0, false, true));
        assert!(timer.visible(41.0, false, false));
        assert!(!timer.visible(43.5, false, false));
    }

    #[test]
    fn fullscreen_capsule_controls_stay_frameless_and_equal_height() {
        let ctx = egui::Context::default();
        super::super::install(&ctx);
        let mut chrome = WatchChrome::default();
        let snapshot = PlayerInfoSnapshot {
            generation: 1,
            source: Some("Studio Mac"),
            frame: None,
            resolution: Some((1920, 1080)),
            video_codec: Some("avc1"),
            decoder: Some("videotoolbox"),
            requested_policy: Some("auto"),
            execution: None,
            awaiting_backend: false,
            presentation: None,
            hardware_refused: false,
            audio_state: crate::PlayerAudioState::Ready,
            audio_codec: Some("opus"),
            audio_sample_rate: Some(48_000),
            audio_channels: Some(2),
        };
        let available = Rect::from_min_size(egui::Pos2::ZERO, vec2(1040.0, 700.0));
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(available),
                time: Some(1.0),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(root, |ui| {
                        let _ = chrome.show(
                            ui,
                            available,
                            true,
                            WatchPhase::Live,
                            PlayerInfoLocale::English,
                            snapshot,
                        );
                    });
            },
        );
        // Fullscreen icon buttons must not render egui's default light-grey filled squares (#3c3c3c).
        let grey_boxes = out
            .shapes
            .iter()
            .filter(
                |s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == Color32::from_gray(60)),
            )
            .count();
        assert_eq!(
            grey_boxes, 0,
            "fullscreen capsule must not paint raw grey button fills"
        );
        let stop_rect = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if (r.rect.height() - 36.0).abs() < 0.5 && r.rect.width() > 90.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("fullscreen stop button should be 36pt tall");
        assert!((stop_rect.height() - 36.0).abs() < 0.5);
        out.textures_delta.clear();
    }
}
