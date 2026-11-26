//! Opt-in native capture test. Run inside a disposable Xvfb display with xsetroot.
use super::*;
use crate::{
    app::SourceCatalog,
    relay::{Command, Config, Phase, PublishPrefix, Selection},
};
use std::time::Duration;

struct Desktop {
    tx: mpsc::Sender<UserCommand>,
    snapshot: watch::Receiver<Arc<AppSnapshot>>,
    frames: watch::Receiver<Option<Arc<PlaybackFrame>>>,
    task: JoinHandle<()>,
}

impl Desktop {
    fn start() -> Self {
        let (tx, rx) = mpsc::channel(16);
        let (snapshots, snapshot) = watch::channel(Arc::new(AppSnapshot::default()));
        let (playback, frames) = watch::channel(None);
        let task = tokio::spawn(Supervisor::new(playback, Default::default()).run(rx, snapshots));
        Self {
            tx,
            snapshot,
            frames,
            task,
        }
    }

    async fn until(&mut self, predicate: impl Fn(&AppSnapshot) -> bool) -> Arc<AppSnapshot> {
        let latest = self.snapshot.clone();
        match tokio::time::timeout(
            Duration::from_secs(18),
            self.snapshot.wait_for(|s| predicate(s)),
        )
        .await
        {
            Ok(Ok(value)) => value.clone(),
            _ => panic!(
                "runtime did not reach expected state: {:?}",
                latest.borrow()
            ),
        }
    }
    async fn decoded(&mut self, generation: u64, rgb: [u8; 3]) -> Arc<PlaybackFrame> {
        tokio::time::timeout(
            Duration::from_secs(25),
            self.frames.wait_for(|frame| {
                frame.as_ref().is_some_and(|f| {
                    f.identity.view_generation == generation
                        && (f.width, f.height) == (640, 360)
                        && f.rgba
                            .chunks_exact(4)
                            .nth(10 * 640 + 10)
                            .is_some_and(|pixel| {
                                pixel
                                    .iter()
                                    .zip(rgb)
                                    .all(|(actual, expected)| actual.abs_diff(expected) < 20)
                            })
                })
            }),
        )
        .await
        .expect("fixture color did not decode")
        .unwrap()
        .clone()
        .unwrap()
    }
}

fn paint(color: &str) {
    assert!(
        std::process::Command::new("xsetroot")
            .args(["-solid", color])
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a dedicated 640x360 Xvfb display, xsetroot and video codecs"]
async fn native_relay_publish_watch_restart() {
    assert_eq!(
        std::env::var("MOQCAST_TEST_XVFB").unwrap(),
        std::env::var("DISPLAY").unwrap()
    );
    let (address, pin, server) = relay::test_server::listen().await;
    let url =
        std::env::var("MOQCAST_TEST_RELAY_URL").unwrap_or_else(|_| format!("moqt://{address}/"));
    let pin = std::env::var("MOQCAST_TEST_RELAY_PIN").unwrap_or(pin);
    let config = Config::parse(&url, &pin).unwrap();
    let mut publisher = Desktop::start();
    let mut viewer = Desktop::start();
    for desktop in [&mut publisher, &mut viewer] {
        desktop
            .tx
            .send(UserCommand::Relay(Command::Connect(config.clone())))
            .await
            .unwrap();
        let connected = desktop
            .until(|s| s.relay.lifecycle.phase() == Phase::Connected)
            .await;
        eprintln!(
            "connected with {}",
            connected.relay.version.as_ref().unwrap()
        );
    }
    publisher
        .tx
        .send(UserCommand::RefreshCaptureSources)
        .await
        .unwrap();
    let state = publisher
        .until(|s| matches!(s.sources, SourceCatalog::Ready(_)))
        .await;
    let SourceCatalog::Ready(sources) = &state.sources else {
        unreachable!()
    };
    assert_eq!(sources.len(), 1);
    let source = sources[0].clone();
    let mut generation = state.relay.lifecycle.generation();
    let mut previous = None;
    let mut previous_watch = None;
    for (color, rgb) in [("red", [255, 0, 0]), ("blue", [0, 0, 255])] {
        paint(color);
        publisher
            .tx
            .send(UserCommand::StartRelayShare {
                connection: generation,
                prefix: PublishPrefix::parse("fixture").unwrap(),
                system_audio: false,
                source: source.clone(),
            })
            .await
            .unwrap();
        let state = publisher.until(|s| s.media == MediaState::Publishing).await;
        let route = state.route.clone();
        let MediaRoute::Publishing { path, .. } = &route else {
            panic!("missing publication")
        };
        let state = viewer.until(|s| s.relay.entries.contains_key(path)).await;
        if let Some(old_watch) = previous_watch.take() {
            assert!(matches!(state.media, MediaState::FailedView { .. }));
            assert_eq!(state.route, old_watch);
            let request = state.relay.watch_request;
            viewer
                .tx
                .send(UserCommand::Relay(Command::Watch(Selection {
                    generation: state.relay.lifecycle.generation(),
                    path: path.clone(),
                    revision: state.relay.entries[path],
                })))
                .await
                .unwrap();
            let rejected = viewer.until(|s| s.relay.watch_request > request).await;
            assert_eq!(rejected.route, old_watch);
            viewer
                .tx
                .send(UserCommand::Relay(Command::Stop(old_watch)))
                .await
                .unwrap();
            viewer.until(|s| s.media == MediaState::Idle).await;
        } else {
            assert_eq!(state.media, MediaState::Idle);
        }
        let selection = Selection {
            generation: state.relay.lifecycle.generation(),
            path: path.clone(),
            revision: state.relay.entries[path],
        };
        viewer
            .tx
            .send(UserCommand::Relay(Command::Watch(selection)))
            .await
            .unwrap();
        let state = viewer
            .until(|s| matches!(s.media, MediaState::Viewing { .. }))
            .await;
        viewer.decoded(state.view_generation, rgb).await;
        previous_watch = Some(state.route.clone());
        if let Some(old) = previous.take() {
            assert_ne!(route, old);
            publisher
                .tx
                .send(UserCommand::Relay(Command::Stop(old)))
                .await
                .unwrap();
        }
        for desktop in [&mut publisher, &mut viewer] {
            desktop.tx.send(UserCommand::StartDiscovery).await.unwrap();
            desktop
                .until(|s| s.local_peer_id.is_some() && !s.lan_stopping)
                .await;
            desktop.tx.send(UserCommand::StopDiscovery).await.unwrap();
        }
        for desktop in [&mut publisher, &mut viewer] {
            desktop
                .until(|s| {
                    s.discovery == crate::app::DiscoveryState::Idle
                        && !s.lan_stopping
                        && s.relay.lifecycle.phase() == Phase::Connected
                        && s.media != MediaState::Idle
                })
                .await;
        }
        let current = state.view_generation;
        // This distinct color is set only after both LAN owners report Off.
        // A buffered frame from before shutdown cannot satisfy this assertion.
        let (pulse, rgb) = if color == "red" {
            ("#e04020", [224, 64, 32])
        } else {
            ("#2040e0", [32, 64, 224])
        };
        paint(pulse);
        let frame = viewer.decoded(current, rgb).await;
        eprintln!(
            "decoded {color} after Nearby Off ({pulse}) {}x{} generation={} path={path}",
            frame.width, frame.height, current
        );
        publisher
            .tx
            .send(UserCommand::Relay(Command::Stop(route.clone())))
            .await
            .unwrap();
        viewer
            .until(|s| {
                !s.relay.entries.contains_key(path)
                    && matches!(s.media, MediaState::FailedView { .. })
            })
            .await;
        assert!(viewer.frames.borrow().is_none());
        assert_eq!(viewer.snapshot.borrow().view_generation, current);
        previous = Some(route);
        if color == "red" {
            publisher
                .tx
                .send(UserCommand::Relay(Command::Disconnect(generation)))
                .await
                .unwrap();
            publisher
                .until(|s| {
                    s.relay.lifecycle.phase() == Phase::Disconnected && s.media == MediaState::Idle
                })
                .await;
            publisher
                .tx
                .send(UserCommand::Relay(Command::Connect(config.clone())))
                .await
                .unwrap();
            let new = publisher
                .until(|s| s.relay.lifecycle.phase() == Phase::Connected)
                .await;
            assert_ne!(new.relay.lifecycle.generation(), generation);
            publisher
                .tx
                .send(UserCommand::Relay(Command::Disconnect(generation)))
                .await
                .unwrap();
            generation = new.relay.lifecycle.generation();
        }
    }
    for desktop in [publisher, viewer] {
        desktop.tx.send(UserCommand::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), desktop.task)
            .await
            .unwrap()
            .unwrap();
    }
    server.abort();
}

#[tokio::test]
async fn terminal_watch_retains_the_slot_but_releases_playback_before_return() {
    tokio::time::timeout(Duration::from_secs(12), watch_owner_recovery())
        .await
        .unwrap();
}

async fn watch_owner_recovery() {
    let frames = watch::channel(None).0;
    let mut owner = Supervisor::new(frames.clone(), Default::default());
    let (address, pin, server) = relay::test_server::listen().await;
    let config = Config::parse(&format!("https://{address}/"), &pin).unwrap();
    owner.relay_command(Command::Connect(config.clone())).await;
    let update = owner.relay.next(&mut owner.state.relay).await;
    owner.relay_event(update).await;
    let connection = owner.state.relay.lifecycle.generation();
    let mut source = relay::Owner::start(config.clone()).unwrap();
    assert!(matches!(source.recv().await, relay::Event::Connected));
    let first = source.publish_origin().create_broadcast("fixture").unwrap();
    first.announce(Default::default()).unwrap();
    while !owner.state.relay.entries.contains_key("fixture") {
        let update = owner.relay.next(&mut owner.state.relay).await;
        owner.relay_event(update).await;
    }
    let selection = Selection {
        generation: connection,
        path: "fixture".into(),
        revision: owner.state.relay.entries["fixture"],
    };
    owner.relay_command(Command::Watch(selection)).await;
    let original = owner.state.view_generation;
    while owner.view.task.is_none() {
        let update = owner.relay.next(&mut owner.state.relay).await;
        owner.relay_event(update).await;
    }
    // The actual decoder is waiting for the first catalog. Retracting only the
    // announcement keeps the source alive, so the runtime must cancel the reader.
    let volume = owner.view.volume.as_ref().unwrap().clone();
    frames.send_replace(Some(Arc::new(PlaybackFrame {
        identity: crate::runtime::PlaybackFrameIdentity {
            view_generation: original,
            decoder_generation: 1,
            sequence: 1,
        },
        width: 1,
        height: 1,
        rgba: vec![0, 0, 0, 255],
    })));
    first.unannounce();
    while owner.state.relay.entries.contains_key("fixture") {
        let update = owner.relay.next(&mut owner.state.relay).await;
        owner.relay_event(update).await;
    }
    assert!(frames.borrow().is_none());
    assert!(owner.view.task.is_none() && volume.is_closed());
    assert_eq!(owner.state.view_generation, original);
    assert!(matches!(&owner.state.media, MediaState::FailedView { path } if path == "fixture"));
    assert!(!owner.state.access().media_idle);
    let old = owner.state.route.clone();
    first.close();
    let replacement = source.publish_origin().create_broadcast("fixture").unwrap();
    replacement.announce(Default::default()).unwrap();
    while !owner.state.relay.entries.contains_key("fixture") {
        let update = owner.relay.next(&mut owner.state.relay).await;
        owner.relay_event(update).await;
    }
    let next = Selection {
        generation: connection,
        path: "fixture".into(),
        revision: owner.state.relay.entries["fixture"],
    };
    owner.relay_command(Command::Watch(next.clone())).await;
    assert!(owner.view.task.is_none());
    assert_eq!(owner.state.route, old);
    owner.relay_command(Command::Stop(old.clone())).await;
    owner.relay_command(Command::Watch(next)).await;
    assert_ne!(owner.state.view_generation, original);
    owner.relay_command(Command::Stop(old.clone())).await;
    owner
        .handle_operation_event(OperationEvent::ViewEnded {
            generation: original,
            result: Err("late".into()),
        })
        .await;
    assert!(matches!(
        owner.state.media,
        MediaState::PreparingView { .. }
    ));
    server.close_connections();
    tokio::time::timeout(Duration::from_secs(5), async {
        while owner.state.relay.lifecycle.phase() == Phase::Connected {
            let update = owner.relay.next(&mut owner.state.relay).await;
            owner.relay_event(update).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(owner.state.relay.lifecycle.phase(), Phase::Failed);
    assert_eq!(owner.state.media, MediaState::Idle);
    owner.relay_command(Command::Connect(config)).await;
    let update = owner.relay.next(&mut owner.state.relay).await;
    owner.relay_event(update).await;
    let connection = owner.state.relay.lifecycle.generation();
    // A reconnect/late withdrawal cannot release an unrelated Nearby owner.
    owner.state.prepare_view("fixture");
    owner.state.view_generation = owner.view.advance();
    owner.state.route = MediaRoute::Nearby;
    let (nearby, still_live) = watch::channel(false);
    owner.view.cancel = Some(nearby);
    owner.relay_command(Command::Stop(old)).await;
    owner.relay_command(Command::Disconnect(connection)).await;
    owner
        .relay_event(relay::Update::Closed("other transport".into()))
        .await;
    assert_eq!(
        owner.state.media,
        MediaState::PreparingView {
            path: "fixture".into()
        }
    );
    assert!(!*still_live.borrow());
    owner.stop_view().await;
    assert!(*still_live.borrow());
    replacement.close();
    let _ = source.close().await;
    server.abort();
}
