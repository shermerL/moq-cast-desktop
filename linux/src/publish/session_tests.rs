//! Controlled native codec/transport exercise without a desktop or audio device.
use super::*;
use crate::relay::{self, Event, Phase, PublishPrefix, Selection};
use moq_mux::catalog::Stream;
use std::time::Duration;
use tokio::sync::oneshot;

#[tokio::test]
#[ignore = "run in an isolated Linux environment with PIPEWIRE_REMOTE=moqcast-test-absent"]
async fn missing_audio_backend_keeps_decoding_the_same_publication() {
    assert_eq!(
        std::env::var("PIPEWIRE_REMOTE").as_deref(),
        Ok("moqcast-test-absent")
    );
    tokio::time::timeout(Duration::from_secs(40), exercise())
        .await
        .unwrap();
}

async fn exercise() {
    let (addr, pin, server) = relay::test_server::listen().await;
    let address =
        std::env::var("MOQCAST_TEST_RELAY_URL").unwrap_or_else(|_| format!("https://{addr}/"));
    let pin = std::env::var("MOQCAST_TEST_RELAY_PIN").unwrap_or(pin);
    let config = relay::Config::parse(&address, &pin).unwrap();
    let mut publisher = relay::Owner::start(config.clone()).unwrap();
    let mut viewer = relay::Owner::start(config).unwrap();
    for owner in [&mut publisher, &mut viewer] {
        assert!(matches!(owner.recv().await, Event::Connected));
    }
    let mut state = relay::Snapshot::default();
    state.lifecycle.begin(Phase::Connected);
    for rgba in [[220, 30, 15, 255], [15, 30, 220, 255]] {
        let path = PublishPrefix::parse("audio-check")
            .unwrap()
            .broadcast_path();
        let publication = Publication::prepare(
            &publisher.publish_origin(),
            &path,
            Options {
                source: CaptureSource::Portal,
                system_audio: true,
            },
        )
        .unwrap();
        let (failed, failure) = oneshot::channel();
        let (stop, stopped) = oneshot::channel();
        let video = async {
            // Every encoded frame is produced after the actual PipeWire open has failed.
            let reason: String = failure.await.unwrap();
            assert!(!reason.is_empty());
            let size = moq_video::Size::new(160, 96);
            let mut config =
                moq_video::encode::Config::new(160, 96, moq_video::Rate::new(30, 1).unwrap());
            config.kind = moq_video::encode::Kind::Software;
            let rendition = config.probe().await.unwrap();
            let mut producer = moq_video::encode::Producer::new(
                publication.broadcast.clone(),
                publication.catalog.clone(),
                rendition,
            )
            .unwrap();
            let mut encoder = moq_video::encode::Encoder::new(&config).unwrap();
            producer.demand().used().await.unwrap();
            let pixels: Vec<u8> = rgba.repeat(size.pixels() as usize);
            for _ in 0..8 {
                let frame = moq_video::Frame::new(
                    moq_video::Surface::rgba(&pixels, size).unwrap(),
                    publication.clock.now(),
                );
                encoder.cut().unwrap();
                producer.publish(&encoder.encode(&frame).unwrap()).unwrap();
            }
            producer.publish(&encoder.finish().unwrap()).unwrap();
            stopped.await.unwrap();
            producer.finish().unwrap();
        };
        let received = async {
            while !state.entries.contains_key(&path) {
                match viewer.recv().await {
                    Event::Announcement(a) => viewer.apply(&mut state, a),
                    _ => panic!("lost Relay while waiting for the test publication"),
                }
            }
            let selection = Selection {
                generation: state.lifecycle.generation(),
                path: path.clone(),
                revision: state.entries[&path],
            };
            let broadcast = viewer.request(&selection).unwrap().await.unwrap();
            let mut catalog = moq_mux::catalog::Consumer::<()>::new(
                &broadcast,
                moq_mux::catalog::CatalogFormat::Hang,
            )
            .await
            .unwrap();
            let (name, config) = loop {
                let snapshot = catalog.next().await.unwrap().unwrap();
                assert!(snapshot.audio.renditions.is_empty());
                if let Some(item) = snapshot.video.renditions.into_iter().next() {
                    break item;
                }
            };
            let mut decoder =
                moq_video::decode::Consumer::new(&broadcast, &config, name, Default::default())
                    .await
                    .unwrap();
            let frame = decoder.read().await.unwrap().unwrap();
            let data = frame.surface.to_rgba(&Default::default()).unwrap();
            assert_eq!((data.width(), data.height()), (160, 96));
            assert!(
                data.into_data()[..3]
                    .iter()
                    .zip(rgba)
                    .all(|(v, expected)| v.abs_diff(expected) < 20)
            );
            eprintln!(
                "{path}: actual PipeWire open failed, 0 audio tracks, decoded 160x96 color {rgba:?}"
            );
            stop.send(()).unwrap();
        };
        tokio::join!(
            publication.run_tracks(video, async move |error| {
                failed.send(error).unwrap();
            }),
            received
        );
        drop(publication);
        while state.entries.contains_key(&path) {
            match viewer.recv().await {
                Event::Announcement(a) => viewer.apply(&mut state, a),
                _ => panic!("lost Relay before withdrawal"),
            }
        }
    }
    viewer.close().await.unwrap();
    publisher.close().await.unwrap();
    server.abort();
}
