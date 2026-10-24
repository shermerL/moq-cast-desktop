use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use same_file::Handle;
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
    minimal_export_metadata: bool,
    owner_only_file_permissions: bool,
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
    // Keep the checked inputs open so path replacement cannot change what we copy.
    let sources = logs
        .iter()
        .map(Handle::from_path)
        .collect::<io::Result<Vec<_>>>()?;
    let file = create_archive(&destination, owner_only_file_permissions, &sources)?;
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o600);

    for (path, source) in logs.iter().zip(&sources) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        archive.start_file(name, options)?;
        let mut source = source.as_file();
        io::copy(&mut source, &mut archive)?;
    }
    archive.start_file("environment.txt", options)?;
    archive.write_all(
        environment(build, minimal_export_metadata, active_filter, dropped, now).as_bytes(),
    )?;
    archive.finish()?;

    Ok(ExportResult {
        path: destination,
        log_files: logs.len(),
    })
}

fn create_archive(path: &Path, owner_only: bool, sources: &[Handle]) -> Result<File, ExportError> {
    #[cfg(not(unix))]
    let _ = owner_only;
    let mut options = OpenOptions::new();
    // Inspect the opened file, not its path, before any destructive operation.
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    if owner_only {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let destination = Handle::from_file(file.try_clone()?)?;
    if sources.contains(&destination) {
        return Err(ExportError::DestinationOverlapsLog);
    }
    file.set_len(0)?;
    #[cfg(unix)]
    if owner_only {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
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
    minimal_export_metadata: bool,
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
    let mut environment = format!(
        "app_version={}\nos={}\nutc_time={}\nactive_filter={}\ndropped_diagnostics={}\n",
        one_line(&build.app_version),
        one_line(&build.os),
        timestamp,
        one_line(active_filter),
        dropped
    );
    if !minimal_export_metadata {
        environment.push_str(&format!(
            "build_identity={}\nsource_identity={}\ndependency_identity={}\n",
            one_line(&build.build_identity),
            one_line(&build.source_identity),
            one_line(&build.dependency_identity),
        ));
    }
    environment
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
    fn default_export_preserves_existing_build_provenance() {
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
            false,
            false,
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
    fn minimal_export_omits_private_build_provenance_and_build_info_files() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join(ACTIVE_LOG_NAME), "safe log\n").unwrap();
        fs::write(
            directory.path().join("build-info.txt"),
            "private build data\n",
        )
        .unwrap();
        let destination = directory.path().join("minimal.zip");
        let build = BuildInfo::new("0.4.1-dev.2")
            .with_build_identity("macos-universal2")
            .with_source_identity("abcdef123456")
            .with_dependency_identity("moq-dev/moq@private-revision")
            .with_os("macos-aarch64");

        export(
            directory.path(),
            &build,
            true,
            true,
            "base=info; detailed=on",
            2,
            ExportRequest::new(&destination),
        )
        .unwrap();

        let mut archive = ZipArchive::new(File::open(destination).unwrap()).unwrap();
        assert!(archive.by_name(ACTIVE_LOG_NAME).is_ok());
        assert!(archive.by_name("build-info.txt").is_err());
        let mut environment = String::new();
        archive
            .by_name("environment.txt")
            .unwrap()
            .read_to_string(&mut environment)
            .unwrap();
        assert!(environment.contains("app_version=0.4.1-dev.2"));
        assert!(environment.contains("os=macos-aarch64"));
        assert!(environment.contains("utc_time="));
        assert!(environment.contains("active_filter=base=info; detailed=on"));
        assert!(environment.contains("dropped_diagnostics=2"));
        for forbidden in [
            "build_identity",
            "source_identity",
            "dependency_identity",
            "abcdef123456",
            "private-revision",
        ] {
            assert!(!environment.contains(forbidden));
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
                false,
                false,
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

    #[test]
    fn archive_open_rejects_hard_links_without_changing_source_bytes() {
        for name in [ACTIVE_LOG_NAME, "moqcast.log.1"] {
            let directory = tempdir().unwrap();
            let source = directory.path().join(name);
            let destination = directory.path().join("export.zip");
            fs::write(&source, "preserve this log\n").unwrap();
            fs::hard_link(&source, &destination).unwrap();

            let sources = [Handle::from_path(&source).unwrap()];
            let result = create_archive(&destination, true, &sources);
            assert_eq!(fs::read(&source).unwrap(), b"preserve this log\n");
            assert!(matches!(result, Err(ExportError::DestinationOverlapsLog)));
        }
    }

    #[test]
    fn export_rejects_hard_link_destinations_and_preserves_all_logs() {
        for alias_name in [ACTIVE_LOG_NAME, "moqcast.log.1"] {
            for owner_only in [false, true] {
                let directory = tempdir().unwrap();
                let active = directory.path().join(ACTIVE_LOG_NAME);
                let rotated = directory.path().join("moqcast.log.1");
                fs::write(&active, "current\n").unwrap();
                fs::write(&rotated, "older\n").unwrap();
                let destination = directory.path().join("export.zip");
                fs::hard_link(directory.path().join(alias_name), &destination).unwrap();

                let result = export(
                    directory.path(),
                    &BuildInfo::new("test"),
                    false,
                    owner_only,
                    "info",
                    0,
                    ExportRequest::new(&destination),
                );
                assert!(matches!(result, Err(ExportError::DestinationOverlapsLog)));
                assert_eq!(fs::read(&active).unwrap(), b"current\n");
                assert_eq!(fs::read(&rotated).unwrap(), b"older\n");
            }
        }
    }

    #[test]
    fn archive_open_rechecks_identity_after_destination_replacement() {
        let directory = tempdir().unwrap();
        let source = directory.path().join(ACTIVE_LOG_NAME);
        let destination = directory.path().join("export.zip");
        fs::write(&source, "original log\n").unwrap();
        fs::write(&destination, "old archive").unwrap();
        let logs = collect_logs(directory.path()).unwrap();
        assert!(!destination_overlaps_logs(
            directory.path(),
            &destination,
            &logs
        ));
        let sources = [Handle::from_path(&source).unwrap()];

        // Replace the target after the path check; the source handle remains authoritative.
        fs::rename(&destination, directory.path().join("previous.zip")).unwrap();
        fs::hard_link(&source, &destination).unwrap();
        fs::rename(&source, directory.path().join("moved.log")).unwrap();
        fs::write(&source, "replacement log\n").unwrap();

        let result = create_archive(&destination, true, &sources);
        assert!(matches!(result, Err(ExportError::DestinationOverlapsLog)));
        assert_eq!(fs::read(&destination).unwrap(), b"original log\n");
        assert_eq!(fs::read(&source).unwrap(), b"replacement log\n");
    }

    #[test]
    fn export_replaces_independent_archive_without_touching_logs() {
        let directory = tempdir().unwrap();
        let source = directory.path().join(ACTIVE_LOG_NAME);
        let destination = directory.path().join("export.zip");
        fs::write(&source, "keep log\n").unwrap();
        fs::write(&destination, vec![b'x'; 4096]).unwrap();
        export(
            directory.path(),
            &BuildInfo::new("test"),
            false,
            false,
            "info",
            0,
            ExportRequest::new(&destination),
        )
        .unwrap();
        let file = File::open(&destination).unwrap();
        assert!(file.metadata().unwrap().len() < 4096);
        let mut archive = ZipArchive::new(file).unwrap();
        let mut content = String::new();
        archive
            .by_name(ACTIVE_LOG_NAME)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "keep log\n");
        assert_eq!(fs::read(&source).unwrap(), b"keep log\n");
    }

    #[test]
    fn failed_destination_open_preserves_logs() {
        let directory = tempdir().unwrap();
        let source = directory.path().join(ACTIVE_LOG_NAME);
        fs::write(&source, "keep log\n").unwrap();
        let result = export(
            directory.path(),
            &BuildInfo::new("test"),
            false,
            false,
            "info",
            0,
            ExportRequest::new(directory.path().join("missing/export.zip")),
        );
        assert!(matches!(result, Err(ExportError::Io(_))));
        assert_eq!(fs::read(&source).unwrap(), b"keep log\n");
    }

    #[cfg(unix)]
    #[test]
    fn export_rejects_symlink_alias_without_touching_log() {
        let directory = tempdir().unwrap();
        let source = directory.path().join(ACTIVE_LOG_NAME);
        fs::write(&source, "keep log\n").unwrap();
        let destination = directory.path().join("export.zip");
        std::os::unix::fs::symlink(&source, &destination).unwrap();
        let result = export(
            directory.path(),
            &BuildInfo::new("test"),
            false,
            true,
            "info",
            0,
            ExportRequest::new(&destination),
        );
        assert!(matches!(result, Err(ExportError::DestinationOverlapsLog)));
        assert_eq!(fs::read(&source).unwrap(), b"keep log\n");
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_export_sets_outer_archive_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempdir().unwrap();
        fs::write(directory.path().join(ACTIVE_LOG_NAME), "private log\n").unwrap();
        let destination = directory.path().join("private.zip");
        fs::write(&destination, "existing").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o644)).unwrap();

        export(
            directory.path(),
            &BuildInfo::new("test"),
            true,
            true,
            "base=info; detailed=off",
            0,
            ExportRequest::new(&destination),
        )
        .unwrap();

        assert_eq!(
            fs::metadata(destination).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
