use super::*;
fn fixture() -> (MoqCastApp, tokio::sync::mpsc::Receiver<UserCommand>) {
    let (runtime, commands) = RuntimeHandle::isolated_for_ui_test();
    let diagnostics = moqcast_diagnostics::init(moqcast_diagnostics::Config::without_file(
        moqcast_diagnostics::BuildInfo::new("test"),
        "UI fixture",
    ));
    (
        MoqCastApp {
            page: Page::Nearby,
            selected_peer: None,
            settings_open: false,
            confirm_nearby_off: false,
            nearby_turn_off_pending: false,
            theme_preference: egui::ThemePreference::Light,
            locale: Locale::English,
            system_audio: false,
            selected_source: None,
            developer_mode: false,
            local_device_name: "LongLocalDeviceName中文".repeat(10),
            diagnostics: diagnostics::DiagnosticsUi::new(diagnostics.handle(), false),
            runtime,
            command_error: None,
            playback_texture: None,
            playback_identity: None,
            player: player::LivePlayer::default(),
        },
        commands,
    )
}
fn render(
    ctx: &egui::Context,
    app: &mut MoqCastApp,
    s: &AppSnapshot,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| app.desktop_shell(root, s),
    );
    out.textures_delta.clear();
    out
}
fn painted(out: &egui::FullOutput, label: &str) -> egui::Rect {
    out.shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(
                text.galley
                    .rect
                    .translate(text.pos.to_vec2())
                    .intersect(shape.clip_rect),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing label {label}"))
}
#[test]
fn page_matrix_preserves_primary_actions_and_log_export() {
    let (mut app, _commands) = fixture();
    let s = AppSnapshot {
        sources: SourceCatalog::Portal,
        ..Default::default()
    };
    for width in [680.0, 919.0, 920.0, 1440.0] {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            theme::configure(&ctx);
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let size = egui::vec2(width, 640.0);
            for page in [Page::Nearby, Page::ScreenShare, Page::Watch] {
                app.page = page;
                app.settings_open = false;
                let out = render(&ctx, &mut app, &s, size, vec![]);
                let label = match page {
                    Page::Nearby => "Turn on Nearby",
                    Page::ScreenShare => "Turn on Nearby",
                    Page::Watch => app.locale.open_nearby(),
                };
                let rect = painted(&out, label);
                assert!(rect.is_positive() && rect.bottom() <= size.y);
            }
            app.page = Page::Nearby;
            app.settings_open = true;
            render(&ctx, &mut app, &s, size, vec![]);
            let out = render(&ctx, &mut app, &s, size, vec![]);
            assert!(painted(&out, app.locale.export_logs()).is_positive());
            assert!(!app.developer_mode);
        }
    }
}
#[test]
fn back_to_nearby_keeps_media_running() {
    let (mut app, mut commands) = fixture();
    let s = AppSnapshot {
        media: MediaState::PreparingView {
            path: crate::screen_path::for_peer("peer"),
        },
        ..Default::default()
    };
    app.page = Page::Watch;
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    let size = egui::vec2(680.0, 640.0);
    let out = render(&ctx, &mut app, &s, size, vec![]);
    let pos = painted(&out, "‹  Nearby").center();
    for pressed in [true, false] {
        render(
            &ctx,
            &mut app,
            &s,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(app.page, Page::Nearby);
    assert!(commands.try_recv().is_err());
}

#[test]
fn shutdown_snapshot_disables_nearby_toggle_and_reports_progress() {
    let (mut app, mut commands) = fixture();
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    let size = egui::vec2(1040.0, 640.0);
    let s = AppSnapshot {
        discovery: DiscoveryState::Ready,
        local_peer_id: Some("local".into()),
        lan_stopping: true,
        ..Default::default()
    };
    let out = render(&ctx, &mut app, &s, size, vec![]);
    assert!(painted(&out, "Turning off Nearby").is_positive());
    let pos = painted(&out, "Turning off").center();
    for pressed in [true, false] {
        render(
            &ctx,
            &mut app,
            &s,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(commands.try_recv().is_err());
}

#[test]
fn nearby_identity_is_visible_without_opening_details() {
    let (mut app, _commands) = fixture();
    let mut snapshot = AppSnapshot::default();
    snapshot.peers.insert(
        "remote-peer".into(),
        PeerSnapshot {
            bandwidth: Default::default(),
            name: "Remote device".into(),
            endpoints: vec![],
            fingerprint_pinned: true,
            dial_role: DialRole::Inbound,
            discovery: PeerDiscoveryState::Found,
            transport: TransportState::Waiting,
            screen: ScreenAvailability::Unavailable,
        },
    );
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    let out = render(&ctx, &mut app, &snapshot, egui::vec2(1040.0, 640.0), vec![]);
    assert!(painted(&out, "Device ID: remote-peer").is_positive());
}

#[test]
fn rendering_after_nearby_off_does_not_restart_discovery() {
    let (mut app, mut commands) = fixture();
    app.send(UserCommand::StopDiscovery);
    assert!(matches!(
        commands.try_recv(),
        Ok(UserCommand::StopDiscovery)
    ));
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    for page in [Page::Nearby, Page::ScreenShare, Page::Nearby] {
        app.page = page;
        render(
            &ctx,
            &mut app,
            &AppSnapshot::default(),
            egui::vec2(1040.0, 640.0),
            vec![],
        );
    }
    while let Ok(command) = commands.try_recv() {
        assert!(!matches!(
            command,
            UserCommand::StartDiscovery | UserCommand::RetryDiscovery
        ));
    }
}

#[test]
fn nearby_highlights_only_playing_not_preparing_or_stopping() {
    for media in [
        MediaState::PreparingView {
            path: crate::screen_path::for_peer("peer"),
        },
        MediaState::Viewing {
            path: crate::screen_path::for_peer("peer"),
        },
        MediaState::StoppingView {
            path: crate::screen_path::for_peer("peer"),
        },
    ] {
        let (mut app, mut commands) = fixture();
        app.nearby_turn_off_pending = true;
        let mut s = AppSnapshot {
            media: media.clone(),
            local_peer_id: Some("local".into()),
            discovery: DiscoveryState::Ready,
            ..Default::default()
        };
        s.peers.insert(
            "peer".into(),
            PeerSnapshot {
                bandwidth: Default::default(),
                name: "Computer".into(),
                endpoints: vec![],
                fingerprint_pinned: true,
                dial_role: DialRole::Outbound,
                discovery: PeerDiscoveryState::Lost,
                transport: TransportState::Connected,
                screen: ScreenAvailability::Available,
            },
        );
        let ctx = egui::Context::default();
        theme::configure(&ctx);
        ctx.set_theme(egui::Theme::Light);
        let out = render(&ctx, &mut app, &s, egui::vec2(1040.0, 700.0), vec![]);
        let highlighted = out.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill == moqcast_ui::v2::Palette::new(false).soft
                && rect.rect.width() > 400.0 && rect.rect.height() < 100.0)
        });
        assert_eq!(
            highlighted,
            matches!(media, MediaState::Viewing { .. }),
            "{media:?}"
        );
        let size = egui::vec2(1040.0, 700.0);
        let out = render(&ctx, &mut app, &s, size, vec![]);
        let pos = painted(&out, "Return").center();
        for pressed in [true, false] {
            render(
                &ctx,
                &mut app,
                &s,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.page, Page::Watch);
        assert!(
            commands.try_recv().is_err(),
            "return must not start or stop media"
        );
    }
}

#[test]
fn pending_nearby_off_disables_new_watch_before_runtime_acknowledges() {
    check_pending_off(Page::Nearby);
}

#[test]
fn pending_nearby_off_disables_new_share_before_runtime_acknowledges() {
    check_pending_off(Page::ScreenShare);
}

fn check_pending_off(page: Page) {
    for (pending, discovered, available) in [
        (false, true, true),
        (true, true, true),
        (false, false, true),
        (true, false, true),
        (false, false, false),
    ] {
        let (mut app, mut commands) = fixture();
        app.page = page;
        let mut s = AppSnapshot {
            local_peer_id: Some("local".into()),
            discovery: DiscoveryState::Ready,
            sources: SourceCatalog::Portal,
            ..Default::default()
        };
        s.peers.insert(
            "peer".into(),
            PeerSnapshot {
                bandwidth: Default::default(),
                name: "Computer".into(),
                endpoints: vec![],
                fingerprint_pinned: true,
                dial_role: DialRole::Outbound,
                discovery: if discovered {
                    PeerDiscoveryState::Found
                } else {
                    PeerDiscoveryState::Lost
                },
                transport: TransportState::Connected,
                screen: if available {
                    ScreenAvailability::Available
                } else {
                    ScreenAvailability::Withdrawn
                },
            },
        );
        s.update_remote_screen(crate::screen_path::for_peer("peer"), available);
        if pending {
            app.send(UserCommand::StopDiscovery);
            assert!(matches!(
                commands.try_recv(),
                Ok(UserCommand::StopDiscovery)
            ));
        }
        let ctx = egui::Context::default();
        theme::configure(&ctx);
        let size = egui::vec2(1040.0, 700.0);
        render(&ctx, &mut app, &s, size, vec![]);
        let out = render(&ctx, &mut app, &s, size, vec![]);
        let label = if page == Page::Nearby {
            app.locale.watch()
        } else {
            "Start sharing"
        };
        let pos = painted(&out, label).center();
        for pressed in [true, false] {
            render(
                &ctx,
                &mut app,
                &s,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            commands.try_recv().is_ok(),
            !pending && (page != Page::Nearby || available),
            "{page:?} pending={pending} discovered={discovered} available={available}"
        );
    }
}

#[test]
fn nearby_pending_intent_clears_after_cleanup_and_is_not_set_on_queue_failure() {
    let (mut app, mut commands) = fixture();
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    let size = egui::vec2(1040.0, 700.0);
    let mut s = AppSnapshot {
        local_peer_id: Some("local".into()),
        discovery: DiscoveryState::Ready,
        ..Default::default()
    };
    app.send(UserCommand::StopDiscovery);
    assert!(matches!(
        commands.try_recv(),
        Ok(UserCommand::StopDiscovery)
    ));
    render(&ctx, &mut app, &s, size, vec![]);
    assert!(app.nearby_turn_off_pending);
    s.lan_stopping = true;
    s.local_peer_id = None;
    render(&ctx, &mut app, &s, size, vec![]);
    assert!(app.nearby_turn_off_pending);
    s.lan_stopping = false;
    s.discovery = DiscoveryState::Idle;
    render(&ctx, &mut app, &s, size, vec![]);
    assert!(!app.nearby_turn_off_pending);
    let out = render(&ctx, &mut app, &s, size, vec![]);
    let pos = painted(&out, "Turn on Nearby").center();
    for pressed in [true, false] {
        render(
            &ctx,
            &mut app,
            &s,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(matches!(
        commands.try_recv(),
        Ok(UserCommand::StartDiscovery)
    ));
    drop(commands);
    app.send(UserCommand::StopDiscovery);
    assert!(app.command_error.is_some());
    assert!(!app.nearby_turn_off_pending);
}

#[test]
fn nearby_off_during_startup_waits_for_terminal_discovery_state() {
    let (mut app, mut commands) = fixture();
    let ctx = egui::Context::default();
    theme::configure(&ctx);
    let mut s = AppSnapshot {
        discovery: DiscoveryState::Scanning,
        ..Default::default()
    };
    app.send(UserCommand::StopDiscovery);
    assert!(matches!(
        commands.try_recv(),
        Ok(UserCommand::StopDiscovery)
    ));
    let size = egui::vec2(1040.0, 700.0);
    render(&ctx, &mut app, &s, size, vec![]);
    assert!(
        app.nearby_turn_off_pending,
        "startup without a local ID is not a shutdown acknowledgement"
    );
    s.lan_stopping = true;
    s.discovery = DiscoveryState::Idle;
    render(&ctx, &mut app, &s, size, vec![]);
    assert!(app.nearby_turn_off_pending);
    s.lan_stopping = false;
    s.discovery = DiscoveryState::Error;
    render(&ctx, &mut app, &s, size, vec![]);
    assert!(
        !app.nearby_turn_off_pending,
        "completed cleanup errors must not lock the retry action"
    );
}
