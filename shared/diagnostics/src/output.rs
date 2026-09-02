use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};

pub(crate) struct RotatingOutput {
    file: Option<FileRotate<AppendCount>>,
    active_path: PathBuf,
    history: usize,
    active_bytes: u64,
    max_file_bytes: u64,
    max_file_age: Option<Duration>,
    active_started: SystemTime,
    owner_only_file_permissions: bool,
    stderr: Option<Box<dyn Write + Send>>,
}

impl RotatingOutput {
    pub(crate) fn new(
        active_path: &Path,
        max_file_bytes: u64,
        max_log_files: usize,
        stderr: bool,
        max_file_age: Option<Duration>,
        owner_only_file_permissions: bool,
    ) -> io::Result<Self> {
        let stderr = stderr.then(|| Box::new(io::stderr()) as Box<dyn Write + Send>);
        Self::with_stderr(
            active_path,
            max_file_bytes,
            max_log_files,
            stderr,
            max_file_age,
            owner_only_file_permissions,
        )
    }

    fn with_stderr(
        active_path: &Path,
        max_file_bytes: u64,
        max_log_files: usize,
        stderr: Option<Box<dyn Write + Send>>,
        max_file_age: Option<Duration>,
        owner_only_file_permissions: bool,
    ) -> io::Result<Self> {
        let log_dir = active_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "log path has no parent directory",
            )
        })?;
        if owner_only_file_permissions {
            set_owner_only_directory(log_dir)?;
        }
        if let Some(max_file_age) = max_file_age {
            prune_stale_logs(log_dir, max_file_age, SystemTime::now())?;
        }
        if owner_only_file_permissions {
            secure_existing_logs(log_dir)?;
        }
        prepare_active_file(active_path, owner_only_file_permissions)?;
        let active_metadata = fs::metadata(active_path)?;
        let active_bytes = active_metadata.len();
        let active_started = active_metadata
            .modified()
            .unwrap_or_else(|_| SystemTime::now());
        let history = max_log_files.saturating_sub(1);
        Ok(Self {
            file: Some(FileRotate::new(
                active_path,
                AppendCount::new(history),
                ContentLimit::None,
                Compression::None,
                None,
            )),
            active_path: active_path.to_owned(),
            history,
            active_bytes,
            max_file_bytes: max_file_bytes.max(1),
            max_file_age,
            active_started,
            owner_only_file_permissions,
            stderr,
        })
    }

    fn reset_expired_active(&mut self, now: SystemTime, max_file_age: Duration) -> io::Result<()> {
        drop(self.file.take());
        let log_dir = self.active_path.parent().expect("validated log path");
        if self.active_path.exists() {
            fs::remove_file(&self.active_path)?;
        }
        prune_stale_logs(log_dir, max_file_age, now)?;
        prepare_active_file(&self.active_path, self.owner_only_file_permissions)?;
        self.file = Some(FileRotate::new(
            &self.active_path,
            AppendCount::new(self.history),
            ContentLimit::None,
            Compression::None,
            None,
        ));
        self.active_bytes = 0;
        self.active_started = now;
        Ok(())
    }

    fn file_mut(&mut self) -> io::Result<&mut FileRotate<AppendCount>> {
        self.file.as_mut().ok_or_else(|| {
            io::Error::other("diagnostics log writer is unavailable after retention failure")
        })
    }
}

impl Write for RotatingOutput {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let now = SystemTime::now();
        if let Some(max_file_age) = self.max_file_age
            && now
                .duration_since(self.active_started)
                .is_ok_and(|age| age >= max_file_age)
        {
            self.reset_expired_active(now, max_file_age)?;
        }
        let incoming = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let size_limit_reached = self.active_bytes > 0
            && self.active_bytes.saturating_add(incoming) > self.max_file_bytes;
        let time_limit_reached = self.max_file_age.is_some_and(|max_file_age| {
            now.duration_since(self.active_started)
                .is_ok_and(|age| age >= rotation_interval(max_file_age))
        });
        if size_limit_reached || time_limit_reached {
            self.file_mut()?.rotate()?;
            if self.owner_only_file_permissions {
                set_owner_only_file(&self.active_path)?;
            }
            self.active_bytes = 0;
            self.active_started = now;
        }
        self.file_mut()?.write_all(buffer)?;
        self.active_bytes = self.active_bytes.saturating_add(incoming);
        if let Some(stderr) = &mut self.stderr {
            let _ = stderr.write_all(buffer);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()?;
        if let Some(stderr) = &mut self.stderr {
            let _ = stderr.flush();
        }
        Ok(())
    }
}

fn rotation_interval(max_file_age: Duration) -> Duration {
    const MAX_ROTATION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
    max_file_age.min(MAX_ROTATION_INTERVAL)
}

fn prepare_active_file(path: &Path, owner_only: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    if owner_only {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    if owner_only {
        set_owner_only_file(path)?;
    }
    drop(file);
    Ok(())
}

fn secure_existing_logs(log_dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && is_log_name(&entry.file_name().to_string_lossy()) {
            set_owner_only_file(&entry.path())?;
        }
    }
    Ok(())
}

fn prune_stale_logs(log_dir: &Path, max_age: Duration, now: SystemTime) -> io::Result<()> {
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || !is_log_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let stale = entry
            .metadata()?
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if stale {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn is_log_name(name: &str) -> bool {
    name == crate::config::ACTIVE_LOG_NAME
        || name
            .strip_prefix(&format!("{}.", crate::config::ACTIVE_LOG_NAME))
            .is_some_and(|suffix| suffix.parse::<usize>().is_ok())
}

#[cfg(unix)]
fn set_owner_only_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_owner_only_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> io::Result<()> {
    Ok(())
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
    use crate::worker::{ExportContext, Writer};

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
        let mut output = RotatingOutput::new(&active, 16, 5, false, None, false).unwrap();

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
        let output = RotatingOutput::with_stderr(
            &active,
            1024,
            5,
            Some(Box::new(FailingStderr)),
            None,
            false,
        )
        .unwrap();
        let (writer, guard) = Writer::spawn(Box::new(output), 8).unwrap();

        writer.write_line(b"persisted\n");
        let destination = directory.path().join("stderr-failed.zip");
        let result = writer
            .export(
                ExportContext {
                    log_dir: directory.path().to_owned(),
                    build: BuildInfo::new("test"),
                    minimal_export_metadata: false,
                    owner_only_file_permissions: false,
                    active_filter: "base=info; detailed=off".to_owned(),
                    dropped: 0,
                },
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

    #[test]
    fn retention_removes_only_stale_matching_log_files() {
        use std::fs::FileTimes;

        let directory = tempdir().unwrap();
        let stale = directory.path().join("moqcast.log.4");
        let recent = directory.path().join("moqcast.log.3");
        let unrelated = directory.path().join("notes.txt");
        fs::write(&stale, "stale\n").unwrap();
        fs::write(&recent, "recent\n").unwrap();
        fs::write(&unrelated, "keep\n").unwrap();
        let now = SystemTime::now();
        let stale_time = now - Duration::from_secs(8 * 24 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_times(FileTimes::new().set_modified(stale_time))
            .unwrap();

        prune_stale_logs(directory.path(), Duration::from_secs(7 * 24 * 60 * 60), now).unwrap();

        assert!(!stale.exists());
        assert!(recent.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn startup_retention_does_not_desynchronize_later_rotations() {
        use std::fs::FileTimes;

        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        let stale = directory.path().join("moqcast.log.4");
        fs::write(&active, "current\n").unwrap();
        fs::write(&stale, "stale\n").unwrap();
        let stale_time = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_times(FileTimes::new().set_modified(stale_time))
            .unwrap();

        let mut output = RotatingOutput::new(
            &active,
            8,
            5,
            false,
            Some(Duration::from_secs(7 * 24 * 60 * 60)),
            false,
        )
        .unwrap();
        for sequence in 0..8 {
            writeln!(output, "line-{sequence}").unwrap();
        }
        output.flush().unwrap();

        let log_count = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_log_name(&entry.file_name().to_string_lossy()))
            .count();
        assert!(log_count <= 5);
    }

    #[test]
    fn retained_logs_rotate_at_least_daily_even_below_the_size_limit() {
        use std::fs::FileTimes;

        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        fs::write(&active, "yesterday\n").unwrap();
        let old_time = SystemTime::now() - Duration::from_secs(25 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&active)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old_time))
            .unwrap();
        let mut output = RotatingOutput::new(
            &active,
            8 * 1024 * 1024,
            5,
            false,
            Some(Duration::from_secs(7 * 24 * 60 * 60)),
            false,
        )
        .unwrap();

        writeln!(output, "today").unwrap();
        output.flush().unwrap();

        assert_eq!(fs::read_to_string(&active).unwrap(), "today\n");
        assert_eq!(
            fs::read_to_string(directory.path().join("moqcast.log.1")).unwrap(),
            "yesterday\n"
        );
    }

    #[test]
    fn first_write_after_retention_discards_expired_active_log() {
        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        let max_age = Duration::from_secs(7 * 24 * 60 * 60);
        let mut output =
            RotatingOutput::new(&active, 8 * 1024 * 1024, 5, false, Some(max_age), false).unwrap();
        writeln!(output, "private-old-line").unwrap();
        output.flush().unwrap();
        let expired_at = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        output.active_started = expired_at;

        writeln!(output, "fresh-line").unwrap();
        output.flush().unwrap();

        assert_eq!(fs::read_to_string(&active).unwrap(), "fresh-line\n");
        let all_logs = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_log_name(&entry.file_name().to_string_lossy()))
            .map(|entry| fs::read_to_string(entry.path()).unwrap())
            .collect::<String>();
        assert!(!all_logs.contains("private-old-line"));
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_policy_sets_directory_and_log_modes() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempdir().unwrap();
        let active = directory.path().join("moqcast.log");
        let mut output = RotatingOutput::new(&active, 1024, 5, false, None, true).unwrap();
        writeln!(output, "private").unwrap();
        output.flush().unwrap();

        assert_eq!(
            fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(active).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
