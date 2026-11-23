//! Process-local media observations for the explicitly requested local export.
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use tracing::field::{Field, Visit};

static HISTORY: LazyLock<Mutex<History>> = LazyLock::new(Default::default);
static NEXT_SOFTWARE: AtomicBool = AtomicBool::new(false);

/// Select software decoding for the next watch in this process.
pub fn set_next_software(value: bool) {
    NEXT_SOFTWARE.store(value, Ordering::Relaxed);
}
/// Read the preference without changing a running watch.
pub fn next_software() -> bool {
    NEXT_SOFTWARE.load(Ordering::Relaxed)
}

/// Coarse source classification, safe to include in a local summary.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// A display selected for publication.
    Display,
    /// A window selected for publication.
    Window,
    /// A remote screen selected for watching.
    Nearby,
}
impl Source {
    fn label(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Window => "window",
            Self::Nearby => "nearby",
        }
    }
}

fn now() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Allowlisted observations for one local media session.
#[derive(Clone, Debug)]
pub struct Record {
    id: u64,
    generation: u64,
    source: Source,
    /// Requested backend selection at session start.
    pub policy: &'static str,
    /// Known codec family; never a raw catalog field.
    pub codec: Option<&'static str>,
    /// Allowlisted name of an encoder or decoder actually opened.
    pub backend: Option<&'static str>,
    /// Known dimensions from the selected source or rendition.
    pub dimensions: Option<(u32, u32)>,
    /// A scoped SDK observation of hardware refusal for this decoder.
    pub hardware_refused: bool,
    no_software_decoder: bool,
    observed_backend: bool,
    opens: u32,
    started: i64,
    observed: i64,
    status: &'static str,
}

impl Record {
    /// A known implementation class. A platform wrapper alone cannot establish execution.
    pub fn execution(&self) -> Option<&'static str> {
        match self.backend? {
            "openh264" | "vpx" => Some("software"),
            "nvenc" | "nvdec" | "vaapi" => Some("hardware"),
            // The publishing SDK enumerates hardware MFTs. Accepting a D3D manager
            // on the decode side supplies no per-session acceleration verdict.
            "mediafoundation" if !matches!(self.source, Source::Nearby) => Some("hardware"),
            _ => None,
        }
    }
    /// Source category without user-provided labels.
    pub fn source(&self) -> &'static str {
        self.source.label()
    }
    /// Whether a codec instance has supplied an observation.
    pub fn awaiting_backend(&self) -> bool {
        !self.observed_backend && self.status == "running"
    }
    fn json(&self) -> Value {
        json!({"session_id":self.id,"role": if matches!(self.source,Source::Nearby) {"watch"} else {"publish"},
        "source":self.source(),"started_ms_utc":self.started,"observed_ms_utc":self.observed,
        "status":self.status,"requested":self.policy,"codec":self.codec,"backend":self.backend,
        "execution":self.execution(),"dimensions":self.dimensions,"backend_opens":self.opens,
        "backend_availability":if self.backend.is_some() {"observed"} else if self.awaiting_backend() {"pending"} else {"unavailable"},
        "failure_reason":self.no_software_decoder.then_some("no_software_decoder"),
        "failure_stage": if self.status == "failed" { Some(if self.opens == 0 { "before_first_backend" } else { "after_backend_open" }) } else { None },
        "fallback_reason":self.hardware_refused.then_some("hardware_candidates_refused")})
    }
}

#[derive(Default)]
struct History {
    serial: u64,
    current: Option<Record>,
    ended: Option<Record>,
}
impl History {
    fn start(&mut self, generation: u64, source: Source, policy: &'static str) -> u64 {
        if let Some(old) = self.current.as_ref() {
            self.end(old.id, "replaced");
        }
        self.serial += 1;
        let time = now();
        self.current = Some(Record {
            id: self.serial,
            generation,
            source,
            policy,
            codec: if matches!(source, Source::Nearby) {
                None
            } else {
                Some("H.264")
            },
            backend: None,
            dimensions: None,
            hardware_refused: false,
            no_software_decoder: false,
            observed_backend: false,
            opens: 0,
            started: time,
            observed: time,
            status: "running",
        });
        self.serial
    }
    fn end(&mut self, id: u64, status: &'static str) {
        if self.current.as_ref().is_some_and(|r| r.id == id) {
            let mut record = self.current.take().unwrap();
            record.status = status;
            record.observed = now();
            self.ended = Some(record);
        }
    }
    fn observe(&mut self, id: u64, backend: &str) {
        if let Some(record) = self.current.as_mut().filter(|r| r.id == id) {
            record.backend = [
                "openh264",
                "vpx",
                "videotoolbox",
                "mediafoundation",
                "nvenc",
                "nvdec",
                "vaapi",
                "v4l2",
                "mediacodec",
            ]
            .into_iter()
            .find(|s| *s == backend);
            record.observed_backend = true;
            record.opens = record.opens.saturating_add(1);
            record.observed = now();
        }
    }
}

/// Owner of one measurement interval. Dropping an unfinished guard records cancellation, or failure during panic unwinding.
#[derive(Debug)]
pub struct Session {
    id: u64,
    generation: u64,
    software: bool,
}
impl Session {
    /// Freeze the process preference when the watch is accepted.
    pub fn watch(generation: u64) -> Self {
        let software = next_software();
        let id = HISTORY.lock().unwrap().start(
            generation,
            Source::Nearby,
            if software { "software" } else { "auto" },
        );
        Self {
            id,
            generation,
            software,
        }
    }
    /// Begin the publisher record before preparing the requested source.
    pub fn publish(generation: u64, source: Source, hardware_only: bool) -> Self {
        let id = HISTORY.lock().unwrap().start(
            generation,
            source,
            if hardware_only { "hardware" } else { "auto" },
        );
        Self {
            id,
            generation,
            software: false,
        }
    }
    /// Original generation owned by the platform runtime.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Strict policy for this watch, including decoder reconstruction.
    pub fn software(&self) -> bool {
        self.software
    }
    /// Attribute typed capture-driver observations to the publication future.
    pub fn capture_span(&self) -> tracing::Span {
        tracing::info_span!(target:"moqcast_diagnostics::media","publication",media_id=self.id)
    }
    /// Scope SDK selection events to this decoder opening.
    pub fn decoder_span(&self) -> tracing::Span {
        tracing::info_span!(target:"moqcast_diagnostics::media","decoder_open",media_id=self.id)
    }
    /// Record only known dimensions and codec families.
    pub fn format(&self, codec: &str, size: Option<(u32, u32)>) {
        let mut history = HISTORY.lock().unwrap();
        if let Some(r) = history.current.as_mut().filter(|r| r.id == self.id) {
            r.backend = None;
            r.observed_backend = false;
            r.hardware_refused = false;
            r.codec = match codec.split('.').next() {
                Some("avc1" | "avc3" | "H264") => Some("H.264"),
                Some("hev1" | "hvc1") => Some("H.265"),
                _ if codec == "H.264" => Some("H.264"),
                _ => None,
            };
            r.dimensions = size;
            r.observed = now();
        }
    }
    /// Store the first decoded frame size for the selected rendition.
    pub fn frame_size(&self, width: u32, height: u32) {
        if let Some(record) = HISTORY
            .lock()
            .unwrap()
            .current
            .as_mut()
            .filter(|r| r.id == self.id)
        {
            record.dimensions = Some((width, height));
            record.observed = now();
        }
    }
    /// Record a typed NoDecoder result when this session explicitly requested software.
    pub fn no_decoder(&self) {
        if self.software
            && let Some(record) = HISTORY
                .lock()
                .unwrap()
                .current
                .as_mut()
                .filter(|r| r.id == self.id)
        {
            record.no_software_decoder = true;
        }
    }
    /// Record the instance actually opened, independently of the presentation surface.
    pub fn decoder(&self, name: &str) {
        HISTORY.lock().unwrap().observe(self.id, name);
    }
    /// The owner confirmed that this media operation was stopped.
    pub fn stopped(&self) {
        HISTORY.lock().unwrap().end(self.id, "stopped");
    }
    /// Record completion before the runtime discards the result.
    pub fn finish(&self, failed: bool) {
        HISTORY
            .lock()
            .unwrap()
            .end(self.id, if failed { "failed" } else { "ended" });
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let status = if std::thread::panicking() {
            "failed"
        } else {
            "cancelled"
        };
        HISTORY.lock().unwrap().end(self.id, status);
    }
}

/// Read only the record belonging to the displayed watch.
pub fn watch_record(generation: u64) -> Option<Record> {
    HISTORY
        .lock()
        .unwrap()
        .current
        .as_ref()
        .filter(|r| matches!(r.source, Source::Nearby) && r.generation == generation)
        .cloned()
}
/// Whether the displayed terminal watch failed because no strict software decoder opened.
pub fn software_decoder_failed(generation: u64) -> bool {
    HISTORY.lock().unwrap().ended.as_ref().is_some_and(|r| {
        r.generation == generation
            && matches!(r.source, Source::Nearby)
            && r.status == "failed"
            && r.no_software_decoder
    })
}

/// Read the current local publication.
pub fn publication() -> Option<Record> {
    HISTORY
        .lock()
        .unwrap()
        .current
        .as_ref()
        .filter(|r| !matches!(r.source, Source::Nearby))
        .cloned()
}
pub(crate) fn snapshot() -> Value {
    let (current, ended, exported) = {
        let history = HISTORY.lock().unwrap();
        (history.current.clone(), history.ended.clone(), now())
    };
    json!({"version":1, "exported_ms_utc":exported,"current":current.as_ref().map(Record::json),"last_ended":ended.as_ref().map(Record::json)})
}

pub(crate) fn attach_build(summary: &mut Value, build: &crate::BuildInfo) {
    // Correlate exports from the same build without including internal provenance strings.
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    (
        &build.app_version,
        &build.build_identity,
        &build.source_identity,
        &build.dependency_identity,
        &build.os,
    )
        .hash(&mut hasher);
    summary["build_fingerprint"] = json!(format!("{:016x}", hasher.finish()));
    summary["app_version"] = json!(build.app_version);
    summary["platform"] = json!(build.os);
}
#[derive(Default)]
pub(crate) struct MediaId(pub u64);
impl Visit for MediaId {
    fn record_u64(&mut self, f: &Field, v: u64) {
        if f.name() == "media_id" {
            self.0 = v;
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}
impl Visit for MediaEncoder {
    fn record_str(&mut self, f: &Field, v: &str) {
        if f.name() == "encoder" {
            HISTORY.lock().unwrap().observe(self.0, v);
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}
pub(crate) struct MediaEncoder(pub u64);

pub(crate) fn decoder_fallback(id: u64) {
    if let Some(r) = HISTORY
        .lock()
        .unwrap()
        .current
        .as_mut()
        .filter(|r| r.id == id && matches!(r.source, Source::Nearby))
    {
        r.hardware_refused = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildInfo, Config, ExportRequest, Paths};
    use std::io::Read;
    use tracing_subscriber::prelude::*;

    // Media history is process-wide in the application and in this unit-test binary.
    static SESSION_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn delayed_zip_write_keeps_the_session_at_destination_confirmation() {
        let _lock = SESSION_TEST.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let config =
            Config::new(Paths::for_test(temp.path()), BuildInfo::new("test")).with_stderr(false);
        let (handle, _worker, _layer) = crate::build(config);
        let first = Session::watch(30);
        first.decoder("openh264");
        let confirmed = handle.prepare_export(temp.path().join("confirmed.zip"));

        first.stopped();
        let second = Session::watch(31);
        second.decoder("nvdec");
        let written = std::thread::spawn(confirmed).join().unwrap().unwrap();
        let mut archive =
            zip::ZipArchive::new(std::fs::File::open(written.path()).unwrap()).unwrap();
        let summary: Value =
            serde_json::from_reader(archive.by_name("media-session.json").unwrap()).unwrap();
        assert_eq!(summary["current"]["session_id"], first.id);
        assert_eq!(summary["current"]["status"], "running");
        assert_eq!(summary["current"]["backend"], "openh264");
        assert_eq!(watch_record(31).unwrap().backend, Some("nvdec"));
    }

    #[test]
    fn session_observations_are_scoped_latched_and_exported_at_request_time() {
        let _lock = SESSION_TEST.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let build = BuildInfo::new("1.0.0").with_build_identity("private-test-build");
        let config = Config::new(Paths::for_test(temp.path()), build)
            .with_minimal_export_metadata()
            .with_private_location_redaction()
            .with_stderr(false);
        let (handle, _worker, layer) = crate::build(config);
        let subscriber = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
        let read = |name: &str| {
            let mut zip =
                zip::ZipArchive::new(std::fs::File::open(temp.path().join(name)).unwrap()).unwrap();
            let mut text = String::new();
            zip.by_name("media-session.json")
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            for private in [
                "secret-window-title",
                "token=TEST",
                "private-test-build",
                "10.1.2.3",
                "moqcast.screen/",
            ] {
                assert!(!text.contains(private));
            }
            serde_json::from_str::<Value>(&text).unwrap()
        };
        let old = Session::publish(10, Source::Window, false);
        assert!(publication().unwrap().awaiting_backend());
        tracing::dispatcher::with_default(&subscriber, || {
            let event = || tracing::info!(target:"moq_video::encode::producer", encoder="openh264", device="secret-window-title", "capture");
            event();
            assert!(publication().unwrap().awaiting_backend());
            old.capture_span().in_scope(event);
        });
        assert_eq!(publication().unwrap().execution(), Some("software"));
        let running = ExportRequest::new(temp.path().join("running.zip"));
        old.stopped();
        set_next_software(false);
        let first = Session::watch(11);
        set_next_software(true);
        tracing::dispatcher::with_default(&subscriber, || {
            first.decoder_span().in_scope(|| {
                tracing::warn!(target:"moq_video::decode::backend",
                decoder="openh264", refused="secret-window-title", "selection")
            });
        });
        first.decoder("openh264");
        assert!(!first.software());
        first.finish(false);
        let second = Session::watch(12);
        assert!(second.software());
        assert!(watch_record(11).is_none());
        first.decoder("mediafoundation");
        first.finish(true);
        assert!(watch_record(12).unwrap().awaiting_backend());
        second.format("avc1.42c01e", Some((1280, 720)));
        tracing::dispatcher::with_default(&subscriber, || {
            let warning = || {
                tracing::warn!(target:"moq_video::decode::backend",
                decoder="openh264", refused="private-probe-error", "selection")
            };
            // The non-Apple SDK opens the decoder on its own uninstrumented thread.
            second.decoder_span().in_scope(|| {
                std::thread::scope(|threads| {
                    threads.spawn(|| tracing::dispatcher::with_default(&subscriber, warning));
                });
            });
            first.decoder_span().in_scope(warning);
            second.capture_span().in_scope(warning);
        });
        assert!(!watch_record(12).unwrap().hardware_refused);
        second.decoder("openh264");
        set_next_software(false);
        assert!(second.software());
        handle.export(running).unwrap();
        handle
            .export(ExportRequest::new(temp.path().join("pair.zip")))
            .unwrap();
        let a = read("running.zip");
        assert_eq!(a["current"]["role"], "publish");
        assert_eq!(a["current"]["status"], "running");
        assert_eq!(a["current"]["backend"], "openh264");
        let pair = read("pair.zip");
        assert_ne!(
            pair["current"]["session_id"],
            pair["last_ended"]["session_id"]
        );
        assert_eq!(pair["current"]["requested"], "software");
        assert_eq!(pair["last_ended"]["requested"], "auto");
        assert_eq!(
            pair["last_ended"]["fallback_reason"],
            "hardware_candidates_refused"
        );
        assert!(pair["current"]["fallback_reason"].is_null());
        assert_eq!(pair["current"]["backend"], pair["last_ended"]["backend"]);
        second.decoder("nvdec");
        assert_eq!(watch_record(12).unwrap().execution(), Some("hardware"));
        second.decoder("mediafoundation");
        assert_eq!(watch_record(12).unwrap().execution(), None);
        second.decoder("videotoolbox");
        assert_eq!(watch_record(12).unwrap().execution(), None);
        second.decoder("https://10.1.2.3/moqcast.screen/token=TEST");
        let record = watch_record(12).unwrap();
        assert!(record.backend.is_none() && !record.awaiting_backend());
        assert_eq!(record.opens, 5);
        drop(second);
        assert_eq!(snapshot()["last_ended"]["status"], "cancelled");
        set_next_software(true);
        let failed = Session::watch(13);
        set_next_software(false);
        failed.no_decoder();
        failed.finish(true);
        handle
            .export(ExportRequest::new(temp.path().join("failed.zip")))
            .unwrap();
        let failure = read("failed.zip");
        assert!(software_decoder_failed(13));
        assert!(!software_decoder_failed(12));
        assert_eq!(
            failure["last_ended"]["failure_reason"],
            "no_software_decoder"
        );
        assert_eq!(failure["version"], 1);
        assert!(failure["current"].is_null());
        assert_eq!(
            failure["last_ended"]["failure_stage"],
            "before_first_backend"
        );
        assert_eq!(failure["last_ended"]["backend_availability"], "unavailable");
        assert_eq!(failure["build_fingerprint"], pair["build_fingerprint"]);
    }
}
