use std::thread;
use std::time::{Duration, Instant};

use moq_cast_desktop::app::{
    AppSnapshot, DiscoveredPeer, DiscoveryState, MediaState, PeerDiscoveryState,
    ScreenAvailability, StateError, TransportState, UserCommand,
};
use moq_cast_desktop::runtime::RuntimeHandle;

fn peer(id: &str) -> DiscoveredPeer {
    DiscoveredPeer {
        id: id.into(),
        name: id.into(),
        endpoints: vec!["192.0.2.10:4443".into()],
        fingerprint_pinned: true,
    }
}

fn connected_snapshot() -> AppSnapshot {
    let mut snapshot = AppSnapshot::default();
    snapshot.upsert_peer(peer("android-living-room"));
    snapshot.set_transport("android-living-room", TransportState::Connected);
    snapshot
}

#[test]
fn publishing_requires_a_mesh_session() {
    let mut snapshot = AppSnapshot::default();

    assert_eq!(snapshot.begin_publish(), Err(StateError::MeshNotConnected));
    assert_eq!(snapshot.media, MediaState::Idle);
}

#[test]
fn duplicate_publish_and_stop_are_rejected() {
    let mut snapshot = connected_snapshot();

    snapshot.begin_publish().unwrap();
    snapshot.finish_publish().unwrap();
    assert_eq!(
        snapshot.begin_publish(),
        Err(StateError::MediaAlreadyActive)
    );

    snapshot.begin_stop_publish().unwrap();
    assert_eq!(
        snapshot.begin_stop_publish(),
        Err(StateError::PublishAlreadyStopping)
    );
    snapshot.finish_stop_publish().unwrap();
    assert_eq!(snapshot.media, MediaState::Idle);
}

#[test]
fn publish_and_remote_view_are_mutually_exclusive() {
    let mut snapshot = connected_snapshot();
    snapshot.update_remote_screen("moqcast.screen/android-living-room".into(), true);

    snapshot.begin_publish().unwrap();
    assert_eq!(
        snapshot.begin_view("moqcast.screen/android-living-room"),
        Err(StateError::MediaAlreadyActive)
    );
    snapshot.fail_publish("cancelled").unwrap();

    snapshot
        .begin_view("moqcast.screen/android-living-room")
        .unwrap();
    snapshot.finish_view().unwrap();
    assert_eq!(
        snapshot.begin_publish(),
        Err(StateError::MediaAlreadyActive)
    );
}

#[test]
fn a_failed_media_preparation_returns_to_idle_and_can_retry() {
    let mut snapshot = connected_snapshot();

    snapshot.begin_publish().unwrap();
    snapshot
        .fail_publish("screen capture request denied")
        .unwrap();
    assert_eq!(snapshot.media, MediaState::Idle);
    assert_eq!(
        snapshot.last_error.as_deref(),
        Some("screen capture request denied")
    );

    snapshot.begin_publish().unwrap();
    assert_eq!(snapshot.media, MediaState::PreparingPublish);
}

#[test]
fn runtime_publishes_discovery_state_and_shuts_down() {
    let runtime = RuntimeHandle::start().unwrap();
    runtime.try_send(UserCommand::StartDiscovery).unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    while runtime.snapshot().discovery != DiscoveryState::Scanning {
        assert!(
            Instant::now() < deadline,
            "runtime did not publish its state"
        );
        thread::sleep(Duration::from_millis(5));
    }

    drop(runtime);
    drop(RuntimeHandle::start().unwrap());
}

#[test]
fn discovery_lost_does_not_overwrite_transport_state() {
    let mut snapshot = connected_snapshot();
    snapshot.start_discovery();
    snapshot.mark_peer_lost("android-living-room");

    assert_eq!(snapshot.discovery, DiscoveryState::Empty);
    assert_eq!(
        snapshot.peers["android-living-room"].discovery,
        PeerDiscoveryState::Lost
    );
    assert_eq!(
        snapshot.peers["android-living-room"].transport,
        TransportState::Connected
    );
}

#[test]
fn stop_and_failure_retain_peer_history_but_mark_discovery_lost() {
    let mut snapshot = AppSnapshot::default();
    snapshot.start_discovery();
    snapshot.upsert_peer(peer("android-living-room"));

    snapshot.stop_discovery();
    assert_eq!(snapshot.discovery, DiscoveryState::Idle);
    assert_eq!(
        snapshot.peers["android-living-room"].discovery,
        PeerDiscoveryState::Lost
    );

    snapshot.start_discovery();
    snapshot.upsert_peer(peer("android-living-room"));
    snapshot.fail_discovery("listener stopped");
    assert_eq!(snapshot.discovery, DiscoveryState::Error);
    assert_eq!(
        snapshot.peers["android-living-room"].discovery,
        PeerDiscoveryState::Lost
    );
}

#[test]
fn remote_screen_directory_tracks_available_withdrawn_and_returned() {
    let mut snapshot = connected_snapshot();
    let path = "moqcast.screen/android-living-room";

    assert!(snapshot.update_remote_screen(path.into(), true));
    assert_eq!(
        snapshot.remote_screens[path].availability,
        ScreenAvailability::Available
    );
    assert_eq!(
        snapshot.peers["android-living-room"].screen,
        ScreenAvailability::Available
    );

    assert!(snapshot.update_remote_screen(path.into(), false));
    assert_eq!(
        snapshot.remote_screens[path].availability,
        ScreenAvailability::Withdrawn
    );

    assert!(snapshot.update_remote_screen(path.into(), true));
    assert_eq!(
        snapshot.remote_screens[path].availability,
        ScreenAvailability::Available
    );
    assert_eq!(snapshot.media, MediaState::Idle);
}

#[test]
fn remote_view_can_be_stopped_while_catalog_is_preparing() {
    let mut snapshot = connected_snapshot();
    let path = "moqcast.screen/android-living-room";
    snapshot.update_remote_screen(path.into(), true);

    snapshot.begin_view(path).unwrap();
    snapshot.begin_stop_view().unwrap();
    snapshot.finish_stop_view().unwrap();

    assert_eq!(snapshot.media, MediaState::Idle);
}

#[test]
fn announcement_before_discovery_is_projected_when_peer_arrives() {
    let mut snapshot = AppSnapshot::default();
    snapshot.update_remote_screen("moqcast.screen/android-living-room".into(), true);

    snapshot.upsert_peer(peer("android-living-room"));

    assert_eq!(
        snapshot.peers["android-living-room"].screen,
        ScreenAvailability::Available
    );
}

#[test]
fn local_and_malformed_screen_paths_do_not_enter_remote_directory() {
    let mut snapshot = AppSnapshot {
        local_peer_id: Some("linux-local".into()),
        ..Default::default()
    };

    assert!(!snapshot.update_remote_screen("moqcast.screen/linux-local".into(), true));
    assert!(!snapshot.update_remote_screen("moqcast.screen/peer/extra".into(), true));
    assert!(snapshot.remote_screens.is_empty());
}

#[test]
fn three_peer_transport_updates_are_independent() {
    let mut snapshot = AppSnapshot::default();
    snapshot.upsert_peer(peer("peer-a"));
    snapshot.upsert_peer(peer("peer-b"));
    snapshot.upsert_peer(peer("peer-c"));
    snapshot.set_transport("peer-a", TransportState::Connected);
    snapshot.set_transport("peer-b", TransportState::Connected);
    snapshot.set_transport("peer-c", TransportState::Connected);

    snapshot.set_transport("peer-b", TransportState::Failed);

    assert_eq!(
        snapshot.peers["peer-a"].transport,
        TransportState::Connected
    );
    assert_eq!(snapshot.peers["peer-b"].transport, TransportState::Failed);
    assert_eq!(
        snapshot.peers["peer-c"].transport,
        TransportState::Connected
    );
}

#[test]
fn inbound_count_can_keep_media_available_without_claiming_peer_identity() {
    let mut snapshot = AppSnapshot::default();
    snapshot.set_inbound_session_count(3);

    assert!(snapshot.has_mesh_session());
    assert!(snapshot.peers.is_empty());
    snapshot.begin_publish().unwrap();
}
