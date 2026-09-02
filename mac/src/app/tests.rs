use super::*;

fn peer(discovered: bool, session: PeerSession) -> PeerSnapshot {
    PeerSnapshot {
        ordinal: 1,
        discovered,
        session,
    }
}

#[test]
fn stored_locale_defaults_to_chinese_and_accepts_english() {
    assert_eq!(Locale::from_storage(None), Locale::Chinese);
    assert_eq!(Locale::from_storage(Some("en".to_owned())), Locale::English);
    assert_eq!(Locale::English.stored(), "en");
}

#[test]
fn developer_mode_defaults_off_and_requires_an_explicit_true_value() {
    assert!(!developer_mode_from_storage(None));
    assert!(!developer_mode_from_storage(Some("false".to_owned())));
    assert!(!developer_mode_from_storage(Some("invalid".to_owned())));
    assert!(developer_mode_from_storage(Some("true".to_owned())));
}

#[test]
fn layout_breakpoints_match_the_frozen_preview() {
    assert_eq!(ContentLayout::for_width(919.0), ContentLayout::SingleColumn);
    assert_eq!(ContentLayout::for_width(920.0), ContentLayout::ListDetail);
    assert_eq!(
        NavigationLayout::for_width(919.0),
        NavigationLayout::TwoRows
    );
    assert_eq!(NavigationLayout::for_width(920.0), NavigationLayout::OneRow);
}

#[test]
fn peer_presentation_keeps_presence_and_session_independent() {
    let waiting = PeerPresentation::from(&peer(true, PeerSession::Waiting));
    assert_eq!(waiting.presence, PresenceView::Nearby);
    assert_eq!(waiting.connection, ConnectionView::Waiting);

    let connecting = PeerPresentation::from(&peer(true, PeerSession::Connecting));
    assert_eq!(connecting.connection, ConnectionView::ConnectingSecurely);

    let lost_but_connected = PeerPresentation::from(&peer(false, PeerSession::Connected));
    assert_eq!(lost_but_connected.presence, PresenceView::NotNearby);
    assert_eq!(lost_but_connected.connection, ConnectionView::Connected);
}

#[test]
fn screen_availability_uses_only_the_canonical_peer_path() {
    let mut screens = std::collections::BTreeMap::new();
    screens.insert(
        crate::contract::screen_path("peer-a"),
        crate::remote::ScreenView {
            peer_id: "peer-a".to_owned(),
            availability: crate::remote::ScreenAvailability::Available,
        },
    );
    screens.insert(
        crate::contract::screen_path("peer-b"),
        crate::remote::ScreenView {
            peer_id: "different-peer".to_owned(),
            availability: crate::remote::ScreenAvailability::Available,
        },
    );

    assert_eq!(
        screen_availability("peer-a", &screens),
        crate::remote::ScreenAvailability::Available
    );
    assert_eq!(
        screen_availability("peer-b", &screens),
        crate::remote::ScreenAvailability::Unavailable
    );
}

#[test]
fn selection_survives_lost_while_the_session_is_healthy() {
    let mut peers = std::collections::BTreeMap::new();
    peers.insert("peer-a".to_owned(), peer(false, PeerSession::Connected));
    peers.insert("peer-b".to_owned(), peer(true, PeerSession::Waiting));

    assert_eq!(
        selected_peer(Some("peer-a"), &peers),
        Some("peer-a".to_owned())
    );
    assert_eq!(
        selected_peer(Some("gone"), &peers),
        Some("peer-a".to_owned())
    );
}

#[test]
fn configured_fonts_cover_core_simplified_chinese() {
    let context = egui::Context::default();
    install_ui_font(
        &context,
        std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
    );
    let mut output = context.run_ui(Default::default(), |ui| {
        ui.fonts_mut(|fonts| {
            assert!(fonts.has_glyphs(&egui::FontId::proportional(13.0), "附近设备屏幕共享设置"));
        });
    });
    output.textures_delta.clear();
}

#[test]
fn watch_idle_and_failed_use_state_panels_while_active_phases_use_the_player() {
    assert_eq!(
        watch_projection(None, MediaPhase::Idle),
        WatchProjection::Empty
    );
    assert_eq!(
        watch_projection(Some(MediaOwner::Watch), MediaPhase::Failed),
        WatchProjection::Failed
    );
    for phase in [
        MediaPhase::PreparingWatch,
        MediaPhase::Watching,
        MediaPhase::Stopping,
    ] {
        assert_eq!(
            watch_projection(Some(MediaOwner::Watch), phase),
            WatchProjection::Player
        );
    }
}

#[test]
fn nearby_copy_and_confirmation_follow_the_network_and_media_lifecycles() {
    let mut snapshot = AppSnapshot::default();
    snapshot.discovery.begin(DiscoveryPhase::Starting);
    assert_eq!(
        global_summary(&snapshot, Locale::Chinese),
        "正在开启附近设备"
    );
    assert_eq!(
        global_summary(&snapshot, Locale::English),
        "Turning on Nearby"
    );

    snapshot.discovery.begin(DiscoveryPhase::Scanning);
    snapshot.session.begin(SessionPhase::Listening);
    assert_eq!(global_summary(&snapshot, Locale::Chinese), "附近设备已开启");
    assert_eq!(global_summary(&snapshot, Locale::English), "Nearby is on");
    assert_eq!(
        local_status(&snapshot, Locale::Chinese),
        "附近设备已开启，正在自动查找"
    );
    assert!(!has_active_media(&snapshot));

    snapshot.media.begin(MediaPhase::PreparingWatch);
    assert!(has_active_media(&snapshot));

    snapshot.discovery.begin(DiscoveryPhase::Stopped);
    snapshot.session.begin(SessionPhase::Stopped);
    assert_eq!(local_status(&snapshot, Locale::Chinese), "附近设备已关闭");
}

#[test]
fn nearby_action_pending_coalesces_duplicate_start_intents_until_acknowledged() {
    let mut snapshot = AppSnapshot::default();
    snapshot.discovery.begin(DiscoveryPhase::Stopped);
    let generation = snapshot.discovery.generation();
    let mut pending = None;
    let mut sends = 0;

    assert!(begin_nearby_action(
        &mut pending,
        NearbyAction::TurnOn,
        generation,
        || {
            sends += 1;
            true
        },
    ));
    assert!(!begin_nearby_action(
        &mut pending,
        NearbyAction::TurnOn,
        generation,
        || {
            sends += 1;
            true
        },
    ));
    assert_eq!(sends, 1);
    assert!(!nearby_action_enabled(pending, RuntimePhase::Ready));

    reconcile_nearby_action(&mut pending, &snapshot);
    assert!(pending.is_some());
    snapshot.discovery.begin(DiscoveryPhase::Stopped);
    snapshot.runtime.begin(RuntimePhase::Suspended);
    reconcile_nearby_action(&mut pending, &snapshot);
    assert!(pending.is_some());
    snapshot.runtime.begin(RuntimePhase::Ready);
    snapshot.discovery.begin(DiscoveryPhase::Starting);
    reconcile_nearby_action(&mut pending, &snapshot);
    assert!(pending.is_none());
    assert!(nearby_action_enabled(pending, RuntimePhase::Ready));
}

#[test]
fn shared_navigation_uses_the_compact_height_below_the_split_breakpoint() {
    assert_eq!(navigation_height(919.0), Size::APP_BAR_COMPACT);
    assert_eq!(navigation_height(920.0), Size::APP_BAR);
}

#[test]
fn scrollable_pages_have_independent_stable_ids() {
    let nearby = Page::Nearby.scroll_id();
    let share = Page::ScreenShare.scroll_id();
    let settings = Page::Settings.scroll_id();
    assert_ne!(nearby, share);
    assert_ne!(nearby, settings);
    assert_ne!(share, settings);
    assert_eq!(nearby, Page::Nearby.scroll_id());
}

#[test]
fn centered_page_content_leaves_the_scrollbar_at_the_viewport_edge() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1040.0, 700.0));
    let content = moqcast_ui::page_content_rect(viewport, PageWidth::Narrow);
    assert_eq!(content.center().x, viewport.center().x);
    assert!(content.right() < viewport.right());
    assert_eq!(content.width(), Size::PAGE_NARROW_MAX);
}

#[test]
fn ordinary_ui_copy_keeps_internal_provenance_private() {
    let source = include_str!("../app.rs");
    let forbidden = [
        ["commit", " SHA"].concat(),
        ["A", "BI"].concat(),
        ["build", " variant"].concat(),
        ["dependency", " identity"].concat(),
        ["unattributed incoming", " sessions"].concat(),
        ["未归属传入", "连接"].concat(),
    ];
    for forbidden in forbidden {
        assert!(
            !source.contains(&forbidden),
            "visible copy contains {forbidden}"
        );
    }
}

#[test]
fn identity_copy_shows_current_run_and_remote_peer_ids_without_provenance() {
    let local = local_device_description(
        Locale::English,
        "Mac Studio",
        "Nearby is ready",
        Some("local-peer"),
    );
    let remote = peer_list_subtitle(Locale::English, "remote-peer", "Connected");

    assert_eq!(local_device_id_label(Locale::English), "This device ID");
    assert_eq!(remote_device_id_label(Locale::English), "Device ID");
    assert!(local.contains("Current run: local-peer"));
    assert!(remote.contains("Device ID: remote-peer"));
    assert!(!local.contains("fingerprint"));
    assert!(!remote.contains("fingerprint"));
}

#[test]
fn share_action_requires_permission_source_and_idle_media() {
    let mut snapshot = AppSnapshot::default();
    assert!(!share_action_available(
        CapturePermission::Allowed,
        &snapshot
    ));

    snapshot.share_selection = Some(crate::publication::Selection::Display {
        display_id: 7,
        primary: true,
        label: "Display 7".to_owned(),
    });
    assert!(!share_action_available(
        CapturePermission::NotRequested,
        &snapshot
    ));
    snapshot.session.begin(SessionPhase::Listening);
    assert!(share_action_available(
        CapturePermission::Allowed,
        &snapshot
    ));

    snapshot.media.begin(MediaPhase::PreparingShare);
    assert!(!share_action_available(
        CapturePermission::Allowed,
        &snapshot
    ));
}

#[test]
fn system_audio_action_requires_listening_session() {
    let mut snapshot = AppSnapshot {
        share_selection: Some(crate::publication::Selection::Display {
            display_id: 7,
            primary: true,
            label: "Display 7".to_owned(),
        }),
        ..Default::default()
    };

    assert!(!system_audio_action_available(&snapshot));
    snapshot.session.begin(SessionPhase::Listening);
    assert!(system_audio_action_available(&snapshot));
}

#[test]
fn system_audio_action_requires_an_idle_primary_display() {
    let mut snapshot = AppSnapshot {
        share_selection: Some(crate::publication::Selection::Display {
            display_id: 7,
            primary: true,
            label: "Display 7".to_owned(),
        }),
        ..AppSnapshot::default()
    };
    snapshot.session.begin(SessionPhase::Listening);
    assert!(system_audio_action_available(&snapshot));

    snapshot.share_selection = Some(crate::publication::Selection::Display {
        display_id: 8,
        primary: false,
        label: "Display 8".to_owned(),
    });
    assert!(!system_audio_action_available(&snapshot));

    snapshot.share_selection = Some(crate::publication::Selection::Window {
        window_id: 9,
        label: "Window".to_owned(),
    });
    assert!(!system_audio_action_available(&snapshot));

    snapshot.share_selection = Some(crate::publication::Selection::Display {
        display_id: 7,
        primary: true,
        label: "Display 7".to_owned(),
    });
    snapshot.media.begin(MediaPhase::PreparingShare);
    assert!(!system_audio_action_available(&snapshot));
}
