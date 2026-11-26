use super::*;

#[tokio::test]
async fn pending_watch_is_scoped_to_one_connection_and_one_media_generation() {
    tokio::time::timeout(std::time::Duration::from_secs(12), exercise())
        .await
        .unwrap();
}

async fn exercise() {
    let (address, pin, server) = relay::test_server::listen().await;
    let config = relay::Config::parse(&format!("https://{address}/"), &pin).unwrap();
    let mut publisher = relay::Owner::start(config.clone()).unwrap();
    assert!(matches!(publisher.recv().await, relay::Event::Connected));
    let source = publisher
        .publish_origin()
        .create_broadcast("fixture")
        .unwrap();
    source.announce(Default::default()).unwrap();

    let mut snapshot = RuntimeSnapshot::default();
    let mut link = relay::Link::default();
    let mut services = ServiceOwner::default();
    let mut publication = PublicationOwner::default();
    let mut view = ViewOwner::default();
    let (snapshots, _) = watch::channel(snapshot.clone());
    let (media_events, _) = mpsc::channel(4);
    let (view_events, _) = mpsc::channel(4);
    let playback = watch::channel(None).0;
    let audio_updates = watch::channel(None).0;
    let runtime = RuntimeConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        node: None,
        secret_file: None,
    };
    macro_rules! send {
        ($command:expr) => {{
            let cmd = $command;
            assert!(
                !handle_command(
                    cmd,
                    RuntimeContext {
                        config: &runtime,
                        snapshots: &snapshots,
                        snapshot: &mut snapshot,
                        relay: &mut link,
                        services: &mut services,
                        publication: &mut publication,
                        view: &mut view,
                        media_events: &media_events,
                        view_events: &view_events,
                        playback: &playback,
                        audio_updates: &audio_updates
                    }
                )
                .await
            );
        }};
    }
    send!(RuntimeCommand::Relay(relay::Command::Connect(
        config.clone()
    )));
    while !snapshot.relay.entries.contains_key("fixture") {
        assert!(matches!(
            link.next(&mut snapshot.relay).await,
            relay::Update::Directory
        ));
    }
    let connection = snapshot.relay.lifecycle.generation();
    let selection = relay::Selection {
        generation: connection,
        path: "fixture".into(),
        revision: snapshot.relay.entries["fixture"],
    };
    send!(RuntimeCommand::Relay(relay::Command::Watch(
        selection.clone()
    )));
    let first = snapshot.route.clone();
    assert_eq!(snapshot.view.phase, ViewPhase::Preparing);
    send!(RuntimeCommand::StopScan);
    assert_eq!(snapshot.route, first);
    assert_eq!(snapshot.view.phase, ViewPhase::Preparing);
    assert_eq!(snapshot.relay.lifecycle.phase(), relay::Phase::Connected);
    send!(RuntimeCommand::Relay(relay::Command::Stop(first.clone())));
    assert_eq!(snapshot.view.phase, ViewPhase::Idle);
    send!(RuntimeCommand::Relay(relay::Command::Watch(selection)));
    assert_ne!(first, snapshot.route);
    send!(RuntimeCommand::Relay(relay::Command::Stop(first)));
    send!(RuntimeCommand::StopWatching(snapshot.view.generation));
    assert_eq!(snapshot.view.phase, ViewPhase::Preparing);
    let mut found = false;
    while !found {
        match link.next(&mut snapshot.relay).await {
            relay::Update::Broadcast(Ok(_)) => found = true,
            relay::Update::Directory => {}
            _ => panic!("pinned HTTPS relay request failed"),
        }
    }
    // Release physical work on withdrawal but retain the logical slot and generation.
    let active = snapshot.route.clone();
    let active_generation = snapshot.view.generation;
    let (cancel, mut cancelled) = watch::channel(false);
    let (released, was_released) = oneshot::channel();
    view.cancel = Some(cancel);
    view.task = Some(tokio::spawn(async move {
        cancelled.changed().await.unwrap();
        released.send(()).unwrap();
    }));
    source.close();
    while snapshot.relay.entries.contains_key("fixture") {
        let update = link.next(&mut snapshot.relay).await;
        handle_relay_update(
            update,
            &mut snapshot,
            &mut link,
            &mut publication,
            &mut view,
            &playback,
            &view_events,
        )
        .await;
    }
    was_released.await.unwrap();
    assert!(view.task.is_none());
    assert_eq!(snapshot.view.phase, ViewPhase::Failed);
    assert_eq!(snapshot.view.path.as_deref(), Some("fixture"));
    assert_eq!(snapshot.route, active);
    assert_eq!(snapshot.view.generation, active_generation);
    assert!(!snapshot.access().media_idle);
    let reopened = publisher
        .publish_origin()
        .create_broadcast("fixture")
        .unwrap();
    reopened.announce(Default::default()).unwrap();
    while !snapshot.relay.entries.contains_key("fixture") {
        let update = link.next(&mut snapshot.relay).await;
        handle_relay_update(
            update,
            &mut snapshot,
            &mut link,
            &mut publication,
            &mut view,
            &playback,
            &view_events,
        )
        .await;
    }
    let next = relay::Selection {
        generation: connection,
        revision: snapshot.relay.entries["fixture"],
        path: "fixture".into(),
    };
    send!(RuntimeCommand::Relay(relay::Command::Watch(next.clone())));
    assert_eq!(snapshot.route, active);
    send!(RuntimeCommand::Relay(relay::Command::Stop(active.clone())));
    send!(RuntimeCommand::Relay(relay::Command::Watch(next)));
    assert_eq!(snapshot.view.phase, ViewPhase::Preparing);
    send!(RuntimeCommand::Relay(relay::Command::Stop(active)));
    assert_eq!(snapshot.view.phase, ViewPhase::Preparing);
    server.close_connections();
    while snapshot.relay.lifecycle.phase() == relay::Phase::Connected {
        let update = link.next(&mut snapshot.relay).await;
        handle_relay_update(
            update,
            &mut snapshot,
            &mut link,
            &mut publication,
            &mut view,
            &playback,
            &view_events,
        )
        .await;
    }
    assert_eq!(snapshot.view.phase, ViewPhase::Idle);
    assert!(snapshot.view.path.is_none());
    assert_eq!(snapshot.relay.lifecycle.phase(), relay::Phase::Failed);
    send!(RuntimeCommand::Relay(relay::Command::Connect(config)));
    while snapshot.relay.lifecycle.phase() != relay::Phase::Connected {
        let update = link.next(&mut snapshot.relay).await;
        handle_relay_update(
            update,
            &mut snapshot,
            &mut link,
            &mut publication,
            &mut view,
            &playback,
            &view_events,
        )
        .await;
    }
    // Relay disconnect must not cancel a Nearby watch, even with the same path.
    snapshot.route = MediaRoute::Nearby;
    let nearby = snapshot.view.begin("fixture").unwrap();
    let (cancel, cancelled) = watch::channel(false);
    view.cancel = Some(cancel);
    let connection = snapshot.relay.lifecycle.generation();
    send!(RuntimeCommand::Relay(relay::Command::Disconnect(
        connection
    )));
    send!(RuntimeCommand::StopWatching(active_generation));
    assert_eq!(snapshot.view.phase, ViewPhase::Preparing);
    assert!(!*cancelled.borrow());
    send!(RuntimeCommand::StopWatching(nearby));
    assert!(*cancelled.borrow());
    let old_share = snapshot.media.begin("local").unwrap();
    send!(RuntimeCommand::StopSharing(old_share));
    let new_share = snapshot.media.begin("local").unwrap();
    send!(RuntimeCommand::StopSharing(old_share));
    assert_eq!(snapshot.media.phase, crate::media::MediaPhase::Preparing);
    send!(RuntimeCommand::StopSharing(new_share));
    assert_eq!(snapshot.media.phase, crate::media::MediaPhase::Idle);
    assert_eq!(snapshot.relay.lifecycle.phase(), relay::Phase::Disconnected);
    reopened.close();
    publisher.close().await.unwrap();
    server.abort();
}
