use super::*;

#[test]
fn fresh_paths_preserve_the_relative_prefix_without_reusing_content_identity() {
    for invalid in [
        "",
        "/",
        "/screen",
        "a//b",
        "a/../b",
        ".hidden",
        "a%2Fb",
        "a?q",
        "a#fragment",
        "a b",
    ] {
        assert!(relay::PublishPrefix::parse(invalid).is_none(), "{invalid}");
    }
    let prefix = relay::PublishPrefix::parse("team/screen/").unwrap();
    let first = prefix.broadcast_path();
    let next = relay::PublishPrefix::parse("team/screen")
        .unwrap()
        .broadcast_path();
    assert_ne!(first, next);
    for path in [first, next] {
        let suffix = path.strip_prefix("team/screen/").unwrap();
        assert_eq!(suffix.len(), 36);
        assert!(!suffix.contains('/'));
    }
}

async fn wait_for(
    owner: &RuntimeOwner,
    predicate: impl Fn(&AppSnapshot) -> bool,
) -> anyhow::Result<Arc<AppSnapshot>> {
    let mut updates = owner.snapshot.clone();
    let snapshot =
        tokio::time::timeout(Duration::from_secs(15), updates.wait_for(|s| predicate(s)))
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "timed out: relay={:?} media={:?} error={:?}",
                    owner.snapshot().relay.lifecycle.phase(),
                    owner.snapshot().media.phase(),
                    owner.snapshot().media_error
                )
            })??
            .clone();
    Ok(snapshot)
}

// Use a specifically named test card and an opt-in loopback relay. Never fall back to the display.
#[test]
#[ignore = "requires the designated macOS test window, Screen Recording permission and local relay"]
fn native_relay_window_publish_watch_stop_and_restart() -> anyhow::Result<()> {
    let window_id: u32 = std::env::var("MOQCAST_TEST_WINDOW_ID")?.parse()?;
    let address = std::env::var("MOQCAST_TEST_RELAY_URL")?;
    let url = url::Url::parse(&address)?;
    anyhow::ensure!(url.host_str() == Some("127.0.0.1"));
    let config = relay::Config::parse(&address, &std::env::var("MOQCAST_TEST_RELAY_PIN")?)
        .map_err(|_| anyhow::anyhow!("invalid fixture relay"))?;
    let publisher = RuntimeOwner::start(|| {})?;
    let viewer = RuntimeOwner::start(|| {})?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let windows = moq_video::capture::windows().await?;
        anyhow::ensure!(windows.iter().any(|w| w.id == window_id.to_string() && w.title == "MoQCast Diagnostic Test Card"), "designated test card unavailable");
        for owner in [&publisher, &viewer] {
            anyhow::ensure!(owner.connect_relay(config.clone()));
            wait_for(owner, |s| s.relay.lifecycle.phase() == relay::Phase::Connected).await?;
        }
        let connection = publisher.snapshot().relay.lifecycle.generation();
        anyhow::ensure!(publisher.select_share_source(ShareSelection::Window { window_id, label: "designated test card".into() }));
        let mut old = None;
        for round in 1..=2 {
            anyhow::ensure!(publisher.start_relay_share(connection, relay::PublishPrefix::parse("test-card").unwrap()));
            let sharing = wait_for(&publisher, |s| s.media.phase() == MediaPhase::Sharing).await?;
            let path = sharing.media_path.as_deref().unwrap();
            assert_ne!(old.as_deref(), Some(path));
            let directory = wait_for(&viewer, |s| s.relay.entries.contains_key(path)).await?;
            assert_eq!(directory.media_owner, None);
            anyhow::ensure!(viewer.watch_relay(relay::Selection { generation: directory.relay.lifecycle.generation(), path: path.into(), revision: directory.relay.entries[path] }));
            if round == 1 {
                anyhow::ensure!(publisher.stop_network());
                wait_for(&publisher, |s| s.discovery.phase() == DiscoveryPhase::Stopped && s.media.phase() == MediaPhase::Sharing).await?;
            }
            let watching = wait_for(&viewer, |s| s.media.phase() == MediaPhase::Watching).await?;
            let mut frames = viewer.frames.clone();
            let frame = tokio::time::timeout(Duration::from_secs(10), frames.wait_for(|frame| frame.as_ref().is_some_and(|f| f.identity.view_generation == watching.media.generation().value()))).await??.clone().unwrap();
            assert!(frame.display_width > 0 && frame.display_height > 0);
            assert!(!viewer.stats.snapshot(watching.media.generation().value()).unwrap().enabled);
            eprintln!("round={round} path={path} version={:?} decoded={}x{} generation={}", directory.relay.version, frame.display_width, frame.display_height, frame.identity.view_generation);
            if round == 1 {
                anyhow::ensure!(publisher.stop_sharing(sharing.media.generation()));
                wait_for(&publisher, |s| s.media_owner.is_none() && s.relay.lifecycle.phase() == relay::Phase::Connected).await?;
            } else {
                anyhow::ensure!(publisher.disconnect_relay(connection));
                wait_for(&publisher, |s| s.media_owner.is_none() && s.relay.lifecycle.phase() == relay::Phase::Disconnected).await?;
            }
            let ended = wait_for(&viewer, |s| !s.relay.entries.contains_key(path) && (s.media.phase() == MediaPhase::Failed || s.media_owner.is_none())).await?;
            if ended.media_owner == Some(MediaOwner::Watch) {
                anyhow::ensure!(viewer.stop_watching(ended.media.generation()));
            }
            wait_for(&viewer, |s| s.media_owner.is_none() && s.relay.lifecycle.phase() == relay::Phase::Connected).await?;
            old = Some(path.to_owned());
        }
        anyhow::ensure!(publisher.connect_relay(config));
        let reopened = wait_for(&publisher, |s| s.relay.lifecycle.phase() == relay::Phase::Connected).await?;
        assert_ne!(reopened.relay.lifecycle.generation(), connection);
        assert!(reopened.media_owner.is_none());
        Ok(())
    })
}
