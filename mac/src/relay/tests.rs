use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

async fn roundtrip(config: Config) {
    let publish = moq_tokio::origin::spawn();
    let client = config
        .client(moq_tokio::origin::spawn())
        .unwrap()
        .with_publisher(publish.consume());
    let connection = client
        .connect(config.url.clone())
        .established()
        .await
        .unwrap();
    let mut viewer = Owner::start(config).unwrap();
    assert!(matches!(viewer.recv().await, Event::Connected));
    eprintln!(
        "publisher version={} viewer version={}",
        connection.version().unwrap(),
        viewer.version().unwrap()
    );
    let name = format!(
        "probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let broadcast = publish.create_broadcast(&name).unwrap();
    let track = broadcast.create_track("probe", None).unwrap();
    broadcast.announce(Default::default()).unwrap();
    let mut snapshot = Snapshot::default();
    snapshot.lifecycle.begin(Phase::Connected);
    loop {
        match viewer.recv().await {
            Event::Announcement(event) => viewer.apply(&mut snapshot, event),
            Event::Closed(error) => panic!("relay closed: {error}"),
            Event::Connected => panic!("duplicate established"),
        }
        if snapshot.entries.contains_key(&name) {
            break;
        }
    }
    let selection = Selection {
        generation: snapshot.lifecycle.generation(),
        path: name.clone(),
        revision: snapshot.entries[&name],
    };
    eprintln!("received exact relative announcement={name}");
    let received = viewer.request(&selection).unwrap().await.unwrap();
    let mut subscription = received
        .track("probe")
        .unwrap()
        .subscribe(moq_net::track::Subscription::default())
        .await
        .unwrap();
    track.demand().used().await.unwrap();
    let mut group = track.append_group().unwrap();
    group
        .write_frame(moq_net::Timestamp::ZERO, b"relay-byte-roundtrip")
        .unwrap();
    group.finish().unwrap();
    let mut group = subscription.recv_group().await.unwrap().unwrap();
    let frame = group.read_frame().await.unwrap().unwrap();
    assert_eq!(frame.payload.as_ref(), b"relay-byte-roundtrip");
    broadcast.unannounce();
    while snapshot.entries.contains_key(&name) {
        match viewer.recv().await {
            Event::Announcement(event) => viewer.apply(&mut snapshot, event),
            _ => panic!("connection ended before withdrawal"),
        }
    }
    assert!(viewer.request(&selection).is_none());
    drop(group);
    drop(subscription);
    drop(received);
    broadcast.close();
    drop(track);
    drop(broadcast);
    verify_video(&publish, &mut viewer, &mut snapshot).await;
    viewer.close().await.unwrap();
    connection.close().await.unwrap();
    client.close().await;
    eprintln!("announcement, exact request, payload, withdrawal and close passed");
}

#[tokio::test]
async fn pinned_loopback_relay_roundtrip_and_cancel() {
    let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut config = moq_tokio::listen::Config::default();
    config.bind = Some(moq_tokio::listen::Bind::Addr(
        "127.0.0.1:0".parse().unwrap(),
    ));
    config.tls.generate = vec!["localhost".into()];
    let server = config.init(moq_tokio::quic::Config::default()).unwrap();
    let address = server.local_addr().unwrap();
    let pin = server.certificates().fingerprints()[0].clone();
    let mut listener = server.listen().await.unwrap();
    let origin = moq_tokio::origin::spawn();
    let server = tokio::spawn(async move {
        let mut sessions = tokio::task::JoinSet::new();
        while let Some(request) = listener.accept().await {
            let origin = origin.clone();
            sessions.spawn(async move {
                if let Ok(session) = request
                    .with_publisher(&origin)
                    .with_subscriber(origin.clone())
                    .ok()
                    .await
                {
                    let _ = session.closed().await;
                }
            });
        }
    });
    for scheme in ["moqt", "https"] {
        let config = Config::parse(&format!("{scheme}://{address}/"), &pin).unwrap();
        tokio::time::timeout(Duration::from_secs(15), roundtrip(config.clone()))
            .await
            .unwrap();
        let cancelled = Owner::start(config).unwrap();
        // Close before observing established, including the race where TLS has already won.
        tokio::time::timeout(Duration::from_secs(5), cancelled.close())
            .await
            .unwrap()
            .unwrap();
        let mut rejected = Owner::start(
            Config::parse(&format!("{scheme}://{address}/"), &"00".repeat(32)).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), rejected.recv())
                .await
                .unwrap(),
            Event::Closed(_)
        ));
        let _ = rejected.close().await;
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
#[ignore = "requires the explicitly configured local Docker relay"]
async fn docker_relay_roundtrip() {
    let url = std::env::var("MOQCAST_TEST_RELAY_URL").expect("set MOQCAST_TEST_RELAY_URL");
    let pin = std::env::var("MOQCAST_TEST_RELAY_PIN").expect("set MOQCAST_TEST_RELAY_PIN");
    let config = Config::parse(&url, &pin).unwrap();
    tokio::time::timeout(Duration::from_secs(20), roundtrip(config))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a local Docker relay allowing only the test/ anonymous scope"]
async fn docker_relay_rejects_wrong_pin_and_unauthorized_scope() {
    let url = std::env::var("MOQCAST_TEST_RELAY_URL").expect("set MOQCAST_TEST_RELAY_URL");
    let pin = std::env::var("MOQCAST_TEST_RELAY_PIN").expect("set MOQCAST_TEST_RELAY_PIN");
    let mut wrong_pin = Owner::start(Config::parse(&url, &"00".repeat(32)).unwrap()).unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), wrong_pin.recv())
            .await
            .unwrap(),
        Event::Closed(_)
    ));
    let _ = wrong_pin.close().await;
    eprintln!("wrong certificate pin rejected");

    let mut config = Config::parse(&url, &pin).unwrap();
    config.url.set_path("/private-probe/");
    config.url.set_query(None);
    let client = config.client(moq_tokio::origin::spawn()).unwrap();
    let connection = client.connect(config.url);
    let result = tokio::time::timeout(Duration::from_secs(5), connection.clone().established())
        .await
        .unwrap();
    // QUIC can expose the negotiated session before the server's admission closes it.
    let error = match result {
        Ok(_) => tokio::time::timeout(Duration::from_secs(5), connection.closed())
            .await
            .expect("unauthorized session remained open")
            .expect_err("unauthorized session ended without a refusal"),
        Err(error) => error,
    };
    assert!(
        error.is_auth(),
        "expected authorization rejection: {error:?}"
    );
    eprintln!("anonymous scope outside test/ rejected: {error:?}");
    let _ = connection.close().await;
    client.close().await;
}

async fn verify_video(
    publish: &moq_net::origin::Producer,
    viewer: &mut Owner,
    snapshot: &mut Snapshot,
) {
    let name = format!(
        "video-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut broadcast = publish.create_broadcast(&name).unwrap();
    let clock = moq_mux::Clock::new();
    let catalog = moq_mux::catalog::Producer::<()>::new(
        &mut broadcast,
        moq_mux::catalog::Config::default().with_clock(clock.clone()),
    )
    .unwrap();
    let size = moq_video::Size::new(160, 96);
    let mut config = moq_video::encode::Config::new(
        size.width,
        size.height,
        moq_video::Rate::new(30, 1).unwrap(),
    );
    config.kind = moq_video::encode::Kind::Software;
    let rendition = config.probe().await.unwrap();
    let mut encoder = moq_video::encode::Encoder::new(&config).unwrap();
    let mut video =
        moq_video::encode::Producer::new(broadcast.clone(), catalog, rendition).unwrap();
    broadcast.announce(Default::default()).unwrap();
    while !snapshot.entries.contains_key(&name) {
        match viewer.recv().await {
            Event::Announcement(event) => viewer.apply(snapshot, event),
            _ => panic!("relay ended before video announcement"),
        }
    }
    let selection = Selection {
        generation: snapshot.lifecycle.generation(),
        path: name.clone(),
        revision: snapshot.entries[&name],
    };
    let received = viewer.request(&selection).unwrap().await.unwrap();
    let (events, mut event_rx) = tokio::sync::mpsc::channel(8);
    let (frames, mut frame_rx) = tokio::sync::watch::channel(None);
    let mut playback = crate::playback::Owner::default();
    playback.start(
        1,
        name,
        received,
        events,
        frames,
        std::sync::Arc::new(|| {}),
    );
    video.demand().used().await.unwrap();
    for index in 0..8u8 {
        let rgba: Vec<u8> = [40 + index * 8, 120, 200 - index * 8, 255]
            .into_iter()
            .cycle()
            .take(size.pixels() as usize * 4)
            .collect();
        let frame =
            moq_video::Frame::new(moq_video::Surface::rgba(&rgba, size).unwrap(), clock.now());
        encoder.cut().unwrap();
        video.publish(&encoder.encode(&frame).unwrap()).unwrap();
    }
    video.publish(&encoder.finish().unwrap()).unwrap();
    video.finish().unwrap();
    loop {
        tokio::select! {
            result = frame_rx.changed() => {
                result.unwrap();
                if let Some(frame) = frame_rx.borrow_and_update().as_ref() {
                    assert_eq!((frame.display_width,frame.display_height),(160,96));
                    eprintln!("H.264 catalog and production playback delivered {}x{} frame; generation={}", frame.display_width,frame.display_height,frame.identity.view_generation);
                    break;
                }
            }
            event = event_rx.recv() => match event.unwrap() {
                crate::playback::Event::Ended { result, .. } => panic!("playback ended before frame: {result:?}"),
                crate::playback::Event::VideoChanged { decoder,codec,.. } => eprintln!("decoder={decoder} codec={codec}"),
                _ => {}
            }
        }
    }
    playback.stop().await.unwrap();
    drop(frame_rx);
    broadcast.close();
    drop(video);
    drop(broadcast);
}

#[tokio::test]
#[ignore = "manual GUI fixture; publishes synthetic H.264 for five minutes"]
async fn manual_gui_video_fixture() {
    let url = std::env::var("MOQCAST_TEST_RELAY_URL").expect("set MOQCAST_TEST_RELAY_URL");
    let pin = std::env::var("MOQCAST_TEST_RELAY_PIN").expect("set MOQCAST_TEST_RELAY_PIN");
    let config = Config::parse(&url, &pin).unwrap();
    let publish = moq_tokio::origin::spawn();
    let client = config
        .client(moq_tokio::origin::spawn())
        .unwrap()
        .with_publisher(publish.consume());
    let connection = client.connect(config.url).established().await.unwrap();
    let name = format!(
        "desktop-colors-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut broadcast = publish.create_broadcast(&name).unwrap();
    let clock = moq_mux::Clock::new();
    let catalog = moq_mux::catalog::Producer::<()>::new(
        &mut broadcast,
        moq_mux::catalog::Config::default().with_clock(clock.clone()),
    )
    .unwrap();
    let size = moq_video::Size::new(640, 360);
    let mut config = moq_video::encode::Config::new(
        size.width,
        size.height,
        moq_video::Rate::new(30, 1).unwrap(),
    );
    config.kind = moq_video::encode::Kind::Software;
    let rendition = config.probe().await.unwrap();
    let mut encoder = moq_video::encode::Encoder::new(&config).unwrap();
    let mut video =
        moq_video::encode::Producer::new(broadcast.clone(), catalog, rendition).unwrap();
    broadcast.announce(Default::default()).unwrap();
    eprintln!(
        "GUI fixture relative path={name}; version={}",
        connection.version().unwrap()
    );
    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut index = 0u64;
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            result = connection.closed() => panic!("fixture relay closed: {result:?}"),
            _ = tick.tick() => {
                let color = ((index / 30) % 3) as usize;
                let palette = [[220, 60, 60, 255], [60, 220, 60, 255], [60, 60, 220, 255]];
                let rgba: Vec<u8> = palette[color].into_iter().cycle().take(size.pixels() as usize * 4).collect();
                let frame = moq_video::Frame::new(moq_video::Surface::rgba(&rgba, size).unwrap(), clock.now());
                if index.is_multiple_of(30) { encoder.cut().unwrap(); }
                video.publish(&encoder.encode(&frame).unwrap()).unwrap();
                index += 1;
            }
        }
    }
    video.publish(&encoder.finish().unwrap()).unwrap();
    video.finish().unwrap();
    broadcast.unannounce();
    broadcast.close();
    drop(video);
    drop(broadcast);
    connection.close().await.unwrap();
    client.close().await;
}
