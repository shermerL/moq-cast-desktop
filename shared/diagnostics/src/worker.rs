use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub(crate) dropped: u64,
}

enum Command {
    Line(Vec<u8>),
    Export {
        context: ExportContext,
        request: ExportRequest,
        acknowledge: Sender<Result<ExportResult, ExportError>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct Writer {
    commands: Sender<Command>,
    dropped: Arc<AtomicU64>,
}

impl Writer {
    pub(crate) fn disabled() -> Self {
        let (commands, receiver) = bounded(1);
        drop(receiver);
        Self {
            commands,
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn spawn(
        output: Box<dyn Write + Send>,
        capacity: usize,
    ) -> io::Result<(Self, WorkerGuard)> {
        let (commands, receiver) = bounded(capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let worker = thread::Builder::new()
            .name("moqcast-diagnostics".to_owned())
            .spawn(move || run(receiver, output))?;
        let writer = Self {
            commands: commands.clone(),
            dropped,
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
        match self.commands.try_send(Command::Line(line.to_vec())) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                let _ =
                    self.dropped
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |dropped| {
                            Some(dropped.saturating_add(1))
                        });
            }
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
                context,
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

fn run(receiver: Receiver<Command>, mut output: Box<dyn Write + Send>) {
    let mut failure = None;
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Line(line) => {
                if let Err(error) = output.write_all(&line) {
                    failure = Some(OutputFailure::from_error(error));
                }
            }
            Command::Export {
                context,
                request,
                acknowledge,
            } => {
                let result =
                    output
                        .flush()
                        .map_err(ExportError::Io)
                        .and_then(|()| match &failure {
                            Some(failure) => Err(ExportError::Io(failure.to_error())),
                            None => export::export(
                                &context.log_dir,
                                &context.build,
                                context.minimal_export_metadata,
                                context.owner_only_file_permissions,
                                &context.active_filter,
                                context.dropped,
                                request,
                            ),
                        });
                let _ = acknowledge.send(result);
            }
            Command::Shutdown => {
                let _ = output.flush();
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
        let (writer, guard) = Writer::spawn(Box::new(output), 1).unwrap();

        writer.write_line(b"before-one\n");
        started_rx.recv().unwrap();
        writer.write_line(b"before-two\n");

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
                    dropped: 0,
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
        assert_eq!(log, "before-one\nbefore-two\n");
        assert_eq!(fs::read_to_string(active).unwrap(), log);

        drop(guard);
    }
}
