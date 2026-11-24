use super::{PublishCompletion, Supervisor};
use crate::app::{MediaState, UserCommand};
use moqcast_diagnostics::{
    BuildInfo, Config, Paths, Platform,
    media::{Session, Source},
};
use std::{fs::File, path::PathBuf, process::Command};
use tokio::sync::watch;

#[test]
fn publication_outcomes_in_exported_zip() {
    // Session history and the diagnostics log writer are process-wide.
    let root = tempfile::tempdir().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime::supervisor::publication_diagnostics_tests::exercise_publication_completion",
            "--ignored",
        ])
        .env("XDG_STATE_HOME", root.path())
        .env("MOQCAST_TEST_PUBLICATION_DIR", root.path())
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");

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
        let mut supervisor = Supervisor::new(watch::channel(None).0);
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
