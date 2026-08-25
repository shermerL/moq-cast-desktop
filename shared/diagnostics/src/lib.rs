//! Bounded local diagnostics shared by MoQCast desktop applications.

#![deny(missing_docs)]

mod config;
mod export;
mod filter;
mod output;
mod ring;
mod worker;

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::thread;

pub use config::{BuildInfo, Config, FileStatus, PathError, Paths, Platform};
pub use export::{ExportError, ExportRequest, ExportResult};
pub use ring::{Entry, LogLevel, Snapshot};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry};

use crate::config::BuildInfo as InternalBuildInfo;
use crate::filter::FilterPolicy;
use crate::output::RotatingOutput;
use crate::ring::Ring;
use crate::worker::{WorkerGuard, Writer};

const MAX_RENDERED_EVENT_BYTES: usize = 16 * 1024;
const MAX_CONTEXT_BYTES: usize = 1024;
const MAX_LOG_LINE_BYTES: usize = 128 * 1024;
const TRUNCATED_SUFFIX: &str = " [truncated]";

/// An initialized local diagnostics pipeline and its shutdown flush guard.
pub struct Diagnostics {
    handle: Handle,
    _worker: Option<WorkerGuard>,
}

impl Diagnostics {
    /// Return a cloneable runtime and UI handle.
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
}

/// A cloneable handle for runtime filtering, snapshots, paths, and local export.
#[derive(Clone)]
pub struct Handle {
    inner: Arc<Inner>,
}

impl Handle {
    /// Enable or disable detailed diagnostics for the fixed allowed target set.
    pub fn set_detailed(&self, detailed: bool) {
        self.inner.filter.set_detailed(detailed);
    }

    /// Return whether detailed diagnostics are enabled.
    pub fn detailed(&self) -> bool {
        self.inner.filter.detailed()
    }

    /// Return the active filter description included in exports.
    pub fn active_filter(&self) -> String {
        self.inner.filter.description()
    }

    /// Return whether persistent local diagnostic files are available.
    pub fn file_status(&self) -> FileStatus {
        self.inner
            .file_status
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Return the current ring revision without cloning its entries.
    pub fn revision(&self) -> u64 {
        self.inner.ring.revision()
    }

    /// Return a stable bounded snapshot of application log entries.
    pub fn snapshot(&self) -> Snapshot {
        self.inner.ring.snapshot(self.dropped_count())
    }

    /// Return diagnostics dropped by the lossy writer queue or contended ring.
    pub fn dropped_count(&self) -> u64 {
        self.inner
            .writer
            .dropped()
            .saturating_add(self.inner.ring.contention_dropped())
    }

    /// Return the recommended timestamped ZIP filename for a local export.
    pub fn recommended_export_name(&self) -> String {
        export::recommended_name(OffsetDateTime::now_utc())
    }

    /// Flush logs and write them with environment metadata to a local ZIP.
    pub fn export(&self, request: ExportRequest) -> Result<ExportResult, ExportError> {
        let log_dir = self
            .file_status()
            .directory()
            .map(Path::to_owned)
            .ok_or(ExportError::FileOutputUnavailable)?;
        self.inner.writer.export(
            log_dir,
            self.inner.build.clone(),
            self.active_filter(),
            self.dropped_count(),
            request,
        )
    }
}

struct Inner {
    file_status: RwLock<FileStatus>,
    build: InternalBuildInfo,
    filter: FilterPolicy,
    ring: Arc<Ring>,
    writer: Writer,
}

/// Install the bounded local diagnostics subscriber for this process.
pub fn init(config: Config) -> Diagnostics {
    let (handle, worker, layer) = build(config);
    if let Err(error) = Registry::default().with(layer).try_init() {
        let reason = format!("diagnostics subscriber unavailable: {error}");
        eprintln!("MoQCast diagnostics degraded: {reason}");
        *handle
            .inner
            .file_status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = FileStatus::Unavailable(reason);
    } else if handle.file_status().unavailable_reason().is_some() {
        tracing::warn!(
            diagnostics_status = "file-output-unavailable",
            "persistent diagnostics unavailable; bounded memory and stderr remain active"
        );
    }
    Diagnostics {
        handle,
        _worker: worker,
    }
}

fn build(config: Config) -> (Handle, Option<WorkerGuard>, DiagnosticsLayer) {
    let file_result = config.paths.as_ref().map(|paths| {
        fs::create_dir_all(paths.log_dir())?;
        let output = RotatingOutput::new(
            &paths.active_log(),
            config.max_file_bytes,
            config.max_log_files,
            config.stderr,
        )?;
        let (writer, worker) = Writer::spawn(Box::new(output), config.queue_capacity)?;
        Ok::<_, std::io::Error>((writer, worker, paths.log_dir().to_owned()))
    });
    let (writer, worker, file_status) = match file_result {
        Some(Ok((writer, worker, log_dir))) => {
            (writer, Some(worker), FileStatus::Available(log_dir))
        }
        result => {
            let reason = match result {
                Some(Err(error)) => {
                    format!("local file diagnostics initialization failed: {error}")
                }
                None => config
                    .file_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "local file diagnostics were not configured".to_owned()),
                Some(Ok(_)) => unreachable!(),
            };
            match Writer::spawn(output::fallback(config.stderr), config.queue_capacity) {
                Ok((writer, worker)) => (writer, Some(worker), FileStatus::Unavailable(reason)),
                Err(error) => (
                    Writer::disabled(),
                    None,
                    FileStatus::Unavailable(format!(
                        "{reason}; diagnostics worker could not start: {error}"
                    )),
                ),
            }
        }
    };
    let filter = FilterPolicy::new(config.detailed);
    let ring = Ring::new(config.ring_capacity);
    let inner = Arc::new(Inner {
        file_status: RwLock::new(file_status),
        build: config.build,
        filter: filter.clone(),
        ring: ring.clone(),
        writer: writer.clone(),
    });
    (
        Handle { inner },
        worker,
        DiagnosticsLayer {
            filter,
            ring,
            writer,
        },
    )
}

struct DiagnosticsLayer {
    filter: FilterPolicy,
    ring: Arc<Ring>,
    writer: Writer,
}

impl<S> Layer<S> for DiagnosticsLayer
where
    S: Subscriber,
{
    fn enabled(&self, metadata: &Metadata<'_>, _context: Context<'_, S>) -> bool {
        self.filter.allows(metadata.level(), metadata.target())
    }

    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let entry = format_event(event);
        self.ring.push(entry.clone());
        self.writer.write_line(entry.line.as_bytes());
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::DEBUG)
    }
}

fn format_event(event: &Event<'_>) -> Entry {
    let metadata = event.metadata();
    let rendered = if opaque_dependency_target(metadata.target()) {
        "dependency warning details omitted".to_owned()
    } else {
        let mut visitor = EventVisitor::new(metadata.target());
        event.record(&mut visitor);
        let message = visitor
            .message
            .unwrap_or_else(|| bounded_text(metadata.name(), MAX_RENDERED_EVENT_BYTES));
        let mut rendered = BoundedText::new(MAX_RENDERED_EVENT_BYTES);
        let _ = rendered.write_str(&message);
        for (name, value) in visitor.fields {
            let _ = write!(rendered, " {name}={value}");
        }
        rendered.finish()
    };

    let current = thread::current();
    let thread = bounded_text(
        &current
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{:?}", current.id())),
        MAX_CONTEXT_BYTES,
    );
    let timestamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    let level = match *metadata.level() {
        tracing::Level::ERROR => LogLevel::Error,
        tracing::Level::WARN => LogLevel::Warn,
        tracing::Level::INFO => LogLevel::Info,
        tracing::Level::DEBUG | tracing::Level::TRACE => LogLevel::Debug,
    };
    let target = bounded_text(metadata.target(), MAX_CONTEXT_BYTES);
    let serialized = serde_json::to_string(&json!({
        "timestamp": timestamp,
        "level": level.as_str(),
        "target": target,
        "thread": thread,
        "event": rendered,
    }))
    .unwrap_or_else(|_| "{\"level\":\"ERROR\",\"event\":\"log formatting failed\"}".to_owned());
    let line = if serialized.len().saturating_add(1) <= MAX_LOG_LINE_BYTES {
        serialized + "\n"
    } else {
        "{\"level\":\"ERROR\",\"event\":\"log line exceeded bounded output\"}\n".to_owned()
    };
    Entry {
        timestamp,
        level,
        target,
        thread,
        event: rendered,
        line,
    }
}

struct EventVisitor {
    message: Option<String>,
    fields: BTreeMap<String, String>,
    redact_location_fields: bool,
}

impl EventVisitor {
    fn new(target: &str) -> Self {
        Self {
            message: None,
            fields: BTreeMap::new(),
            redact_location_fields: target_matches(target, "moq_tokio"),
        }
    }

    fn record_value(&mut self, field: &Field, value: String) {
        let name = field.name();
        if name == "message" {
            self.message = Some(value);
        } else {
            self.fields.insert(name.to_owned(), value);
        }
    }

    fn record_redacted(&mut self, field: &Field) -> bool {
        let name = field.name().to_ascii_lowercase();
        let value = if sensitive_field(&name) {
            Some("[REDACTED]")
        } else if self.redact_location_fields && matches!(name.as_str(), "url" | "uri") {
            Some("[LOCATION OMITTED]")
        } else {
            None
        };
        if let Some(value) = value {
            self.record_value(field, value.to_owned());
            true
        } else {
            false
        }
    }
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if self.record_redacted(field) {
            return;
        }
        let mut rendered = BoundedText::new(MAX_RENDERED_EVENT_BYTES);
        let _ = write!(rendered, "{value:?}");
        self.record_value(field, rendered.finish());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if self.record_redacted(field) {
            return;
        }
        self.record_value(field, bounded_text(value, MAX_RENDERED_EVENT_BYTES));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if self.record_redacted(field) {
            return;
        }
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if self.record_redacted(field) {
            return;
        }
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if self.record_redacted(field) {
            return;
        }
        self.record_value(field, value.to_string());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if self.record_redacted(field) {
            return;
        }
        let mut rendered = BoundedText::new(MAX_RENDERED_EVENT_BYTES);
        let _ = write!(rendered, "{value}");
        self.record_value(field, rendered.finish());
    }
}

fn sensitive_field(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "cookie"
            | "credential"
            | "fingerprint"
            | "password"
            | "secret"
            | "tls_fingerprint"
            | "token"
    )
}

fn opaque_dependency_target(target: &str) -> bool {
    target_matches(target, "mdns_sd")
}

fn target_matches(target: &str, prefix: &str) -> bool {
    target == prefix
        || target
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

struct BoundedText {
    value: String,
    max_bytes: usize,
    truncated: bool,
}

impl BoundedText {
    fn new(max_bytes: usize) -> Self {
        Self {
            value: String::with_capacity(max_bytes.min(1024)),
            max_bytes,
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            while self.value.len().saturating_add(TRUNCATED_SUFFIX.len()) > self.max_bytes {
                self.value.pop();
            }
            self.value.push_str(TRUNCATED_SUFFIX);
        }
        self.value
    }

    fn push_fragment(&mut self, fragment: &str) {
        if self.truncated {
            return;
        }
        if self.value.len().saturating_add(fragment.len()) <= self.max_bytes {
            self.value.push_str(fragment);
        } else {
            self.truncated = true;
        }
    }
}

impl fmt::Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for character in value.chars() {
            match character {
                '\r' => self.push_fragment("\\r"),
                '\n' => self.push_fragment("\\n"),
                character => {
                    let mut encoded = [0; 4];
                    self.push_fragment(character.encode_utf8(&mut encoded));
                }
            }
            if self.truncated {
                break;
            }
        }
        Ok(())
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    let mut rendered = BoundedText::new(max_bytes);
    let _ = rendered.write_str(value);
    rendered.finish()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;
    use tracing_subscriber::prelude::*;

    use super::*;

    fn config(directory: &Path) -> Config {
        Config::new(Paths::for_test(directory), BuildInfo::new("test"))
            .with_stderr(false)
            .with_test_limits(1_024, 5, 8, 8)
    }

    fn log_dir(handle: &Handle) -> PathBuf {
        handle
            .file_status()
            .directory()
            .expect("test file diagnostics are available")
            .to_owned()
    }

    #[test]
    fn utf8_cjk_event_is_flushed_on_shutdown() {
        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "moqcast_diagnostics::tests", event = "屏幕共享", "中文日志");
        });
        drop(worker);

        let log = fs::read_to_string(log_dir(&handle).join("moqcast.log")).unwrap();
        assert!(log.contains("中文日志"));
        assert!(log.contains("屏幕共享"));
        assert_eq!(handle.snapshot().entries().len(), 1);
    }

    #[test]
    fn exact_sensitive_fields_are_redacted_before_ring_and_file() {
        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                target: "moqcast_diagnostics::tests",
                credential = "lan-proof",
                fingerprint = "full-fingerprint",
                peer_id = "peer-a",
                "authorization rejected"
            );
        });
        drop(worker);

        let snapshot = handle.snapshot();
        let entry = &snapshot.entries()[0];
        assert!(entry.event().contains("credential=[REDACTED]"));
        assert!(entry.event().contains("fingerprint=[REDACTED]"));
        assert!(entry.event().contains("peer_id=peer-a"));
        assert!(!entry.line().contains("lan-proof"));
        assert!(!entry.line().contains("full-fingerprint"));
    }

    #[test]
    fn endpoint_safe_connection_errors_keep_detail_without_location_fields() {
        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        let endpoint = "moqt://192.168.1.5:4443";
        let error = format!("timed out connecting to {endpoint}");
        let credential_path = "/.cluster/private-lan-credential";
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                target: "moqcast_diagnostics::tests",
                stage = "transport",
                error = %error,
                "mesh peer connection failed"
            );
            tracing::warn!(
                target: "moq_tokio::connection",
                peer = endpoint,
                error = %error,
                uri = credential_path,
                "native transport failed"
            );
        });
        drop(worker);

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.entries().len(), 2);
        for entry in snapshot.entries() {
            assert!(entry.event().contains(&format!("error={error}")));
            assert!(entry.event().contains(endpoint));
        }
        assert!(
            snapshot.entries()[1]
                .event()
                .contains("uri=[LOCATION OMITTED]")
        );
        let file = fs::read_to_string(log_dir(&handle).join("moqcast.log")).unwrap();
        assert!(file.contains(&error));
        assert!(!file.contains(credential_path));
    }

    #[test]
    fn mdns_sd_dependency_details_are_not_persisted() {
        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        let credential = "private-lan-credential";
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                target: "mdns_sd::service",
                record = %format_args!("TXT a={credential}"),
                "mDNS record rejected"
            );
        });
        drop(worker);

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.entries().len(), 1);
        assert_eq!(
            snapshot.entries()[0].event(),
            "dependency warning details omitted"
        );
        let file = fs::read_to_string(log_dir(&handle).join("moqcast.log")).unwrap();
        assert!(!file.contains(credential));
    }

    #[test]
    fn file_initialization_failure_keeps_bounded_memory_diagnostics_active() {
        let directory = tempdir().unwrap();
        let blocking_file = directory.path().join("not-a-directory");
        fs::write(&blocking_file, "file").unwrap();
        let config = Config::new(
            Paths::for_test(blocking_file.join("logs")),
            BuildInfo::new("test"),
        )
        .with_stderr(false);
        let (handle, worker, layer) = build(config);
        assert!(matches!(handle.file_status(), FileStatus::Unavailable(_)));

        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "moqcast_diagnostics::tests", "memory fallback active");
        });
        assert_eq!(handle.snapshot().entries().len(), 1);
        assert!(matches!(
            handle.export(ExportRequest::new(directory.path().join("logs.zip"))),
            Err(ExportError::FileOutputUnavailable)
        ));

        drop(worker);
    }

    #[test]
    fn oversized_unicode_message_and_field_are_utf8_safe_and_bounded() {
        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        let huge = "中文🙂\n".repeat(20_000);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                target: "moqcast_diagnostics::tests",
                detail = huge.as_str(),
                "{huge}"
            );
        });
        drop(worker);

        let snapshot = handle.snapshot();
        let entry = &snapshot.entries()[0];
        assert!(entry.event().ends_with(TRUNCATED_SUFFIX));
        assert!(entry.event().len() <= MAX_RENDERED_EVENT_BYTES);
        assert!(entry.line().len() <= MAX_LOG_LINE_BYTES);
        assert!(std::str::from_utf8(entry.event().as_bytes()).is_ok());
        assert!(std::str::from_utf8(entry.line().as_bytes()).is_ok());

        let file = fs::read(log_dir(&handle).join("moqcast.log")).unwrap();
        assert!(file.len() <= MAX_LOG_LINE_BYTES);
        assert!(std::str::from_utf8(&file).is_ok());
    }

    #[test]
    fn runtime_toggle_rebuilds_interest_and_captures_allowed_debug_events() {
        fn emit_debug() {
            tracing::debug!(target: "moqcast_diagnostics::tests", "runtime detail");
        }

        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            emit_debug();
            assert!(handle.snapshot().entries().is_empty());
            handle.set_detailed(true);
            emit_debug();
        });
        drop(worker);

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.entries().len(), 1);
        assert_eq!(snapshot.entries()[0].event(), "runtime detail");
    }

    #[test]
    fn export_barrier_includes_queued_event_without_dropping_worker_guard() {
        use std::io::Read as _;

        use zip::ZipArchive;

        let directory = tempdir().unwrap();
        let (handle, worker, layer) = build(config(directory.path()));
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "moqcast_diagnostics::tests", "queued before export");
        });

        let destination = directory.path().join("export.zip");
        handle.export(ExportRequest::new(&destination)).unwrap();
        let mut archive = ZipArchive::new(fs::File::open(destination).unwrap()).unwrap();
        let mut log = String::new();
        archive
            .by_name("moqcast.log")
            .unwrap()
            .read_to_string(&mut log)
            .unwrap();
        assert!(log.contains("queued before export"));

        drop(worker);
    }

    #[test]
    fn manifest_has_no_network_or_upload_capability() {
        let manifest = include_str!("../Cargo.toml").to_ascii_lowercase();
        for forbidden in [
            "reqwest",
            "ureq",
            "hyper =",
            "http://",
            "https://",
            "upload",
            "telemetry",
            "tracing-appender",
        ] {
            assert!(!manifest.contains(forbidden), "found {forbidden}");
        }
    }
}
