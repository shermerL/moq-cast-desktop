use std::env;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub(crate) const ACTIVE_LOG_NAME: &str = "moqcast.log";
pub(crate) const DEFAULT_RING_CAPACITY: usize = 1_000;
pub(crate) const DEFAULT_QUEUE_CAPACITY: usize = 4_096;
pub(crate) const DEFAULT_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_LOG_FILES: usize = 5;

/// An operating-system path convention supported by MoQCast diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// XDG state paths used by Linux desktops.
    Linux,
    /// Local application-data paths used by Windows desktops.
    Windows,
}

impl Platform {
    /// Return the path convention for the current build target.
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Resolved filesystem locations used by local diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    log_dir: PathBuf,
}

impl Paths {
    /// Resolve diagnostics paths from the current process environment.
    pub fn discover() -> Result<Self, PathError> {
        Self::for_platform(Platform::current())
    }

    /// Resolve diagnostics paths for a supported platform convention.
    pub fn for_platform(platform: Platform) -> Result<Self, PathError> {
        let roots = EnvironmentRoots {
            xdg_state_home: env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            home: env::var_os("HOME").map(PathBuf::from),
            local_app_data: env::var_os("LOCALAPPDATA").map(PathBuf::from),
        };
        Self::from_roots(platform, roots)
    }

    /// Return the directory containing current and rotated log files.
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    pub(crate) fn active_log(&self) -> PathBuf {
        self.log_dir.join(ACTIVE_LOG_NAME)
    }

    #[cfg(test)]
    pub(crate) fn for_test(log_dir: impl Into<PathBuf>) -> Self {
        Self {
            log_dir: log_dir.into(),
        }
    }

    fn from_roots(platform: Platform, roots: EnvironmentRoots) -> Result<Self, PathError> {
        let log_dir = match platform {
            Platform::Linux => roots
                .xdg_state_home
                .filter(|path| !path.as_os_str().is_empty())
                .or_else(|| roots.home.map(|home| home.join(".local/state")))
                .ok_or(PathError::MissingLinuxStateRoot)?
                .join("moqcast/logs"),
            Platform::Windows => roots
                .local_app_data
                .filter(|path| !path.as_os_str().is_empty())
                .ok_or(PathError::MissingWindowsLocalAppData)?
                .join("MoQCast/logs"),
        };
        Ok(Self { log_dir })
    }
}

#[derive(Default)]
struct EnvironmentRoots {
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
}

/// Failure to resolve a supported local diagnostics directory.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PathError {
    /// Neither XDG_STATE_HOME nor HOME is available for the Linux fallback.
    #[error("XDG_STATE_HOME and HOME are both unavailable")]
    MissingLinuxStateRoot,
    /// LOCALAPPDATA is unavailable for the Windows target directory.
    #[error("LOCALAPPDATA is unavailable")]
    MissingWindowsLocalAppData,
}

/// Availability of persistent local diagnostic files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// File diagnostics are available in this directory.
    Available(PathBuf),
    /// Persistent file diagnostics are unavailable and the application continues.
    Unavailable(String),
}

impl FileStatus {
    /// Return the persistent log directory when file diagnostics are available.
    pub fn directory(&self) -> Option<&Path> {
        match self {
            Self::Available(directory) => Some(directory),
            Self::Unavailable(_) => None,
        }
    }

    /// Return the local initialization failure when file diagnostics are unavailable.
    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable(reason) => Some(reason),
        }
    }
}

/// Build provenance included in local diagnostic exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    pub(crate) app_version: String,
    pub(crate) build_identity: String,
    pub(crate) source_identity: String,
    pub(crate) os: String,
}

impl BuildInfo {
    /// Create build provenance with conservative local defaults.
    pub fn new(app_version: impl Into<String>) -> Self {
        Self {
            app_version: app_version.into(),
            build_identity: "local".to_owned(),
            source_identity: "unknown".to_owned(),
            os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
        }
    }

    /// Set the package or build-variant identity.
    pub fn with_build_identity(mut self, identity: impl Into<String>) -> Self {
        self.build_identity = identity.into();
        self
    }

    /// Set the source revision identity.
    pub fn with_source_identity(mut self, identity: impl Into<String>) -> Self {
        self.source_identity = identity.into();
        self
    }

    /// Set the operating-system description used by an export.
    pub fn with_os(mut self, os: impl Into<String>) -> Self {
        self.os = os.into();
        self
    }
}

/// Local diagnostics initialization settings.
#[derive(Clone, Debug)]
pub struct Config {
    pub(crate) paths: Option<Paths>,
    pub(crate) file_unavailable_reason: Option<String>,
    pub(crate) build: BuildInfo,
    pub(crate) detailed: bool,
    pub(crate) stderr: bool,
    pub(crate) ring_capacity: usize,
    pub(crate) queue_capacity: usize,
    pub(crate) max_file_bytes: u64,
    pub(crate) max_log_files: usize,
}

impl Config {
    /// Create the default bounded local diagnostics configuration.
    pub fn new(paths: Paths, build: BuildInfo) -> Self {
        Self {
            paths: Some(paths),
            file_unavailable_reason: None,
            build,
            detailed: false,
            stderr: true,
            ring_capacity: DEFAULT_RING_CAPACITY,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_log_files: DEFAULT_MAX_LOG_FILES,
        }
    }

    /// Create a bounded memory and stderr configuration without file output.
    pub fn without_file(build: BuildInfo, reason: impl Into<String>) -> Self {
        Self {
            paths: None,
            file_unavailable_reason: Some(reason.into()),
            build,
            detailed: false,
            stderr: true,
            ring_capacity: DEFAULT_RING_CAPACITY,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_log_files: DEFAULT_MAX_LOG_FILES,
        }
    }

    /// Set the initial detailed-diagnostics state.
    pub fn with_detailed(mut self, detailed: bool) -> Self {
        self.detailed = detailed;
        self
    }

    /// Enable or disable mirroring local log lines to stderr.
    pub fn with_stderr(mut self, stderr: bool) -> Self {
        self.stderr = stderr;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_limits(
        mut self,
        max_file_bytes: u64,
        max_log_files: usize,
        ring_capacity: usize,
        queue_capacity: usize,
    ) -> Self {
        self.max_file_bytes = max_file_bytes;
        self.max_log_files = max_log_files;
        self.ring_capacity = ring_capacity;
        self.queue_capacity = queue_capacity;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_prefers_xdg_state_home() {
        let paths = Paths::from_roots(
            Platform::Linux,
            EnvironmentRoots {
                xdg_state_home: Some(PathBuf::from("/state")),
                home: Some(PathBuf::from("/home/person")),
                ..EnvironmentRoots::default()
            },
        )
        .unwrap();

        assert_eq!(paths.log_dir(), Path::new("/state/moqcast/logs"));
    }

    #[test]
    fn linux_falls_back_to_home_local_state() {
        let paths = Paths::from_roots(
            Platform::Linux,
            EnvironmentRoots {
                home: Some(PathBuf::from("/home/person")),
                ..EnvironmentRoots::default()
            },
        )
        .unwrap();

        assert_eq!(
            paths.log_dir(),
            Path::new("/home/person/.local/state/moqcast/logs")
        );
    }

    #[test]
    fn windows_reserves_local_app_data_directory() {
        let paths = Paths::from_roots(
            Platform::Windows,
            EnvironmentRoots {
                local_app_data: Some(PathBuf::from("C:/Users/Test/AppData/Local")),
                ..EnvironmentRoots::default()
            },
        )
        .unwrap();

        assert_eq!(
            paths.log_dir(),
            Path::new("C:/Users/Test/AppData/Local/MoQCast/logs")
        );
    }

    #[test]
    fn defaults_define_eight_mib_and_five_total_log_files() {
        assert_eq!(DEFAULT_MAX_FILE_BYTES, 8 * 1024 * 1024);
        assert_eq!(DEFAULT_MAX_LOG_FILES, 5);
    }
}
