use super::*;
use std::io::Read as _;
use tempfile::tempdir;

fn exported_log(handle: &Handle, path: &Path) -> String {
    handle.export(ExportRequest::new(path)).unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut log = String::new();
    archive
        .by_name("moqcast.log")
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    log
}

#[test]
fn quic_details_toggle_and_export_on_both_platform_policies() {
    for strict in [false, true] {
        let directory = tempdir().unwrap();
        let mut config = Config::new(Paths::for_test(directory.path()), BuildInfo::new("test"))
            .with_stderr(false);
        if strict {
            config = config.with_private_location_redaction();
        }
        let (handle, worker, layer) = build(config);
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            // Reuse these callsites across runtime toggles, including an off transition.
            let emit = || {
                tracing::debug!(target: "moq_noq::endpoint", "endpoint-debug");
                tracing::trace!(target: "moq_noq::connection", "driver-trace");
                tracing::trace!(target: "moq_noq_proto::endpoint", id = 9, network_path = "192.0.2.1:4433", "initial-trace");
                tracing::trace!(target: "moq_noq_proto::connection", "tls-trace");
                tracing::trace!(target: "moq_noq_proto::connection::packet_builder", "packet-trace");
                tracing::debug!(target: "moq_noq_udp", error = "sendmsg failure", "udp-debug");
                tracing::debug!(target: "moq_noq_udp::imp", "udp-impl-debug");
                tracing::trace!(target: "moq_noq_proto::connection_extra", "unrelated-prefix");
                tracing::trace!(target: "moq_noq_proto::crypto", "unrelated-module");
                tracing::debug!(target: "unrelated_dependency", "unrelated-debug");
            };
            emit();
            assert!(handle.snapshot().entries().is_empty());
            handle.set_detailed(true);
            emit();
            assert_eq!(handle.snapshot().entries().len(), 7);
            handle.set_detailed(false);
            emit();
            assert_eq!(handle.snapshot().entries().len(), 7);
        });
        let log = exported_log(&handle, &directory.path().join("export.zip"));
        for marker in [
            "endpoint-debug",
            "driver-trace",
            "initial-trace",
            "tls-trace",
            "packet-trace",
            "udp-debug",
            "udp-impl-debug",
            "192.0.2.1:4433",
            "sendmsg failure",
        ] {
            assert!(log.contains(marker), "missing {marker}, strict={strict}");
        }
        assert!(!log.contains("unrelated-"));
        let initial: serde_json::Value = log
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|entry| entry["event"].as_str().unwrap().contains("initial-trace"))
            .unwrap();
        assert_eq!(initial["level"], "TRACE");
        assert_eq!(handle.dropped_count(), 0);
        drop(worker);
    }
}

#[test]
fn quic_span_context_tracks_records_explicit_parent_and_does_not_leak() {
    let directory = tempdir().unwrap();
    let (handle, worker, layer) = build(
        Config::new(Paths::for_test(directory.path()), BuildInfo::new("test"))
            .with_stderr(false)
            .with_private_location_redaction()
            .with_detailed(true),
    );
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        let first = tracing::debug_span!(target: "moq_noq::connection", "drive", id = 11);
        let second = tracing::debug_span!(target: "moq_noq::connection", "drive", id = 22);
        let packet = tracing::trace_span!(target: "moq_noq_proto::connection", parent: &first, "recv", pn = 3, path_id = tracing::field::Empty);
        packet.record("path_id", 7);
        second.in_scope(|| {
            // Explicit parent must win over the currently entered sibling.
            tracing::trace!(target: "moq_noq_proto::connection", parent: &packet, "first-handshake");
            tracing::debug!(target: "moq_noq::connection", "second-handshake");
        });
        drop(packet);
        drop(first);
        drop(second);
        tracing::debug!(target: "moq_noq::connection", "outside-connection");
    });
    let snapshot = handle.snapshot();
    let first = snapshot
        .entries()
        .iter()
        .find(|e| e.event().contains("first-handshake"))
        .unwrap();
    assert!(first.event().contains("drive{id=11}"), "{}", first.event());
    assert!(first.event().contains("recv{path_id=7 pn=3}"));
    assert!(!first.event().contains("id=22"));
    let second = snapshot
        .entries()
        .iter()
        .find(|e| e.event().contains("second-handshake"))
        .unwrap();
    assert!(second.event().contains("drive{id=22}"));
    assert!(!second.event().contains("id=11"));
    let outside = snapshot
        .entries()
        .iter()
        .find(|e| e.event().contains("outside-connection"))
        .unwrap();
    assert!(!outside.event().contains("context="));
    let log = exported_log(&handle, &directory.path().join("export.zip"));
    assert!(log.contains("drive{id=11}"));
    assert!(log.contains("recv{path_id=7 pn=3}"));
    drop(worker);
}

#[test]
fn quic_span_context_is_bounded_and_ignores_unrelated_fields() {
    let directory = tempdir().unwrap();
    let (handle, worker, layer) = build(
        Config::new(Paths::for_test(directory.path()), BuildInfo::new("test"))
            .with_stderr(false)
            .with_detailed(true),
    );
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        let long = "界".repeat(10_000);
        let span = tracing::debug_span!(target: "moq_noq::connection", "drive", id = 42, path_id = %long, secret = "do-not-copy", arbitrary = "ignore-me");
        span.in_scope(|| tracing::trace!(target: "moq_noq_proto::connection", "bounded-context"));
    });
    let snapshot = handle.snapshot();
    let event = snapshot.entries()[0].event();
    assert!(event.contains("id=42"));
    assert!(event.contains("[truncated]"));
    assert!(event.len() < 4096);
    assert!(!event.contains("do-not-copy"));
    assert!(!event.contains("ignore-me"));
    drop(worker);
}

#[test]
fn quic_span_context_follows_dispatch_across_threads() {
    let directory = tempdir().unwrap();
    let (handle, worker, layer) = build(
        Config::new(Paths::for_test(directory.path()), BuildInfo::new("test"))
            .with_stderr(false)
            .with_detailed(true),
    );
    let dispatch = tracing::Dispatch::new(Registry::default().with(layer));
    let parent = tracing::dispatcher::with_default(
        &dispatch,
        || tracing::debug_span!(target: "moq_noq::connection", "drive", id = 37),
    );
    let thread_dispatch = dispatch.clone();
    std::thread::spawn(move || {
        tracing::dispatcher::with_default(&thread_dispatch, || {
            tracing::trace!(target: "moq_noq_proto::connection", parent: &parent, "thread-handshake");
        });
    }).join().unwrap();
    let log = exported_log(&handle, &directory.path().join("thread.zip"));
    assert!(log.contains("thread-handshake context=drive{id=37}"));
    drop(worker);
}
