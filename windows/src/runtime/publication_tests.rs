use super::*;
use moqcast_diagnostics::{
    BuildInfo, Config, ExportRequest, Paths, Platform,
    media::{Session, Source},
};
use std::{future::pending, io::Read, process::Command};

#[test]
fn publication_end_reasons_survive_immediate_reopen() {
    // Use a dedicated process for the process-wide session history and log writer.
    let root = tempfile::tempdir().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime::publication_tests::exercise_publication_owner",
            "--ignored",
        ])
        .env("LOCALAPPDATA", root.path())
        .env("MOQCAST_TEST_PUBLICATION_DIR", root.path())
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");

    let summary = |file: &str| {
        let file = std::fs::File::open(root.path().join(file)).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut json = String::new();
        archive
            .by_name("media-session.json")
            .unwrap()
            .read_to_string(&mut json)
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&json).unwrap()
    };
    let reopened = summary("reopened.zip");
    let first = &reopened["last_ended"];
    let second = &reopened["current"];
    assert_eq!(first["status"], "stopped");
    assert_eq!(second["status"], "running");
    assert_ne!(first["session_id"], second["session_id"]);
    let cancelled = summary("cancelled.zip");
    assert!(cancelled["current"].is_null());
    assert_eq!(cancelled["last_ended"]["session_id"], second["session_id"]);
    assert_eq!(cancelled["last_ended"]["status"], "cancelled");
    for name in ["failed.zip", "panic.zip"] {
        assert_eq!(summary(name)["last_ended"]["status"], "failed");
    }
    assert_eq!(summary("ended.zip")["last_ended"]["status"], "ended");
}

#[tokio::test]
#[ignore = "runs in the isolated process started by publication_end_reasons_survive_immediate_reopen"]
async fn exercise_publication_owner() {
    let root = PathBuf::from(std::env::var_os("MOQCAST_TEST_PUBLICATION_DIR").unwrap());
    let diagnostics = moqcast_diagnostics::init(
        Config::new(
            Paths::for_platform(Platform::Windows).unwrap(),
            BuildInfo::new("test"),
        )
        .with_stderr(false),
    );
    let export = |file: &str| {
        diagnostics
            .handle()
            .export(ExportRequest::new(root.join(file)))
            .unwrap();
    };
    let mut owner = PublicationOwner::default();
    let (events, mut receiver) = mpsc::channel(1);
    let session = |generation| Session::publish(generation, Source::Window, false);

    let mut snapshot = RuntimeSnapshot::default();
    let generation = snapshot.media.begin("local").unwrap();
    assert!(snapshot.media.started(
        generation,
        crate::media::PublicationInfo {
            width: 320,
            height: 240
        }
    ));
    owner.start(session(generation), pending(), events.clone());
    let (snapshots, _) = watch::channel(snapshot.clone());
    handle_command(
        RuntimeCommand::StopSharing,
        RuntimeContext {
            config: &RuntimeConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                node: None,
                secret_file: None,
            },
            snapshot: &mut snapshot,
            snapshots: &snapshots,
            publication: &mut owner,
            media_events: &events,
            services: &mut ServiceOwner::default(),
            view: &mut ViewOwner::default(),
            view_events: &mpsc::channel(1).0,
            playback: &watch::channel(None).0,
            audio_updates: &watch::channel(None).0,
        },
    )
    .await;
    let next = snapshot.media.begin("local").unwrap();
    owner.start(session(next), pending(), events.clone());
    owner.finished(generation);
    assert!(
        !snapshot
            .media
            .ended(generation, Err(PublicationFailure::Unexpected))
    );
    assert!(owner.task.is_some() && owner.stop_request.is_some());
    export("reopened.zip");
    owner.stop(PublicationShutdown::Cancel).await.unwrap();
    export("cancelled.zip");

    // A real capture failure must win over a simultaneous stop, even with a full event queue.
    events
        .try_send(MediaEvent::Ended {
            generation: 1,
            result: Ok(()),
        })
        .unwrap();
    owner.start(
        session(3),
        async { Err(PublicationFailure::Unexpected) },
        events.clone(),
    );
    owner.stop(PublicationShutdown::StopSharing).await.unwrap();
    export("failed.zip");
    receiver.recv().await.unwrap();

    owner.start(
        session(4),
        async { panic!("test capture panic") },
        events.clone(),
    );
    assert!(owner.stop(PublicationShutdown::StopSharing).await.is_err());
    export("panic.zip");

    owner.start(session(5), async { Ok(()) }, events);
    assert!(matches!(
        receiver.recv().await,
        Some(MediaEvent::Ended {
            generation: 5,
            result: Ok(())
        })
    ));
    owner.finished(5);
    export("ended.zip");
}
