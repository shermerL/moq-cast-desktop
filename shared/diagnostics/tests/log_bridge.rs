use moqcast_diagnostics::{BuildInfo, Config};
use tracing_log::log;

// A separate test process exercises global LogTracer installation and the
// TRACE max-level hint, which direct format_trace calls do not exercise.
#[test]
fn global_log_bridge_tracks_detailed_toggle_and_original_target() {
    let diagnostics = moqcast_diagnostics::init(
        Config::without_file(BuildInfo::new("test"), "isolated bridge test")
            .with_stderr(false)
            .with_private_location_redaction(),
    );
    let handle = diagnostics.handle();
    let baseline = handle.snapshot().entries().len();
    log::trace!(target: "mdns_sd::dns_cache", "hidden before enable");
    assert_eq!(handle.snapshot().entries().len(), baseline);
    handle.set_detailed(true);
    log::trace!(target: "mdns_sd::dns_cache", "cache record rejected on interface 7");
    log::debug!(target: "mdns_sd::service_daemon", "packet received from 192.0.2.1");
    log::trace!(target: "unrelated_dependency", "unrelated trace");
    let snapshot = handle.snapshot();
    assert_eq!(snapshot.entries().len(), baseline + 2);
    assert_eq!(snapshot.entries()[baseline].target(), "mdns_sd::dns_cache");
    assert!(
        snapshot.entries()[baseline + 1]
            .event()
            .contains("192.0.2.1")
    );
    handle.set_detailed(false);
    log::trace!(target: "mdns_sd::dns_cache", "hidden after disable");
    log::warn!(target: "mdns_sd::service_daemon", "normal mode private warning 192.0.2.1");
    let snapshot = handle.snapshot();
    assert_eq!(snapshot.entries().len(), baseline + 3);
    assert_eq!(
        snapshot.entries().last().unwrap().event(),
        "dependency warning details omitted"
    );
}
