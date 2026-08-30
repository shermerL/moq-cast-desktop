use std::fs;
use std::io::{self, Write};
use std::path::Path;

use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};

pub(crate) struct RotatingOutput {
    file: FileRotate<AppendCount>,
    active_bytes: u64,
    max_file_bytes: u64,
    stderr: Option<Box<dyn Write + Send>>,
}

impl RotatingOutput {
    pub(crate) fn new(
        active_path: &Path,
        max_file_bytes: u64,
        max_log_files: usize,
        stderr: bool,
    ) -> io::Result<Self> {
        let stderr = stderr.then(|| Box::new(io::stderr()) as Box<dyn Write + Send>);
        Self::with_stderr(active_path, max_file_bytes, max_log_files, stderr)
    }

    fn with_stderr(
        active_path: &Path,
        max_file_bytes: u64,
        max_log_files: usize,
        stderr: Option<Box<dyn Write + Send>>,
    ) -> io::Result<Self> {
        let active_bytes = fs::metadata(active_path).map_or(0, |metadata| metadata.len());
        let history = max_log_files.saturating_sub(1);
        Ok(Self {
            file: FileRotate::new(
                active_path,
                AppendCount::new(history),
                ContentLimit::None,
                Compression::None,
                None,
            ),
            active_bytes,
            max_file_bytes: max_file_bytes.max(1),
            stderr,
        })
    }
}

impl Write for RotatingOutput {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let incoming = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if self.active_bytes > 0 && self.active_bytes.saturating_add(incoming) > self.max_file_bytes
        {
            self.file.rotate()?;
            self.active_bytes = 0;
        }
        self.file.write_all(buffer)?;
        self.active_bytes = self.active_bytes.saturating_add(incoming);
        if let Some(stderr) = &mut self.stderr {
            let _ = stderr.write_all(buffer);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()?;
        if let Some(stderr) = &mut self.stderr {
            let _ = stderr.flush();
        }
        Ok(())
    }
}

pub(crate) fn fallback(stderr: bool) -> Box<dyn Write + Send> {
    if stderr {
        Box::new(BestEffortStderr(io::stderr()))
    } else {
        Box::new(io::sink())
    }
}

struct BestEffortStderr(io::Stderr);

impl Write for BestEffortStderr {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let _ = self.0.write_all(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.0.flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use std::io::Write;

    use tempfile::tempdir;
    use zip::ZipArchive;

    use super::*;
    use crate::config::BuildInfo;
    use crate::export::ExportRequest;
    use crate::worker::Writer;

    struct FailingStderr;

    impl Write for FailingStderr {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "stderr closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "stderr closed"))
        }
    }

    #[test]
    fn rotation_keeps_five_total_files_and_whole_utf8_lines() {
        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        let mut output = RotatingOutput::new(&active, 16, 5, false).unwrap();

        for sequence in 0..8 {
            writeln!(output, "中文-{sequence}").unwrap();
        }
        output.flush().unwrap();

        let mut logs = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("moqcast.log")
            })
            .collect::<Vec<_>>();
        logs.sort_by_key(|entry| entry.file_name());
        assert_eq!(logs.len(), 5);
        for log in logs {
            let contents = fs::read(log.path()).unwrap();
            assert!(std::str::from_utf8(&contents).is_ok());
        }
    }

    #[test]
    fn stderr_failure_does_not_poison_file_output_or_export() {
        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        let output =
            RotatingOutput::with_stderr(&active, 1024, 5, Some(Box::new(FailingStderr))).unwrap();
        let (writer, guard) = Writer::spawn(Box::new(output), 8).unwrap();

        writer.write_line(b"persisted\n");
        let destination = directory.path().join("stderr-failed.zip");
        let result = writer
            .export(
                directory.path().to_owned(),
                BuildInfo::new("test"),
                "base=info; detailed=off".to_owned(),
                0,
                ExportRequest::new(&destination),
            )
            .unwrap();

        assert_eq!(fs::read_to_string(active).unwrap(), "persisted\n");
        let mut archive = ZipArchive::new(fs::File::open(result.path()).unwrap()).unwrap();
        let mut log = String::new();
        archive
            .by_name("moqcast.log")
            .unwrap()
            .read_to_string(&mut log)
            .unwrap();
        assert_eq!(log, "persisted\n");

        drop(guard);
    }
}
