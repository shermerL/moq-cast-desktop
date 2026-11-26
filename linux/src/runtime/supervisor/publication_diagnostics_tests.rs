use super::{PublishCompletion, Supervisor};
use crate::app::{MediaState, UserCommand};
use moqcast_diagnostics::{
    BuildInfo, Config, Paths, Platform,
    media::{Session, Source},
};
use std::{fs::File, path::PathBuf, process::Command};
use tokio::sync::watch;

fn run_isolated(test: &str) -> tempfile::TempDir {
    // Session history and the diagnostics log writer are process-wide.
    let root = tempfile::tempdir().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--ignored"])
        .env("XDG_STATE_HOME", root.path())
        .env("MOQCAST_TEST_PUBLICATION_DIR", root.path())
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");
    root
}

#[test]
fn publication_outcomes_in_exported_zip() {
    let root = run_isolated(
        "runtime::supervisor::publication_diagnostics_tests::exercise_publication_completion",
    );

    for (file, expected) in [
        ("stop-ok.zip", "stopped"),
        ("stop-cleanup-error.zip", "failed"),
        ("completed.zip", "ended"),
        ("publication-error.zip", "failed"),
    ] {
        let mut zip = zip::ZipArchive::new(File::open(root.path().join(file)).unwrap()).unwrap();
        let summary: serde_json::Value =
            serde_json::from_reader(zip.by_name("media-session.json").unwrap()).unwrap();
        assert!(summary["current"].is_null(), "{file}");
        assert_eq!(summary["last_ended"]["role"], "publish", "{file}");
        assert_eq!(summary["last_ended"]["status"], expected, "{file}");
    }
}

#[tokio::test]
#[ignore = "runs in the isolated process started by publication_outcomes_in_exported_zip"]
async fn exercise_publication_completion() {
    let root = PathBuf::from(std::env::var_os("MOQCAST_TEST_PUBLICATION_DIR").unwrap());
    let diagnostics = moqcast_diagnostics::init(
        Config::new(
            Paths::for_platform(Platform::Linux).unwrap(),
            BuildInfo::new("test"),
        )
        .with_stderr(false),
    );
    for (file, stop, failure) in [
        ("stop-ok.zip", true, None),
        ("stop-cleanup-error.zip", true, Some("portal close failed")),
        ("completed.zip", false, None),
        ("publication-error.zip", false, Some("capture failed")),
    ] {
        let mut supervisor = Supervisor::new(watch::channel(None).0, Default::default());
        supervisor.state.media = MediaState::Publishing;
        let generation = supervisor.publish.advance();
        let (cancel, mut cancelled) = watch::channel(false);
        let completion = PublishCompletion {
            session: Session::publish(generation, Source::Display, false),
            events: supervisor.operation_tx.clone(),
            cancelled: cancelled.clone(),
        };
        // Publication::run returns one result after capture and portal cleanup.
        let publication = async move {
            if stop {
                cancelled.wait_for(|stopping| *stopping).await.unwrap();
            }
            match failure {
                Some(message) => Err(anyhow::anyhow!(message)),
                None => Ok(()),
            }
        };
        supervisor.publish.cancel = Some(cancel);
        supervisor.publish.task = Some(tokio::spawn(completion.run(publication)));
        if stop {
            supervisor
                .handle_command(UserCommand::StopScreenShare)
                .await;
        } else {
            let event = supervisor.operation_rx.recv().await.unwrap();
            supervisor.handle_operation_event(event).await;
        }
        assert_eq!(supervisor.state.media, MediaState::Idle, "{file}");
        match failure {
            Some(message) => assert!(
                supervisor
                    .state
                    .last_error
                    .as_deref()
                    .unwrap()
                    .contains(message),
                "{file}"
            ),
            None => assert!(supervisor.state.last_error.is_none(), "{file}"),
        }
        diagnostics.handle().prepare_export(root.join(file))().unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn watch_reopen_resets_sampling_and_exports_its_own_outcome() {
    let root =
        run_isolated("runtime::supervisor::publication_diagnostics_tests::exercise_watch_reopen");
    let record = |file: &str| {
        let mut zip = zip::ZipArchive::new(File::open(root.path().join(file)).unwrap()).unwrap();
        let summary: serde_json::Value =
            serde_json::from_reader(zip.by_name("media-session.json").unwrap()).unwrap();
        assert!(summary["current"].is_null());
        assert_eq!(summary["last_ended"]["role"], "watch");
        summary["last_ended"].clone()
    };
    let first = record("stopped.zip");
    let second = record("failed.zip");
    assert_eq!(first["status"], "stopped");
    assert_eq!(first["requested"], "software");
    assert_eq!(second["status"], "failed");
    assert_eq!(second["requested"], "auto");
    assert_ne!(first["session_id"], second["session_id"]);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "runs in the isolated process started by watch_reopen_resets_sampling_and_exports_its_own_outcome"]
async fn exercise_watch_reopen() {
    use super::OperationEvent;
    use moq_tokio::moq_net;
    use moqcast_diagnostics::media::set_next_software;
    use moqcast_playback_stats::PlaybackStats;

    let root = PathBuf::from(std::env::var_os("MOQCAST_TEST_PUBLICATION_DIR").unwrap());
    let diagnostics = moqcast_diagnostics::init(
        Config::new(
            Paths::for_platform(Platform::Linux).unwrap(),
            BuildInfo::new("test"),
        )
        .with_stderr(false),
    );
    let stats = PlaybackStats::default();
    let mut supervisor = Supervisor::new(watch::channel(None).0, stats.clone());
    supervisor.state.start_discovery();
    supervisor.state.local_peer_id = Some("local".into());
    let path = "moqcast.screen/peer";
    let producer = moq_net::broadcast::Info::new().produce();
    supervisor
        .remote_screens
        .insert(path.into(), producer.consume());
    supervisor.state.update_remote_screen(path.into(), true);

    set_next_software(true);
    supervisor.start_view(path.into());
    let first = supervisor.state.view_generation;
    stats.set_sampling(first, true);
    stats.mark(first);
    assert!(!stats.snapshot(first).unwrap().marks.is_empty());
    supervisor.handle_command(UserCommand::StopWatching).await;
    assert!(stats.snapshot(first).is_none());
    diagnostics
        .handle()
        .prepare_export(root.join("stopped.zip"))()
    .unwrap();

    set_next_software(false);
    supervisor.start_view(path.into());
    let second = supervisor.state.view_generation;
    assert_ne!(first, second);
    supervisor
        .handle_operation_event(OperationEvent::ViewEnded {
            generation: first,
            result: Err("late completion".into()),
        })
        .await;
    stats.set_sampling(first, true);
    stats.mark(first);
    stats.accepted(first, 1, 1);
    let fresh = stats.snapshot(second).unwrap();
    assert!(!fresh.enabled && fresh.samples.is_empty() && fresh.marks.is_empty());
    assert!(matches!(
        supervisor.state.media,
        MediaState::PreparingView { .. }
    ));

    // Closing a source before its first catalog reaches the actual reader's error path.
    producer.close();
    let event = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        supervisor.operation_rx.recv(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        matches!(event, OperationEvent::ViewEnded { generation, result: Err(_) } if generation == second)
    );
    supervisor.handle_operation_event(event).await;
    assert_eq!(supervisor.state.media, MediaState::Idle);
    assert!(stats.snapshot(second).is_none());
    diagnostics.handle().prepare_export(root.join("failed.zip"))().unwrap();
}
