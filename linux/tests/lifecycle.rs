use std::thread;
use std::time::{Duration, Instant};

use moq_cast_desktop::app::{
    AppSnapshot, DiscoveryState, PeerState, PublishState, StateError, UserCommand,
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
