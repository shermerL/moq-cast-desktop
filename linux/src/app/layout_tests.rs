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
