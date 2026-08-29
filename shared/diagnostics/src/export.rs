use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;
use time::OffsetDateTime;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::config::{ACTIVE_LOG_NAME, BuildInfo};

/// A user-selected local destination for a diagnostic archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportRequest {
    destination: PathBuf,
}

impl ExportRequest {
    /// Create a local export request for an explicit destination.
    pub fn new(destination: impl Into<PathBuf>) -> Self {
        Self {
            destination: destination.into(),
        }
    }

    /// Return the requested local archive path.
    pub fn destination(&self) -> &Path {
        &self.destination
    }
}

/// The result of writing one local diagnostic archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportResult {
    path: PathBuf,
    log_files: usize,
}

impl ExportResult {
    /// Return the archive written to the selected destination.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the number of current and rotated log files included.
    pub fn log_files(&self) -> usize {
        self.log_files
    }
}

/// Failure to build a local diagnostic archive.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Persistent file diagnostics are unavailable for export.
    #[error("persistent local diagnostics are unavailable")]
    FileOutputUnavailable,
    /// The selected destination would overwrite a current or rotated log.
    #[error("local log export destination overlaps a diagnostics log")]
    DestinationOverlapsLog,
    /// A local filesystem operation failed.
    #[error("local log export I/O failed: {0}")]
    Io(#[from] io::Error),
    /// ZIP archive construction failed.
    #[error("local log ZIP export failed: {0}")]
    Zip(#[from] zip::result::ZipError),
}

pub(crate) fn recommended_name(now: OffsetDateTime) -> String {
    format!(
        "moqcast-logs-{:04}{:02}{:02}-{:02}{:02}{:02}.zip",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

pub(crate) fn export(
    log_dir: &Path,
    build: &BuildInfo,
    active_filter: &str,
    dropped: u64,
    request: ExportRequest,
) -> Result<ExportResult, ExportError> {
    let now = OffsetDateTime::now_utc();
    let logs = collect_logs(log_dir)?;
    let destination = request.destination;
    if destination_overlaps_logs(log_dir, &destination, &logs) {
        return Err(ExportError::DestinationOverlapsLog);
    }
    let file = File::create(&destination)?;
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o600);

    for path in &logs {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        archive.start_file(name, options)?;
        let mut source = File::open(path)?;
        io::copy(&mut source, &mut archive)?;
    }
    archive.start_file("environment.txt", options)?;
    archive.write_all(environment(build, active_filter, dropped, now).as_bytes())?;
    archive.finish()?;

    Ok(ExportResult {
        path: destination,
        log_files: logs.len(),
    })
}

fn destination_overlaps_logs(log_dir: &Path, destination: &Path, logs: &[PathBuf]) -> bool {
    if destination == log_dir || logs.iter().any(|log| log == destination) {
        return true;
    }

    let destination_parent = destination
        .parent()
        .and_then(|parent| fs::canonicalize(parent).ok());
    let canonical_log_dir = fs::canonicalize(log_dir).ok();
    let reserved_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_log_name);
    if reserved_name && destination_parent == canonical_log_dir {
        return true;
    }

    fs::canonicalize(destination).is_ok_and(|destination| {
        logs.iter()
            .filter_map(|log| fs::canonicalize(log).ok())
            .any(|log| log == destination)
    })
}

fn is_log_name(name: &str) -> bool {
    name == ACTIVE_LOG_NAME
        || name
            .strip_prefix(&format!("{ACTIVE_LOG_NAME}."))
            .is_some_and(|suffix| suffix.parse::<usize>().is_ok())
}

fn collect_logs(log_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut logs = Vec::new();
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_log_name(&name) {
            logs.push(entry.path());
        }
    }
    logs.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(logs)
}

fn environment(
    build: &BuildInfo,
    active_filter: &str,
    dropped: u64,
    now: OffsetDateTime,
) -> String {
    let timestamp = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    format!(
        "app_version={}\nbuild_identity={}\nsource_identity={}\ndependency_identity={}\nos={}\nutc_time={}\nactive_filter={}\ndropped_diagnostics={}\n",
        one_line(&build.app_version),
        one_line(&build.build_identity),
        one_line(&build.source_identity),
        one_line(&build.dependency_identity),
        one_line(&build.os),
        timestamp,
        one_line(active_filter),
        dropped
    )
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use tempfile::tempdir;
    use zip::ZipArchive;

    use super::*;

    #[test]
    fn export_contains_logs_and_minimal_environment() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join(ACTIVE_LOG_NAME), "中文日志\n").unwrap();
        fs::write(directory.path().join("moqcast.log.1"), "older\n").unwrap();
        let destination = directory.path().join("export.zip");
        let build = BuildInfo::new("0.4.1-dev.2")
            .with_build_identity("ubuntu22.04")
            .with_source_identity("abcdef123456")
            .with_dependency_identity("moq-dev/moq@81d39f7bf04c82aae324a9ee4251b7f8aa08fb53")
            .with_os("linux-x86_64");

        let result = export(
            directory.path(),
            &build,
            "base=info; detailed=off",
            3,
            ExportRequest::new(&destination),
        )
        .unwrap();
        assert_eq!(result.log_files(), 2);

        let mut archive = ZipArchive::new(File::open(destination).unwrap()).unwrap();
        assert!(archive.by_name(ACTIVE_LOG_NAME).is_ok());
        assert!(archive.by_name("moqcast.log.1").is_ok());
        let mut environment = String::new();
        archive
            .by_name("environment.txt")
            .unwrap()
            .read_to_string(&mut environment)
            .unwrap();
        assert!(environment.contains("app_version=0.4.1-dev.2"));
        assert!(
            environment.contains(
                "dependency_identity=moq-dev/moq@81d39f7bf04c82aae324a9ee4251b7f8aa08fb53"
            )
        );
        assert!(environment.contains("active_filter=base=info; detailed=off"));
        assert!(environment.contains("dropped_diagnostics=3"));
        for forbidden in ["credential", "fingerprint", "authorization", "token="] {
            assert!(!environment.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn export_name_uses_utc_timestamp_shape() {
        let now = OffsetDateTime::from_unix_timestamp(1_787_616_000).unwrap();
        let name = recommended_name(now);
        assert!(name.starts_with("moqcast-logs-20"));
        assert!(name.ends_with(".zip"));
        assert_eq!(name.len(), "moqcast-logs-YYYYMMDD-HHMMSS.zip".len());
    }

    #[test]
    fn export_rejects_current_rotated_and_directory_destinations_without_truncating() {
        let directory = tempdir().unwrap();
        let active = directory.path().join(ACTIVE_LOG_NAME);
        let rotated = directory.path().join("moqcast.log.1");
        fs::write(&active, "current\n").unwrap();
        fs::write(&rotated, "older\n").unwrap();
        let build = BuildInfo::new("test");

        for destination in [&active, &rotated, directory.path()] {
            let error = export(
                directory.path(),
                &build,
                "base=info; detailed=off",
                0,
                ExportRequest::new(destination),
            )
            .unwrap_err();
            assert!(matches!(error, ExportError::DestinationOverlapsLog));
        }

        assert_eq!(fs::read_to_string(active).unwrap(), "current\n");
        assert_eq!(fs::read_to_string(rotated).unwrap(), "older\n");
    }
}
