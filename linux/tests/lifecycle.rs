use std::thread;
use std::time::{Duration, Instant};

use moq_cast_desktop::app::{
    AppSnapshot, DiscoveredPeer, DiscoveryState, PeerState, PublishState, StateError, UserCommand,
};
use moq_cast_desktop::runtime::RuntimeHandle;

#[test]
fn publishing_requires_a_connected_peer() {
    let mut snapshot = AppSnapshot::default();

    assert_eq!(snapshot.begin_publish(), Err(StateError::PeerNotConnected));
    assert_eq!(snapshot.publish, PublishState::Idle);
}

#[test]
fn duplicate_publish_and_stop_are_rejected() {
    let mut snapshot = AppSnapshot {
        peer: PeerState::Connected {
            peer_id: "android-living-room".into(),
        },
        ..Default::default()
    };

    snapshot.begin_publish().unwrap();
    snapshot.finish_publish().unwrap();
    assert_eq!(
        snapshot.begin_publish(),
        Err(StateError::PublishAlreadyActive)
    );

    snapshot.begin_stop_publish().unwrap();
    assert_eq!(
        snapshot.begin_stop_publish(),
        Err(StateError::PublishAlreadyStopping)
    );
    snapshot.finish_stop_publish().unwrap();
    assert_eq!(snapshot.publish, PublishState::Idle);
}

#[test]
fn disconnect_resets_publish_before_peer_state() {
    let mut snapshot = AppSnapshot {
        peer: PeerState::Connected {
            peer_id: "android-living-room".into(),
        },
        publish: PublishState::Publishing,
        ..Default::default()
    };

    snapshot.disconnect();

    assert_eq!(snapshot.publish, PublishState::Idle);
    assert_eq!(snapshot.peer, PeerState::Disconnected);
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
fn discovery_empty_found_and_lost_do_not_change_the_peer_session() {
    let mut snapshot = AppSnapshot::default();
    snapshot.start_discovery();
    snapshot.finish_initial_scan();
    assert_eq!(snapshot.discovery, DiscoveryState::Empty);

    snapshot.replace_peers(vec![DiscoveredPeer {
        id: "android-living-room".into(),
        name: "Android living room".into(),
        endpoints: vec!["192.0.2.10:4443".into(), "[2001:db8::10]:4443".into()],
        fingerprint_pinned: true,
    }]);
    assert_eq!(snapshot.discovery, DiscoveryState::Ready);
    assert_eq!(snapshot.peer, PeerState::Disconnected);

    snapshot.replace_peers(Vec::new());
    assert_eq!(snapshot.discovery, DiscoveryState::Empty);
    assert_eq!(snapshot.peer, PeerState::Disconnected);
}

#[test]
fn stopping_or_failing_discovery_clears_stale_peers() {
    let peer = DiscoveredPeer {
        id: "android-living-room".into(),
        name: "Android living room".into(),
        endpoints: vec!["192.0.2.10:4443".into()],
        fingerprint_pinned: true,
    };
    let mut snapshot = AppSnapshot::default();
    snapshot.start_discovery();
    snapshot.replace_peers(vec![peer.clone()]);

    snapshot.stop_discovery();
    assert_eq!(snapshot.discovery, DiscoveryState::Idle);
    assert!(snapshot.peers.is_empty());

    snapshot.start_discovery();
    snapshot.replace_peers(vec![peer]);
    snapshot.fail_discovery("listener stopped");
    assert_eq!(snapshot.discovery, DiscoveryState::Error);
    assert!(snapshot.peers.is_empty());
}

#[test]
fn a_failed_connection_can_be_retried_and_disconnected() {
    let mut snapshot = AppSnapshot::default();
    snapshot.begin_connect("android-living-room").unwrap();
    snapshot.fail_connect("fingerprint mismatch").unwrap();
    assert_eq!(
        snapshot.peer,
        PeerState::Failed {
            peer_id: "android-living-room".into(),
        }
    );

    snapshot.begin_connect("android-living-room").unwrap();
    snapshot.finish_connect().unwrap();
    snapshot.begin_disconnect().unwrap();
    snapshot.finish_disconnect().unwrap();
    assert_eq!(snapshot.peer, PeerState::Disconnected);
}
