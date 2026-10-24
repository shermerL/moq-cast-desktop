//! Render the real macOS pages against disconnected snapshots, without network side effects.
use super::*;

fn app() -> (MoqCastApp, impl FnMut() -> bool) {
    let (runtime, commands) = RuntimeOwner::isolated_for_ui_test();
    let diagnostics = moqcast_diagnostics::init(moqcast_diagnostics::Config::without_file(
        crate::build_info::diagnostics(),
        "UI fixture",
    ));
    let app = MoqCastApp {
        page: Page::Nearby,
        locale: Locale::English,
        developer_mode: false,
        diagnostics: diagnostics::DiagnosticsUi::new(diagnostics.handle(), false),
        detail_peer: None,
        settings_open: false,
        theme_preference: egui::ThemePreference::Light,
        command_error: false,
        confirm_turn_off_nearby: false,
        nearby_action_pending: None,
        runtime,
        system_lifecycle: None,
        player: player::Player::default(),
        playback_texture: None,
        playback_identity: None,
        playback_display: None,
        capture_picker: None,
        capture_permission: CapturePermission::Denied,
        picker_kind: capture_picker::Kind::default(),
        picker_pending: false,
        picker_error: false,
    };
    (app, commands)
}

fn snapshot(count: usize) -> AppSnapshot {
    let mut s = AppSnapshot::default();
    s.runtime.begin(RuntimePhase::Ready);
    s.discovery.begin(DiscoveryPhase::Scanning);
    s.session.begin(SessionPhase::Listening);
    s.local_device_name = Some("VeryLongNameWithoutSpaces中文中文".repeat(6));
    for n in 0..count {
        s.peers.insert(
            format!("peer-{n:02}"),
            PeerSnapshot {
                ordinal: n as u64 + 1,
                discovered: true,
                session: if n % 2 == 0 {
                    PeerSession::Connected
                } else {
                    PeerSession::Connecting
                },
            },
        );
    }
    s
}

fn render(
    ctx: &egui::Context,
    app: &mut MoqCastApp,
    snapshot: &AppSnapshot,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            app.desktop_shell(root, snapshot);
            app.show_turn_off_nearby_confirmation(root.ctx());
        },
    );
    output.textures_delta.clear();
    output
}

fn painted(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some(
                text.galley
                    .rect
                    .translate(text.pos.to_vec2())
                    .intersect(shape.clip_rect),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing painted label {label}"))
}

#[test]
fn real_pages_keep_primary_actions_inside_the_viewport() {
    let (mut app, _commands) = app();
    for width in [680.0, 919.0, 920.0, 1040.0, 1440.0] {
        for dark in [false, true] {
            for scale in [1.0, 2.0] {
                let ctx = egui::Context::default();
                install_ui_font(&ctx, std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC));
                moqcast_ui::v2::install(&ctx);
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                ctx.set_pixels_per_point(scale);
                let size = egui::vec2(width, 520.0);
                for count in [0, 1, 40] {
                    let snapshot = snapshot(count);
                    for page in [Page::Nearby, Page::ScreenShare] {
                        app.page = page;
                        for _ in 0..3 {
                            render(&ctx, &mut app, &snapshot, size, vec![]);
                        }
                        {
                            let mut output = render(&ctx, &mut app, &snapshot, size, vec![]);
                            let label = if page == Page::Nearby {
                                "Share…"
                            } else {
                                "Start sharing"
                            };
                            let rect = painted(&output, label);
                            assert!(rect.is_positive(), "{label} at {width}/{scale}/{count}");
                            assert!(
                                egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                                    .contains_rect(rect),
                                "{label} {rect:?} at width={width} scale={scale} count={count} page={page:?}"
                            );
                            if page == Page::Nearby {
                                assert!(rect.bottom() < painted(&output, "Turn off Nearby").top());
                            }
                            if page == Page::Nearby && count == 0 {
                                let label = "Open MoQCast on another device on this network";
                                let shape = output
                                    .shapes
                                    .iter()
                                    .find_map(|shape| match &shape.shape {
                                        egui::epaint::Shape::Text(text)
                                            if text.galley.job.text == label =>
                                        {
                                            Some((
                                                text.galley.rect.translate(text.pos.to_vec2()),
                                                shape.clip_rect,
                                            ))
                                        }
                                        _ => None,
                                    })
                                    .expect("empty state explanation must be painted");
                                assert!(
                                    shape.1.contains_rect(shape.0),
                                    "empty explanation clipped at {width}: {:?}",
                                    shape
                                );
                            }
                            output.textures_delta.clear();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn overflow_list_reaches_last_device_and_settings_keep_close_visible() {
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    let (mut app, _commands) = app();
    let s = snapshot(40);
    let size = egui::vec2(680.0, 520.0);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![])
            .textures_delta
            .clear();
    }
    let events = vec![
        egui::Event::PointerMoved(egui::pos2(320.0, 360.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, -10000.0),
            modifiers: Default::default(),
        },
    ];
    render(&ctx, &mut app, &s, size, events)
        .textures_delta
        .clear();
    // Egui consumes smooth wheel input over several frames; no wall-clock sleep is needed.
    let mut output = render(&ctx, &mut app, &s, size, vec![]);
    for _ in 0..40 {
        output.textures_delta.clear();
        output = render(&ctx, &mut app, &s, size, vec![]);
    }
    assert!(painted(&output, "Nearby device 40").is_positive());
    output.textures_delta.clear();
    app.settings_open = true;
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    for _ in 0..3 {
        let mut output = render(&ctx, &mut app, &s, size, vec![]);
        assert!(painted(&output, "Settings").is_positive());
        assert!(painted(&output, "Settings").bottom() < size.y);
        output.textures_delta.clear();
    }
}

#[test]
fn details_errors_and_player_navigation_preserve_reachable_actions() {
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    let (mut app, mut commands) = app();
    let mut s = snapshot(1);
    let size = egui::vec2(680.0, 520.0);
    app.detail_peer = Some("peer-00".into());
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let output = render(&ctx, &mut app, &s, size, vec![]);
    assert!(painted(&output, "Device details").is_positive());
    app.detail_peer = None;
    app.page = Page::ScreenShare;
    app.picker_error = true;
    app.command_error = true;
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let output = render(&ctx, &mut app, &s, size, vec![]);
    assert!(painted(&output, "Start sharing").is_positive());
    app.command_error = false;
    app.page = Page::Watch;
    s.media_owner = Some(MediaOwner::Watch);
    s.media.begin(MediaPhase::Watching);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let output = render(&ctx, &mut app, &s, size, vec![]);
    assert!(painted(&output, "LIVE").is_positive());
    let point = painted(&output, "‹  Nearby").center();
    for pressed in [true, false] {
        render(
            &ctx,
            &mut app,
            &s,
            size,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert_eq!(app.page, Page::Nearby);
    assert!(!commands(), "navigation must not issue any media command");
    assert!(
        !app.command_error,
        "returning to Nearby must not send a stop command"
    );
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let output = render(&ctx, &mut app, &s, size, vec![]);
    assert!(painted(&output, "Return to player").is_positive());
}

#[test]
fn settings_expose_log_export_without_scrolling_or_developer_mode() {
    for locale in [Locale::English, Locale::Chinese] {
        for developer in [false, true] {
            let ctx = egui::Context::default();
            install_ui_font(&ctx, std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC));
            moqcast_ui::v2::install(&ctx);
            let (mut app, _commands) = app();
            app.settings_open = true;
            app.locale = locale;
            app.developer_mode = developer;
            let size = egui::vec2(680.0, 520.0);
            let s = snapshot(0);
            for _ in 0..3 {
                render(&ctx, &mut app, &s, size, vec![]);
            }
            let output = render(&ctx, &mut app, &s, size, vec![]);
            for label in match locale {
                Locale::English => ["Export local logs", "Developer mode", "Settings"],
                Locale::Chinese => ["导出本地日志", "开发者模式", "设置"],
            } {
                let shape = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some((
                            text.galley.rect.translate(text.pos.to_vec2()),
                            shape.clip_rect,
                        )),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {label}, developer={developer}"));
                assert!(shape.1.contains_rect(shape.0), "clipped {label}: {shape:?}");
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(shape.0));
            }
        }
    }
}

#[test]
fn long_playback_error_keeps_recovery_action_visible() {
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    let (mut app, _commands) = app();
    let mut s = snapshot(0);
    s.media_owner = Some(MediaOwner::Watch);
    s.media.begin(MediaPhase::Failed);
    s.media_error = Some("A detailed media failure with a long explanation.\n".repeat(30));
    app.page = Page::Watch;
    let size = egui::vec2(680.0, 520.0);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let output = render(&ctx, &mut app, &s, size, vec![]);
    let button = painted(&output, "Stop and return");
    assert!(button.is_positive());
    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(button));
}

#[test]
fn dialog_close_icons_only_dismiss_their_surface() {
    for surface in 0..3 {
        let ctx = egui::Context::default();
        moqcast_ui::v2::install(&ctx);
        let (mut app, mut commands) = app();
        let s = snapshot(1);
        match surface {
            0 => app.settings_open = true,
            1 => app.detail_peer = Some("peer-00".into()),
            _ => app.confirm_turn_off_nearby = true,
        }
        let size = egui::vec2(680.0, 520.0);
        for _ in 0..3 {
            render(&ctx, &mut app, &s, size, vec![]);
        }
        let output = render(&ctx, &mut app, &s, size, vec![]);
        let close_texture = ctx
            .data(|data| data.get_temp::<egui::TextureHandle>(egui::Id::new("v2-tabler-close")))
            .expect("close texture")
            .id();
        let icon = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == close_texture => {
                    Some(mesh.calc_bounds())
                }
                _ => None,
            })
            .expect("dialog must have a close image");
        assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(icon));
        for pressed in [true, false] {
            render(
                &ctx,
                &mut app,
                &s,
                size,
                vec![
                    egui::Event::PointerMoved(icon.center()),
                    egui::Event::PointerButton {
                        pos: icon.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert!(!app.settings_open && app.detail_peer.is_none() && !app.confirm_turn_off_nearby);
        assert!(
            !commands(),
            "closing a dialog must not send network or media commands"
        );
    }
}

#[test]
fn scanning_uses_success_indicator_and_listener_failure_is_not_healthy() {
    let (mut app, _commands) = app();
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let size = egui::vec2(1040.0, 700.0);
    let mut s = snapshot(0);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let output = render(&ctx, &mut app, &s, size, vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Circle(circle) if circle.fill == egui::Color32::from_rgb(0x40,0xa6,0x6d))),
        "healthy discovery needs the S2 green indicator");
    s.session.begin(SessionPhase::Failed);
    let output = render(&ctx, &mut app, &s, size, vec![]);
    assert!(painted(&output, "Discovering; device connections unavailable").is_positive());
}

#[test]
fn player_is_a_dedicated_canvas_without_the_discovery_footer() {
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    let (mut app, _commands) = app();
    app.page = Page::Watch;
    let mut s = snapshot(1);
    s.media_owner = Some(MediaOwner::Watch);
    s.media.begin(MediaPhase::PreparingWatch);
    let size = egui::vec2(680.0, 520.0);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let out = render(&ctx, &mut app, &s, size, vec![]);
    let labels: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(t) => Some(t.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !labels.contains(&"Turn off Nearby"),
        "player must not nest inside discovery chrome"
    );
    assert!(
        !labels.contains(&"Preparing video"),
        "waiting uses only a centered spinner"
    );
    assert!(painted(&out, "‹  Nearby").is_positive());
}

#[test]
fn sharing_keeps_audio_and_primary_action_visible_at_minimum_size() {
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    let (mut app, _commands) = app();
    app.page = Page::ScreenShare;
    let s = snapshot(0);
    let size = egui::vec2(680.0, 520.0);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let out = render(&ctx, &mut app, &s, size, vec![]);
    for label in ["Share system audio", "Start sharing"] {
        let bounds = painted(&out, label);
        assert!(
            bounds.is_positive(),
            "{label} must be visible without scrolling"
        );
        assert!(bounds.bottom() < size.y);
    }
}

#[test]
fn sharing_can_enable_nearby_without_leaving_source_setup() {
    let ctx = egui::Context::default();
    moqcast_ui::v2::install(&ctx);
    let (mut app, mut commands) = app();
    app.page = Page::ScreenShare;
    app.capture_permission = CapturePermission::Allowed;
    let mut s = AppSnapshot::default();
    s.runtime.begin(RuntimePhase::Ready);
    s.discovery.begin(DiscoveryPhase::Stopped);
    s.session.begin(SessionPhase::Stopped);
    let size = egui::vec2(680.0, 520.0);
    for _ in 0..3 {
        render(&ctx, &mut app, &s, size, vec![]);
    }
    let out = render(&ctx, &mut app, &s, size, vec![]);
    let point = painted(&out, "Turn on Nearby").center();
    for pressed in [true, false] {
        render(
            &ctx,
            &mut app,
            &s,
            size,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert_eq!(app.page, Page::ScreenShare);
    assert!(
        commands(),
        "the visible recovery action sends a runtime command"
    );
}
