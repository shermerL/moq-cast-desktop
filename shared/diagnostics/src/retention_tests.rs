use super::*;
use std::io::Read;
use tempfile::tempdir;

fn archive_text(path: &Path) -> String {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        if file.name().contains(".log") {
            file.read_to_string(&mut text).unwrap();
        }
    }
    text
}

fn traffic() {
    for packet in 0..200 {
        tracing::trace!(target: "moq_noq_proto::connection", packet, "packet detail");
    }
}

#[test]
fn trace_rotation_preserves_connection_and_playback_in_export() {
    let dir = tempdir().unwrap();
    let cfg = Config::new(Paths::for_test(dir.path()), BuildInfo::new("test"))
        .with_detailed(true)
        .with_stderr(false)
        .with_test_limits(1024, 2, 8, 1024);
    let (handle, worker, layer) = build(cfg);
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        tracing::info!(target: "moqcast_windows::session", "connection-established-fixture");
        tracing::info!(target: "moqcast_windows::media", "playback-started-fixture");
        traffic();
    });
    let path = dir.path().join("retention.zip");
    handle.export(ExportRequest::new(&path)).unwrap();
    let text = archive_text(&path);
    assert_eq!(handle.dropped_count(), 0, "test must isolate disk rotation");
    assert!(
        text.contains("packet detail"),
        "raw details must remain available"
    );
    assert!(
        text.contains("connection-established-fixture"),
        "rotation erased connection lifecycle"
    );
    assert!(
        text.contains("playback-started-fixture"),
        "rotation erased media lifecycle"
    );
    let logs = fs::read_dir(dir.path())
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| output::is_log_name(&entry.file_name().to_string_lossy()))
        .collect::<Vec<_>>();
    assert!(logs.len() <= 4);
    assert!(
        logs.iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>()
            <= 4096
    );
    drop(worker);
}

#[test]
fn trace_pressure_preserves_lifecycle_in_memory() {
    let dir = tempdir().unwrap();
    let cfg = Config::new(Paths::for_test(dir.path()), BuildInfo::new("test"))
        .with_detailed(true)
        .with_stderr(false)
        .with_test_limits(65536, 2, 8, 1024);
    let (handle, worker, layer) = build(cfg);
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        tracing::info!(target: "moqcast_windows::session", "connection-established-fixture");
        traffic();
    });
    assert!(
        handle
            .snapshot()
            .entries()
            .iter()
            .any(|e| e.event().contains("connection-established-fixture")),
        "detail entries evicted lifecycle from memory"
    );
    drop(worker);
}

#[test]
fn shutdown_drains_both_streams_after_detail_mode_is_disabled() {
    let dir = tempdir().unwrap();
    let cfg = Config::new(Paths::for_test(dir.path()), BuildInfo::new("test"))
        .with_detailed(true)
        .with_stderr(false);
    let (handle, worker, layer) = build(cfg);
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        tracing::trace!(target: "moq_noq_proto::connection", "retained detail before toggle");
        handle.set_detailed(false);
        tracing::trace!(target: "moq_noq_proto::connection", "filtered detail after toggle");
        tracing::info!(target: "moqcast_windows::session", "connection closed after toggle");
    });
    drop(worker);
    let key = fs::read_to_string(dir.path().join("moqcast.log")).unwrap();
    let detail = fs::read_to_string(dir.path().join("moqcast-detail.log")).unwrap();
    assert!(key.contains("connection closed after toggle"));
    assert!(detail.contains("retained detail before toggle"));
    assert!(!detail.contains("filtered detail after toggle"));
    assert!(!key.contains("retained detail"));
}

#[test]
fn toggling_details_back_on_preserves_both_export_streams() {
    let dir = tempdir().unwrap();
    let (handle, worker, layer) =
        build(Config::new(Paths::for_test(dir.path()), BuildInfo::new("test")).with_stderr(false));
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        tracing::trace!(target: "moq_noq_proto::connection", "filtered-before-enable");
        handle.set_detailed(true);
        tracing::trace!(target: "moq_noq_proto::connection", "detail-first-enable");
        handle.set_detailed(false);
        tracing::trace!(target: "moq_noq_proto::connection", "filtered-while-disabled");
        tracing::info!(target: "moqcast_windows::session", "connection-while-disabled");
        handle.set_detailed(true);
        tracing::trace!(target: "moq_noq_proto::connection", "detail-second-enable");
        tracing::info!(target: "moqcast_windows::media", "playback-after-reenable");
    });
    let path = dir.path().join("toggle.zip");
    handle.export(ExportRequest::new(&path)).unwrap();
    let text = archive_text(&path);
    for marker in [
        "detail-first-enable",
        "detail-second-enable",
        "connection-while-disabled",
        "playback-after-reenable",
    ] {
        assert_eq!(text.matches(marker).count(), 1, "{marker}");
    }
    assert!(!text.contains("filtered-before-enable"));
    assert!(!text.contains("filtered-while-disabled"));
    assert_eq!(handle.dropped_count(), 0);
    drop(worker);
}

#[test]
fn restart_and_detail_rotation_preserve_previous_lifecycle_history() {
    let dir = tempdir().unwrap();
    for session in 0..2 {
        let (handle, worker, layer) = build(
            Config::new(Paths::for_test(dir.path()), BuildInfo::new("test"))
                .with_stderr(false)
                .with_detailed(true)
                .with_test_limits(1024, 2, 8, 1024),
        );
        tracing::subscriber::with_default(Registry::default().with(layer), || {
            tracing::info!(target: "moqcast_windows::session", session, "restart-connection");
            traffic();
        });
        let path = dir.path().join(format!("restart-{session}.zip"));
        handle.export(ExportRequest::new(&path)).unwrap();
        let text = archive_text(&path);
        assert_eq!(text.matches("restart-connection").count(), session + 1);
        assert!(text.contains("packet detail"));
        assert_eq!(handle.dropped_count(), 0);
        drop(worker);
    }
}
