use super::*;
use crate::{
    PlayerAudioState,
    v2::{WatchChrome, WatchPhase},
};
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, ViewportCommand};
use moqcast_playback_stats::{PlaybackStats, WatchSession};

struct Player {
    ctx: Context,
    chrome: WatchChrome,
    stats: PlaybackStats,
    session: WatchSession,
    time: f64,
    screen: Rect,
    fullscreen: bool,
    focused: bool,
    locale: PlayerInfoLocale,
    stopped: bool,
    cover: Option<Rect>,
}

impl Player {
    fn new(size: Vec2) -> Self {
        let ctx = Context::default();
        crate::v2::install(&ctx);
        let stats = PlaybackStats::default();
        let session = stats.begin(7);
        Self {
            ctx,
            chrome: Default::default(),
            stats,
            session,
            time: 0.0,
            screen: Rect::from_min_size(Pos2::ZERO, size),
            fullscreen: false,
            focused: true,
            locale: PlayerInfoLocale::English,
            stopped: false,
            cover: None,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        self.time += 0.01;
        let info = PlayerInfoSnapshot {
            stats: Some(&self.stats),
            requested_policy: Some("auto"),
            execution: Some("hardware"),
            hardware_refused: false,
            awaiting_backend: false,
            presentation: Some("wgpu"),
            generation: self.session.generation(),
            source: Some("Camera"),
            frame: None,
            resolution: Some((1920, 1080)),
            video_codec: Some("H.264"),
            decoder: Some("VideoToolbox"),
            audio_state: PlayerAudioState::Ready,
            audio_codec: Some("Opus"),
            audio_sample_rate: Some(48000),
            audio_channels: Some(2),
        };
        let stage = Rect::from_min_max(
            pos2(0.0, if self.fullscreen { 0.0 } else { 62.0 }),
            self.screen.max,
        );
        let mut output = self.ctx.run_ui(
            RawInput {
                screen_rect: Some(self.screen),
                time: Some(self.time),
                events,
                focused: self.focused,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(root, |ui| {
                        let command = self.chrome.show(
                            ui,
                            stage,
                            self.fullscreen,
                            WatchPhase::Live,
                            self.locale,
                            info,
                        );
                        self.stopped |= matches!(command, Some(crate::v2::WatchCommand::Stop));
                    });
                if let Some(cover) = self.cover {
                    egui::Area::new(Id::new("foreground-controls"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(cover.min)
                        .show(&self.ctx, |ui| {
                            ui.allocate_exact_size(cover.size(), Sense::click_and_drag());
                        });
                }
            },
        );
        // The headless player has no renderer to apply texture updates.
        output.textures_delta.clear();
        output
    }
    fn click(&mut self, pos: Pos2) -> FullOutput {
        self.frame(vec![Event::PointerMoved(pos), press(pos, true)]);
        self.frame(vec![press(pos, false)]);
        self.frame(vec![])
    }
    fn open(&mut self) -> FullOutput {
        let out = self.frame(vec![]);
        let toggle = text_rect(&out, "Playback statistics").unwrap();
        self.click(toggle.center());
        assert!(self.chrome.stats.open);
        self.frame(vec![])
    }
    fn escape(&mut self) -> FullOutput {
        self.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }])
    }
}

fn text_rect(out: &FullOutput, label: &str) -> Option<Rect> {
    out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == label => {
            Some(t.visual_bounding_rect().intersect(s.clip_rect))
        }
        _ => None,
    })
}
fn icon_center(ctx: &Context, output: &FullOutput, name: &str) -> Pos2 {
    let texture_id = ctx
        .data(|d| d.get_temp::<egui::TextureHandle>(Id::new(name)))
        .unwrap()
        .id();
    output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture_id => {
                Some(mesh.calc_bounds().center())
            }
            _ => None,
        })
        .unwrap()
}

fn press(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn card_modes_events_and_narrow_window_coverage() {
    let mut player = Player::new(vec2(1040.0, 700.0));
    let open = player.open();
    let y_axis_label = text_rect(&open, "−60s").expect("both charts fit the normal window");
    assert!(y_axis_label.is_positive());
    assert!(!player.stats.is_sampling(7));
    let start = text_rect(&open, "Start sampling").unwrap().center();
    let sampling = player.click(start);
    assert!(player.stats.is_sampling(7));
    let mark = text_rect(&sampling, "Mark").unwrap().center();
    let marked = player.click(mark);
    assert_eq!(player.stats.snapshot(7).unwrap().marks.len(), 1);
    let x = text_rect(&marked, "×").unwrap().center();
    player.click(x);
    assert!(!player.chrome.stats.open);
    assert!(player.stats.is_sampling(7));
    let open = player.open();
    player.click(text_rect(&open, "Stop sampling").unwrap().center());
    assert!(!player.stats.is_sampling(7));
    player.screen = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 520.0));
    player.frame(vec![]);
    let narrow = player.frame(vec![]);
    let stage = Rect::from_min_max(pos2(0.0, 62.0), pos2(680.0, 468.0));
    let bounds = card_rect(stage, None);
    assert_eq!(bounds.width(), 396.0);
    let close = text_rect(&narrow, "×").unwrap();
    assert!(bounds.contains_rect(close));
    let body = text_rect(&narrow, "Video frame rate").unwrap().center() + vec2(20.0, 35.0);
    player.frame(vec![
        Event::PointerMoved(body),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: vec2(0.0, -240.0),
            modifiers: Modifiers::NONE,
        },
    ]);
    let mut after = player.frame(vec![]);
    for _ in 0..30 {
        after = player.frame(vec![]);
    }
    assert!(text_rect(&after, "−60s").is_some_and(|r| r.is_positive()));
    assert_eq!(text_rect(&after, "×"), Some(close));
    let mute = icon_center(&player.ctx, &after, "v2-icon-volume-high");
    player.click(mute);
    assert!(player.chrome.volume.muted());
    let info = icon_center(&player.ctx, &after, "v2-icon-info-circle");
    player.click(info);
    assert!(player.chrome.info.open() && player.chrome.stats.open);
    player.escape();
    let fs = icon_center(&player.ctx, &after, "v2-icon-maximize");
    player.frame(vec![Event::PointerMoved(fs), press(fs, true)]);
    let fullscreen = player.frame(vec![press(fs, false)]);
    assert!(
        fullscreen.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, ViewportCommand::Fullscreen(true)))
    );
    player.click(close.center());
    assert!(!player.chrome.stats.open);
    player.click(info - vec2(48.0, 0.0));
    assert!(player.chrome.stats.open);
    player.click(text_rect(&after, "Stop watching").unwrap().center());
    assert!(player.stopped);
}

#[test]
fn drag_capture_and_legend_selection_have_separate_hit_targets() {
    let mut p = Player::new(vec2(1040.0, 700.0));
    let open = p.open();
    let legend = text_rect(&open, "Decoded Missing").unwrap().center();
    p.click(legend);
    assert!(p.chrome.stats.hidden_decoded);
    assert!(!p.chrome.stats.hidden_ui);
    assert_eq!(p.chrome.stats.offset, None);
    let grip = pos2(650.0, 100.0);
    p.frame(vec![Event::PointerMoved(grip), press(grip, true)]);
    p.frame(vec![Event::PointerMoved(grip + vec2(-15.0, 8.0))]);
    assert!(p.chrome.stats.grab_offset.is_some());
    p.frame(vec![Event::PointerMoved(pos2(-500.0, 1500.0))]);
    let at_corner = p.chrome.stats.offset.unwrap();
    assert_eq!(at_corner.x, 0.0);
    assert!(at_corner.y > 0.0);
    p.focused = false;
    p.frame(vec![Event::WindowFocused(false)]);
    assert!(p.chrome.stats.grab_offset.is_none());
    p.focused = true;
    p.frame(vec![Event::WindowFocused(true), Event::PointerMoved(grip)]);
    assert_eq!(p.chrome.stats.offset, Some(at_corner));
    assert!(p.chrome.stats.open);
}

fn painted_card(out: &FullOutput) -> Rect {
    out.shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.rect.size() == vec2(418.0, 444.0) && r.fill.a() == 255 => {
                Some(r.rect)
            }
            _ => None,
        })
        .expect("the opaque statistics card is painted")
}

#[test]
fn title_drag_paints_the_release_position_across_batched_and_separate_frames() {
    for batch in [false, true] {
        let mut player = Player::new(vec2(1040.0, 700.0));
        let initial = painted_card(&player.open());
        let from = initial.min + vec2(45.0, 27.0);
        let delta = vec2(-320.0, 60.0);
        let to = from + delta;
        player.frame(vec![Event::PointerMoved(from)]);
        if batch {
            player.frame(vec![
                press(from, true),
                Event::PointerMoved(to),
                press(to, false),
            ]);
        } else {
            let middle = from + vec2(-1.0, 1.0);
            player.frame(vec![press(from, true)]);
            player.frame(vec![Event::PointerMoved(middle)]);
            assert_eq!(
                painted_card(&player.frame(vec![])),
                initial.translate(middle - from)
            );
            player.frame(vec![Event::PointerMoved(to), press(to, false)]);
        }
        let settled = painted_card(&player.frame(vec![]));
        assert_eq!(settled, initial.translate(delta), "batched={batch}");
        assert!(player.chrome.stats.grab_offset.is_none());
        assert!(!player.stopped);
    }
}

#[test]
fn a_popup_covering_the_title_owns_the_batched_press_and_focus_loss_cancels_capture() {
    let mut player = Player::new(vec2(1040.0, 700.0));
    let initial = painted_card(&player.open());
    let from = initial.min + vec2(45.0, 27.0);
    let to = from + vec2(-260.0, 50.0);
    player.cover = Some(Rect::from_center_size(from, vec2(70.0, 28.0)));
    player.frame(vec![]);
    player.frame(vec![Event::PointerMoved(from)]);
    player.frame(vec![
        press(from, true),
        Event::PointerMoved(to),
        press(to, false),
    ]);
    assert_eq!(painted_card(&player.frame(vec![])), initial);
    player.cover = None;
    player.frame(vec![]);
    player.frame(vec![Event::PointerMoved(from)]);
    player.frame(vec![press(from, true)]);
    assert!(player.chrome.stats.grab_offset.is_some());
    player.focused = false;
    player.frame(vec![Event::WindowFocused(false)]);
    player.focused = true;
    player.frame(vec![
        Event::WindowFocused(true),
        Event::PointerMoved(to),
        press(to, false),
    ]);
    assert_eq!(painted_card(&player.frame(vec![])), initial);
}

#[test]
fn releasing_a_title_drag_over_stop_watching_keeps_playback_active() {
    let mut player = Player::new(vec2(1040.0, 700.0));
    let opened = player.open();
    let initial = painted_card(&opened);
    let from = initial.min + vec2(45.0, 27.0);
    let target = text_rect(&opened, "Stop watching").unwrap().center();
    player.frame(vec![Event::PointerMoved(from)]);
    player.frame(vec![
        press(from, true),
        Event::PointerMoved(target),
        press(target, false),
    ]);
    assert_ne!(painted_card(&player.frame(vec![])), initial);
    assert!(!player.stopped);
}

#[test]
fn escape_dismisses_info_then_card_then_fullscreen_and_card_pins_hud() {
    let mut p = Player::new(vec2(1040.0, 700.0));
    p.open();
    p.fullscreen = true;
    p.frame(vec![]);
    let stable = p.frame(vec![]);
    let stop = text_rect(&stable, "Stop watching").unwrap();
    let info = icon_center(&p.ctx, &stable, "v2-icon-info-circle");
    p.click(info);
    assert!(
        p.chrome.info.open(),
        "info button at {info:?}, stop at {stop:?}"
    );
    assert!(p.chrome.stats.open);
    let info = p.frame(vec![Event::PointerGone]);
    for label in ["Requested policy", "Auto", "Hardware decoding", "wgpu"] {
        assert!(text_rect(&info, label).is_some(), "missing {label}");
    }
    let first = p.escape();
    assert!(!p.chrome.info.open());
    assert!(p.chrome.stats.open);
    let exits = |out: &FullOutput| {
        out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|cmd| matches!(cmd, ViewportCommand::Fullscreen(false)))
    };
    assert!(!exits(&first));
    p.time += 8.0;
    assert!(text_rect(&p.frame(vec![]), "Stop watching").is_some());
    let second = p.escape();
    assert!(!p.chrome.stats.open);
    assert!(!exits(&second));
    p.time += 2.51;
    assert!(text_rect(&p.frame(vec![]), "Stop watching").is_none());
    let third = p.escape();
    assert!(exits(&third));
}

#[test]
fn shared_cursor_reads_the_hovered_interval_in_either_chart_and_clears_in_new_session() {
    let mut p = Player::new(vec2(1040.0, 700.0));
    p.open();
    let sample = |start, at, decoded_fps, ui_fps| Sample {
        start,
        at,
        decoded_fps,
        ui_fps,
        codec_generation: Some(3),
    };
    p.chrome.stats.cached = Some((
        p.time,
        StatsSnapshot {
            generation: 7,
            enabled: true,
            now: 70.0,
            samples: vec![
                sample(39.0, 41.0, Some(30.0), Some(0.0)),
                sample(41.0, 51.0, None, None),
                sample(68.0, 69.0, Some(31.0), Some(30.0)),
            ],
            marks: vec![],
            missed: 9,
            overwritten_marks: 0,
        },
    ));
    let out = p.frame(vec![]);
    assert!(text_rect(&out, "22.5").is_some());
    let mid_x = text_rect(&out, "−30").unwrap().center().x;
    let fps_y = text_rect(&out, "Video frame rate").unwrap().bottom() + 72.0;
    let payload_y = text_rect(&out, "Payload bitrate unavailable")
        .unwrap()
        .center()
        .y;
    for y in [fps_y, payload_y] {
        let hovered = p.frame(vec![Event::PointerMoved(pos2(mid_x, y))]);
        assert!(text_rect(&hovered, "Decoded 30.0").is_some());
        assert!(text_rect(&hovered, "UI received 0.0").is_some());
        let delay = hovered.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay <= Duration::from_secs(1));
    }
    let away = p.frame(vec![Event::PointerMoved(pos2(4.0, 320.0))]);
    assert!(text_rect(&away, "UI received 30.0").is_some());
    let later_x = (mid_x + text_rect(&out, "−15").unwrap().center().x) * 0.5;
    let gap = p.click(pos2(later_x, fps_y));
    assert!(text_rect(&gap, "Decoded Missing").is_some());
    assert!(p.chrome.stats.selected.is_some());
    p.session = p.stats.begin(8);
    p.frame(vec![]);
    assert!(!p.chrome.stats.open && p.chrome.stats.selected.is_none());
    assert!(!p.stats.is_sampling(8));
}

#[test]
fn floating_card_stays_inside_the_stage_at_every_corner_after_resize() {
    for (size, header_height) in [
        (vec2(1040.0, 700.0), 62.0),
        (vec2(680.0, 520.0), 62.0),
        (vec2(1280.0, 720.0), 0.0),
    ] {
        let stage = Rect::from_min_max(pos2(0.0, header_height), pos2(size.x, size.y - 52.0));
        for corner in [
            vec2(-2000.0, -2000.0),
            vec2(2000.0, -2000.0),
            vec2(-2000.0, 2000.0),
            vec2(2000.0, 2000.0),
        ] {
            let card = card_rect(stage, Some(corner));
            assert!(stage.contains_rect(card));
            assert_eq!(card.width(), if size.x <= 680.0 { 396.0 } else { 418.0 });
            assert!(card.bottom() < size.y - 52.0);
        }
    }
}

#[test]
fn chinese_labels_fit_the_real_ui_font_at_normal_and_minimum_sizes() {
    let mut player = Player::new(vec2(1040.0, 700.0));
    crate::install_ui_font(&player.ctx, std::borrow::Cow::Borrowed(crate::NOTO_SANS_SC));
    player.locale = PlayerInfoLocale::Chinese;
    let out = player.frame(vec![]);
    player.click(text_rect(&out, "播放统计").unwrap().center());
    assert!(player.chrome.stats.open);
    let out = player.frame(vec![]);
    player.click(text_rect(&out, "开始采样").unwrap().center());
    assert!(player.stats.is_sampling(7));
    for size in [vec2(1040.0, 700.0), vec2(680.0, 520.0)] {
        player.screen = Rect::from_min_size(Pos2::ZERO, size);
        player.frame(vec![]);
        let painted = player.frame(vec![]);
        let card = card_rect(
            Rect::from_min_max(pos2(0.0, 62.0), pos2(size.x, size.y - 52.0)),
            None,
        );
        let facts_top = painted
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if (r.rect.width() - (card.width() - 22.0)).abs() < 0.1
                        && (r.rect.height() - 28.0).abs() < 0.1 =>
                {
                    Some(r.rect.top())
                }
                _ => None,
            })
            .unwrap();
        for name in ["停止采样", "标记", "导出"] {
            let text = text_rect(&painted, name).unwrap();
            let button = painted
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.rect.contains(text.center())
                            && r.rect.width() < 140.0
                            && r.rect.height() < 60.0 =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(
                button.bottom() <= facts_top - 4.0,
                "{name} {button:?}, strip at {facts_top}"
            );
        }
        for label in [
            "停止采样",
            "标记",
            "导出",
            "×",
            "解码输出 缺测",
            "UI 新接收 缺测",
            "SDK 解码输入 · 未接入",
        ] {
            let glyphs = painted
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some((t.visual_bounding_rect(), s.clip_rect))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label} at {size:?}"));
            assert!(
                card.contains_rect(glyphs.0),
                "{label} exceeds card at {size:?}"
            );
            assert!(
                glyphs.1.expand(1.0).contains_rect(glyphs.0),
                "{label} clipped at {size:?}"
            );
        }
    }
}
