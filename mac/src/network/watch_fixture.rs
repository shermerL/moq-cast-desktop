//! Finite synthetic publisher for the native Nearby watch acceptance flow.
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "publishes a synthetic test card on the local network for native acceptance"]
async fn publish_lan_watch_test_card() -> anyhow::Result<()> {
    let mut services = super::Services::start().await?;
    let path = crate::contract::screen_path(services.local_peer_id());
    let origin = services.publish_origin();
    let mut broadcast = origin.create_broadcast(&path)?;
    let clock = moq_mux::Clock::new();
    let catalog = moq_mux::catalog::Producer::<()>::new(
        &mut broadcast,
        moq_mux::catalog::Config::default().with_clock(clock),
    )?;
    let mut audio = if std::env::var_os("MOQCAST_WATCH_FIXTURE_VIDEO_ONLY").is_some() {
        None
    } else {
        Some(moq_audio::encode::Producer::new(
            &mut broadcast,
            catalog.clone(),
            Default::default(),
            &Default::default(),
        )?)
    };
    let size = moq_video::Size::new(960, 540);
    let mut config = moq_video::encode::Config::new(960, 540, moq_video::Rate::new(30, 1)?);
    config.kind = moq_video::encode::Kind::Software;
    let rendition = config.probe().await?;
    let mut encoder = moq_video::encode::Encoder::new(&config)?;
    let mut video = moq_video::encode::Producer::new(broadcast.clone(), catalog, rendition)?;
    broadcast.announce(Default::default())?;
    let mut pixels = vec![0; size.pixels() as usize * 4];
    let bars = [
        [172, 183, 186],
        [187, 173, 94],
        [78, 159, 157],
        [100, 148, 106],
        [153, 114, 155],
        [174, 103, 96],
        [83, 110, 159],
        [63, 70, 76],
    ];
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let x = index % 960;
        let y = index / 960;
        let [r, g, b] = if y < 390 {
            bars[x / 120]
        } else {
            let gray = (26 + (x / 80) * 15) as u8;
            [gray; 3]
        };
        pixel.copy_from_slice(&[r, g, b, 255]);
    }
    let mut silence = moq_audio::Frame::new(vec![0u8; 1600 * 2 * 4].into(), clock.now());
    let mut ticks = tokio::time::interval(Duration::from_nanos(1_000_000_000 / 30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut index = 0u64;
    let stop_file = std::env::var_os("MOQCAST_WATCH_FIXTURE_STOP");
    // Extended runs use the same continuously changing source for every comparison.
    let lifetime = std::env::var("MOQCAST_WATCH_FIXTURE_SECS")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(600);
    let deadline = tokio::time::sleep(Duration::from_secs(lifetime));
    tokio::pin!(deadline);
    eprintln!(
        "Nearby test card ready: 960x540 H.264, audio track = {}",
        audio.is_some()
    );
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            event = services.recv() => anyhow::ensure!(event.is_some(), "Nearby service ended"),
            _ = ticks.tick() => {
                if index.is_multiple_of(30) {
                    if stop_file.as_ref().is_some_and(|path| std::path::Path::new(path).exists()) { break; }
                    encoder.cut()?;
                }
                let stripe = (index % 960) as usize;
                for row in pixels.chunks_exact_mut(960 * 4).skip(390).take(16) {
                    row.fill(36);
                    row[stripe*4..stripe*4+4].fill(255);
                }
                let now = clock.now();
                if let Some(audio) = &mut audio {
                    silence.timestamp = now;
                    audio.write(&silence)?;
                }
                let frame = moq_video::Frame::new(moq_video::Surface::rgba(&pixels, size)?, now);
                video.publish(&encoder.encode(&frame)?)?;
                index += 1;
            }
        }
    }
    video.publish(&encoder.finish()?)?;
    video.finish()?;
    if let Some(audio) = &mut audio {
        audio.finish()?;
    }
    broadcast.unannounce();
    broadcast.close();
    // The origin driver exits only after the last publisher handle drops.
    drop((video, audio, broadcast, origin));
    services.shutdown().await.map_err(anyhow::Error::msg)
}
