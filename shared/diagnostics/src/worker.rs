use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::config::BuildInfo;
use crate::export::{self, ExportError, ExportRequest, ExportResult};

pub(crate) struct ExportContext {
    pub(crate) log_dir: std::path::PathBuf,
    pub(crate) build: BuildInfo,
    pub(crate) minimal_export_metadata: bool,
    pub(crate) owner_only_file_permissions: bool,
    pub(crate) active_filter: String,
    pub(crate) losses: export::Losses,
}

enum Command {
    Line {
        bytes: Vec<u8>,
        detail: bool,
    },
    Export {
        context: Box<ExportContext>,
        request: ExportRequest,
        acknowledge: Sender<Result<ExportResult, ExportError>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct Writer {
    commands: Sender<Command>,
    dropped: Arc<AtomicU64>,
    detail_pending: Arc<AtomicUsize>,
    detail_capacity: usize,
    detail_dropped: Arc<AtomicU64>,
}

impl Writer {
    pub(crate) fn disabled() -> Self {
        let (commands, receiver) = bounded(1);
        drop(receiver);
        Self {
            commands,
            dropped: Arc::new(AtomicU64::new(0)),
            detail_pending: Arc::new(AtomicUsize::new(0)),
            detail_capacity: 1,
            detail_dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn spawn(
        output: Box<dyn Write + Send>,
        details: Option<Box<dyn Write + Send>>,
        capacity: usize,
    ) -> io::Result<(Self, WorkerGuard)> {
        let capacity = capacity.max(2);
        let (commands, receiver) = bounded(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let detail_pending = Arc::new(AtomicUsize::new(0));
        let pending = detail_pending.clone();
        let worker = thread::Builder::new()
            .name("moqcast-diagnostics".to_owned())
            .spawn(move || run(receiver, output, details, pending))?;
        let writer = Self {
            commands: commands.clone(),
            dropped,
            detail_pending,
            detail_capacity: capacity / 2,
            detail_dropped: Arc::new(AtomicU64::new(0)),
        };
        Ok((
            writer,
            WorkerGuard {
                commands,
                worker: Some(worker),
            },
        ))
    }

    pub(crate) fn write_line(&self, line: &[u8]) {
        self.enqueue(line, false);
    }

    pub(crate) fn write_detail(&self, line: &[u8]) {
        self.enqueue(line, true);
    }

    fn enqueue(&self, line: &[u8], detail: bool) {
        // Reserve half of the existing queue for lifecycle events. Use one FIFO
        // so export and shutdown barriers still cover both streams in order.
        if detail
            && self
                .detail_pending
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
                    (pending < self.detail_capacity).then_some(pending + 1)
                })
                .is_err()
        {
            self.count_drop(true);
            return;
        }
        match self.commands.try_send(Command::Line {
            bytes: line.to_vec(),
            detail,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                if detail {
                    self.detail_pending.fetch_sub(1, Ordering::Relaxed);
                }
                self.count_drop(detail);
            }
        }
    }

    fn count_drop(&self, detail: bool) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        if detail {
            self.detail_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn export(
        &self,
        context: ExportContext,
        request: ExportRequest,
    ) -> Result<ExportResult, ExportError> {
        let (acknowledge, acknowledged) = bounded(0);
        self.commands
            .send(Command::Export {
                context: Box::new(context),
                request,
                acknowledge,
            })
            .map_err(|_| {
                ExportError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "diagnostics worker stopped",
                ))
            })?;
        acknowledged.recv().map_err(|_| {
            ExportError::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostics worker stopped",
            ))
        })?
    }

    pub(crate) fn losses(&self, ring: u64) -> export::Losses {
        // Load the subset first so concurrent producers cannot make it exceed
        // the subsequently observed total. This is an observational snapshot.
        let detail_queue = self.detail_dropped.load(Ordering::Relaxed);
        export::Losses {
            queue: self.dropped(),
            detail_queue,
            ring,
        }
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

pub(crate) struct WorkerGuard {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone)]
struct OutputFailure {
    kind: io::ErrorKind,
    message: String,
}

impl OutputFailure {
    fn from_error(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }

    fn to_error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

fn run(
    receiver: Receiver<Command>,
    mut output: Box<dyn Write + Send>,
    mut details: Option<Box<dyn Write + Send>>,
    pending: Arc<AtomicUsize>,
) {
    let mut failure = None;
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Line { bytes, detail } => {
                if detail {
                    pending.fetch_sub(1, Ordering::Relaxed);
                }
                let sink = if detail {
                    details.as_mut().unwrap_or(&mut output)
                } else {
                    &mut output
                };
                if let Err(error) = sink.write_all(&bytes) {
                    failure = Some(OutputFailure::from_error(io::Error::new(
                        error.kind(),
                        format!(
                            "{} log output failed: {error}",
                            if detail { "detail" } else { "lifecycle" }
                        ),
                    )));
                }
            }
            Command::Export {
                context,
                request,
                acknowledge,
            } => {
                let result = output
                    .flush()
                    .and_then(|()| details.as_mut().map_or(Ok(()), |sink| sink.flush()))
                    .map_err(ExportError::Io)
                    .and_then(|()| match &failure {
                        Some(failure) => Err(ExportError::Io(failure.to_error())),
                        None => export::export(
                            &context.log_dir,
                            &context.build,
                            context.minimal_export_metadata,
                            context.owner_only_file_permissions,
                            &context.active_filter,
                            context.losses,
                            request,
                        ),
                    });
                let _ = acknowledge.send(result);
            }
            Command::Shutdown => {
                let _ = output.flush();
                if let Some(sink) = &mut details {
                    let _ = sink.flush();
                }
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::sync::mpsc;

    use tempfile::tempdir;
    use zip::ZipArchive;

    use super::*;

    #[test]
    fn full_line_queue_drops_only_line_commands() {
        let (commands, _receiver) = bounded(1);
        let writer = Writer {
            commands,
            dropped: Arc::new(AtomicU64::new(0)),
            detail_pending: Arc::new(AtomicUsize::new(0)),
            detail_capacity: 1,
            detail_dropped: Arc::new(AtomicU64::new(0)),
        };

        writer.write_line(b"first\n");
        writer.write_line(b"second\n");

        assert_eq!(writer.dropped(), 1);
    }

    struct BlockingFile {
        file: File,
        started: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }

    impl Write for BlockingFile {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if let Some(started) = self.started.take() {
                let _ = started.send(());
                let _ = self.release.recv();
            }
            self.file.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    #[test]
    fn concurrent_line_pressure_preserves_export_order_and_counts_only_lines() {
        use std::io::Read as _;

        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let output = BlockingFile {
            file: File::create(&active).unwrap(),
            started: Some(started_tx),
            release: release_rx,
        };
        let (writer, guard) = Writer::spawn(Box::new(output), None, 2).unwrap();

        writer.write_line(b"before-one\n");
        started_rx.recv().unwrap();
        writer.write_line(b"before-two\n");
        writer.write_line(b"before-three\n");

        let export_writer = writer.clone();
        let log_dir = directory.path().to_owned();
        let destination = directory.path().join("ordered.zip");
        let export = std::thread::spawn(move || {
            export_writer.export(
                ExportContext {
                    log_dir,
                    build: BuildInfo::new("test"),
                    minimal_export_metadata: false,
                    owner_only_file_permissions: false,
                    active_filter: "base=info; detailed=off".to_owned(),
                    losses: export::Losses::default(),
                },
                ExportRequest::new(destination),
            )
        });

        for _ in 0..32 {
            writer.write_line(b"under-pressure\n");
        }
        assert_eq!(writer.dropped(), 32);
        release_tx.send(()).unwrap();

        let result = export.join().unwrap().unwrap();
        assert_eq!(writer.dropped(), 32);
        let mut archive = ZipArchive::new(File::open(result.path()).unwrap()).unwrap();
        let mut log = String::new();
        archive
            .by_name("moqcast.log")
            .unwrap()
            .read_to_string(&mut log)
            .unwrap();
        assert_eq!(log, "before-one\nbefore-two\nbefore-three\n");
        assert_eq!(fs::read_to_string(active).unwrap(), log);

        drop(guard);
    }
    fn context(path: &std::path::Path, losses: export::Losses) -> ExportContext {
        ExportContext {
            log_dir: path.to_owned(),
            build: BuildInfo::new("test"),
            minimal_export_metadata: false,
            owner_only_file_permissions: false,
            active_filter: "detailed=on".into(),
            losses,
        }
    }

    #[test]
    fn detailed_queue_pressure_reserves_lifecycle_capacity_and_exports_both_streams() {
        use std::io::Read as _;
        let directory = tempdir().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let output = BlockingFile {
            file: File::create(directory.path().join("moqcast.log")).unwrap(),
            started: Some(started_tx),
            release: release_rx,
        };
        let details = File::create(directory.path().join("moqcast-detail.log")).unwrap();
        let (writer, guard) = Writer::spawn(Box::new(output), Some(Box::new(details)), 4).unwrap();
        writer.write_line(b"blocking-write\n");
        started_rx.recv().unwrap();
        for _ in 0..32 {
            writer.write_detail(b"packet detail\n");
        }
        writer.write_line(b"connection-established\n");
        writer.write_line(b"playback-started\n");
        let losses = writer.losses(0);
        // Unblock before assertions so a failure cannot hang WorkerGuard::drop.
        release_tx.send(()).unwrap();
        let path = directory.path().join("pressure.zip");
        writer
            .export(context(directory.path(), losses), ExportRequest::new(&path))
            .unwrap();
        let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut key = String::new();
        archive
            .by_name("moqcast.log")
            .unwrap()
            .read_to_string(&mut key)
            .unwrap();
        assert_eq!(
            key,
            "blocking-write\nconnection-established\nplayback-started\n"
        );
        let mut detail = String::new();
        archive
            .by_name("moqcast-detail.log")
            .unwrap()
            .read_to_string(&mut detail)
            .unwrap();
        assert_eq!(detail.lines().count(), 2);
        let mut environment = String::new();
        archive
            .by_name("environment.txt")
            .unwrap()
            .read_to_string(&mut environment)
            .unwrap();
        assert!(environment.contains("detail_queue_dropped=30\n"));
        assert!(environment.contains("lifecycle_queue_dropped=0\n"));
        assert!(environment.contains("output_errors=0\n"));
        drop(guard);
    }

    struct FailingOutput {
        flush_only: bool,
    }
    impl Write for FailingOutput {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if self.flush_only {
                Ok(buffer.len())
            } else {
                Err(io::Error::other("injected disk full"))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.flush_only {
                Err(io::Error::other("injected flush failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn output_failures_are_not_reported_as_queue_loss_or_successful_exports() {
        for detail in [false, true] {
            for flush_only in [false, true] {
                let directory = tempdir().unwrap();
                let good = Box::new(File::create(directory.path().join("moqcast.log")).unwrap())
                    as Box<dyn Write + Send>;
                let bad = Box::new(FailingOutput { flush_only }) as Box<dyn Write + Send>;
                let (main, raw) = if detail { (good, bad) } else { (bad, good) };
                let (writer, guard) = Writer::spawn(main, Some(raw), 8).unwrap();
                writer.write_line(b"lifecycle\n");
                writer.write_detail(b"detail\n");
                let destination = directory.path().join("failure.zip");
                let result = writer.export(
                    context(directory.path(), writer.losses(0)),
                    ExportRequest::new(&destination),
                );
                assert!(matches!(result, Err(ExportError::Io(_))));
                assert_eq!(writer.dropped(), 0, "I/O errors are not queue rejection");
                assert!(
                    !destination.exists(),
                    "must not export a misleading healthy archive"
                );
                drop(guard);
            }
        }
    }
    struct ShutdownOutput {
        flushed: Arc<AtomicUsize>,
        released: Arc<AtomicUsize>,
        fail_flush: bool,
    }

    impl Write for ShutdownOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed.fetch_add(1, Ordering::Relaxed);
            if self.fail_flush {
                Err(io::Error::other("injected shutdown flush failure"))
            } else {
                Ok(())
            }
        }
    }
    impl Drop for ShutdownOutput {
        fn drop(&mut self) {
            self.released.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn shutdown_flushes_and_releases_both_outputs_even_when_one_flush_fails() {
        for failed_stream in 0..2 {
            let flushed = Arc::new(AtomicUsize::new(0));
            let released = Arc::new(AtomicUsize::new(0));
            let sink = |stream| {
                Box::new(ShutdownOutput {
                    flushed: flushed.clone(),
                    released: released.clone(),
                    fail_flush: stream == failed_stream,
                }) as Box<dyn Write + Send>
            };
            let (writer, guard) = Writer::spawn(sink(0), Some(sink(1)), 8).unwrap();
            writer.write_line(b"last lifecycle\n");
            writer.write_detail(b"last detail\n");
            drop(guard);
            assert_eq!(flushed.load(Ordering::Relaxed), 2);
            assert_eq!(released.load(Ordering::Relaxed), 2);
            assert_eq!(writer.detail_pending.load(Ordering::Relaxed), 0);
            writer.write_detail(b"after shutdown\n");
            assert_eq!(writer.detail_pending.load(Ordering::Relaxed), 0);
            assert_eq!(writer.losses(0).detail_queue, 1);
        }
    }
}
