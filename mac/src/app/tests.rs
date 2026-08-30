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
fn layout_breakpoints_match_the_frozen_preview() {
    assert_eq!(ContentLayout::for_width(919.0), ContentLayout::SingleColumn);
    assert_eq!(ContentLayout::for_width(920.0), ContentLayout::ListDetail);
    assert_eq!(
        NavigationLayout::for_width(759.0),
        NavigationLayout::TwoRows
    );
    assert_eq!(NavigationLayout::for_width(760.0), NavigationLayout::OneRow);
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
    configure_fonts(&context);
    let mut output = context.run_ui(Default::default(), |ui| {
        ui.fonts_mut(|fonts| {
            assert!(fonts.has_glyphs(&egui::FontId::proportional(13.0), "附近设备屏幕共享设置"));
        });
    });
    output.textures_delta.clear();
}

#[test]
fn ordinary_ui_copy_keeps_internal_provenance_private() {
    let source = include_str!("../app.rs");
    let forbidden = [
        ["commit", " SHA"].concat(),
        ["A", "BI"].concat(),
        ["build", " variant"].concat(),
        ["dependency", " identity"].concat(),
    ];
    for forbidden in forbidden {
        assert!(
            !source.contains(&forbidden),
            "visible copy contains {forbidden}"
        );
    }
}
