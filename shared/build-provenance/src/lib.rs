//! Generates validated build and dependency provenance for desktop applications.

use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use toml_edit::{DocumentMut, Item, Table};

mod vendor;

const BUILD_IDENTITY_ENV: &str = "MOQCAST_BUILD_IDENTITY";
const PROVENANCE_OUTPUT_ENV: &str = "MOQCAST_PROVENANCE_OUTPUT";
const SOURCE_COMMIT_ENV: &str = "MOQCAST_SOURCE_COMMIT";
const GENERATED_RUST_FILE: &str = "moqcast-build-info.rs";
const GENERATED_TEXT_FILE: &str = "moqcast-build-info.txt";

/// A provenance generation failure with a user-actionable explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Generate validated provenance files and Cargo rebuild directives.
pub fn generate() -> Result<(), Error> {
    let manifest_directory = required_path("CARGO_MANIFEST_DIR")?;
    let output_directory = required_path("OUT_DIR")?;
    let target = required_env("TARGET")?;
    let build_identity = env::var(BUILD_IDENTITY_ENV)
        .ok()
        .map(validate_value)
        .transpose()?
        .unwrap_or_else(|| "local".to_owned());
    let explicit_source = env::var(SOURCE_COMMIT_ENV)
        .ok()
        .map(|value| validate_revision(&value))
        .transpose()?;
    let manifest_path = manifest_directory.join("Cargo.toml");
    let lock_path = manifest_directory.join("Cargo.lock");
    let repository_root = manifest_directory
        .parent()
        .ok_or_else(|| Error::new("platform manifest directory has no repository parent"))?;

    emit_rebuild_directives(
        &manifest_directory,
        &manifest_path,
        &lock_path,
        repository_root,
    )?;

    let manifest = read_utf8(&manifest_path)?;
    let lock = read_utf8(&lock_path)?;
    let inputs = DependencyInputs::parse(&manifest, &lock)?;
    let vendor = vendor::Vendor::load(&manifest_directory, &manifest, &lock, &inputs)?;
    let cargo_version = required_env("CARGO_PKG_VERSION")?;
    if inputs.app_version != cargo_version {
        return Err(Error::new(format!(
            "Cargo package version {cargo_version} does not match manifest version {}",
            inputs.app_version
        )));
    }

    let source = SourceIdentity::resolve(repository_root, explicit_source.as_deref())?;
    let provenance = Provenance {
        app_version: inputs.app_version,
        build_identity,
        source_identity: source.identity,
        source_commit: source.commit,
        source_state: source.state,
        dependency_identity: format!(
            "{}@{}{}",
            inputs.repository_identity,
            inputs.revision,
            vendor
                .as_ref()
                .map(|value| value.dependency_suffix())
                .unwrap_or_default()
        ),
        moq_revision: inputs.revision,
        target,
    };
    let mut text = provenance.text();
    if let Some(vendor) = vendor {
        text.push_str(&vendor.record());
    }
    write(&output_directory.join(GENERATED_TEXT_FILE), text.as_bytes())?;
    write(
        &output_directory.join(GENERATED_RUST_FILE),
        provenance.rust().as_bytes(),
    )?;

    if let Some(path) = env::var_os(PROVENANCE_OUTPUT_ENV) {
        if path.is_empty() {
            return Err(Error::new(format!(
                "{PROVENANCE_OUTPUT_ENV} must not be empty"
            )));
        }
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            manifest_directory.join(path)
        };
        write(&path, text.as_bytes())?;
    }

    Ok(())
}

fn emit_rebuild_directives(
    manifest_directory: &Path,
    manifest_path: &Path,
    lock_path: &Path,
    repository_root: &Path,
) -> Result<(), Error> {
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-changed={}", lock_path.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_directory.join("src").display()
    );
    println!("cargo:rerun-if-env-changed={BUILD_IDENTITY_ENV}");
    println!("cargo:rerun-if-env-changed={PROVENANCE_OUTPUT_ENV}");
    println!("cargo:rerun-if-env-changed={SOURCE_COMMIT_ENV}");

    if let Some(git_directory) = git_directory(repository_root)? {
        for path in [
            git_directory.join("HEAD"),
            git_directory.join("index"),
            git_directory.join("packed-refs"),
        ] {
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        if let Ok(head) = fs::read_to_string(git_directory.join("HEAD"))
            && let Some(reference) = head.strip_prefix("ref: ").map(str::trim)
        {
            let path = git_directory.join(reference);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        for path in git_paths(repository_root)? {
            println!(
                "cargo:rerun-if-changed={}",
                repository_root.join(path).display()
            );
        }
    }
    Ok(())
}

fn git_paths(repository_root: &Path) -> Result<BTreeSet<PathBuf>, Error> {
    let tracked = git_bytes(repository_root, &["ls-files", "-z"])?;
    let untracked = git_bytes(
        repository_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    tracked
        .split(|byte| *byte == 0)
        .chain(untracked.split(|byte| *byte == 0))
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path)
                .map_err(|error| Error::new(format!("git path was not UTF-8: {error}")))?;
            Ok(PathBuf::from(path))
        })
        .collect()
}

fn required_env(name: &str) -> Result<String, Error> {
    env::var(name)
        .map_err(|_| {
            Error::new(format!(
                "Cargo did not provide required environment variable {name}"
            ))
        })
        .and_then(validate_value)
}

fn required_path(name: &str) -> Result<PathBuf, Error> {
    let value = env::var_os(name)
        .ok_or_else(|| Error::new(format!("Cargo did not provide required path {name}")))?;
    if value.is_empty() {
        return Err(Error::new(format!("{name} must not be empty")));
    }
    Ok(PathBuf::from(value))
}

fn validate_value(value: String) -> Result<String, Error> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(Error::new(
            "provenance values must be non-empty single-line text",
        ));
    }
    Ok(value)
}

fn read_utf8(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path)
        .map_err(|error| Error::new(format!("failed to read {}: {error}", path.display())))
}

fn write(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(format!("output path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| Error::new(format!("failed to create {}: {error}", parent.display())))?;
    fs::write(path, contents)
        .map_err(|error| Error::new(format!("failed to write {}: {error}", path.display())))
}

#[derive(Debug, PartialEq, Eq)]
struct DependencyInputs {
    app_version: String,
    repository_identity: String,
    revision: String,
}

impl DependencyInputs {
    fn parse(manifest: &str, lock: &str) -> Result<Self, Error> {
        let manifest = manifest
            .parse::<DocumentMut>()
            .map_err(|error| Error::new(format!("invalid Cargo.toml: {error}")))?;
        let app_version = manifest["package"]["version"]
            .as_str()
            .ok_or_else(|| Error::new("Cargo.toml is missing package.version"))?
            .to_owned();
        let dependencies = manifest_dependencies(&manifest)?;
        let repositories = dependencies
            .iter()
            .map(|dependency| dependency.repository.clone())
            .collect::<BTreeSet<_>>();
        let revisions = dependencies
            .iter()
            .map(|dependency| dependency.revision.clone())
            .collect::<BTreeSet<_>>();
        if repositories.len() != 1 {
            return Err(Error::new(format!(
                "MoQ dependencies must use one git repository, found: {}",
                repositories.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        if revisions.len() != 1 {
            return Err(Error::new(format!(
                "MoQ dependencies must use one revision, found: {}",
                revisions.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        let repository = dependencies[0].repository.clone();
        let repository_identity = github_identity(&repository)?;
        let revision = dependencies[0].revision.clone();
        validate_lock(lock, &repository, &revision)?;
        Ok(Self {
            app_version,
            repository_identity,
            revision,
        })
    }
}

#[derive(Debug)]
struct DependencySource {
    repository: String,
    revision: String,
}

fn dependency_tables(manifest: &DocumentMut) -> Vec<&Table> {
    let mut parents = vec![manifest.as_table()];
    if let Some(targets) = manifest.get("target").and_then(Item::as_table) {
        parents.extend(targets.iter().filter_map(|(_, item)| item.as_table()));
    }
    parents
        .into_iter()
        .flat_map(|parent| {
            ["dependencies", "build-dependencies", "dev-dependencies"]
                .into_iter()
                .filter_map(move |section| parent.get(section).and_then(Item::as_table))
        })
        .collect()
}

fn manifest_dependencies(manifest: &DocumentMut) -> Result<Vec<DependencySource>, Error> {
    let mut dependencies = Vec::new();
    for table in dependency_tables(manifest) {
        for (alias, item) in table {
            let package = dependency_string(item, "package").unwrap_or(alias);
            if package != "hang" && !package.starts_with("moq-") {
                continue;
            }
            if package == "moq-video" && dependency_string(item, "path") == Some("vendor/moq-video")
            {
                // Its recorded baseline and own dependencies are validated by vendor::Vendor.
                continue;
            }
            if dependency_string(item, "git").is_none()
                && dependency_string(item, "path").is_none()
                && (dependency_string(item, "version").is_some() || item.as_str().is_some())
            {
                // Separately published crates such as moq-vaapi have registry versions.
                continue;
            }
            let repository = dependency_string(item, "git").ok_or_else(|| {
                Error::new(format!(
                    "MoQ dependency {package} must use a pinned git source"
                ))
            })?;
            let revision = dependency_string(item, "rev").ok_or_else(|| {
                Error::new(format!("MoQ dependency {package} is missing an exact rev"))
            })?;
            dependencies.push(DependencySource {
                repository: normalize_repository(repository)?,
                revision: validate_revision(revision)?,
            });
        }
    }
    if dependencies.is_empty() {
        return Err(Error::new(
            "Cargo.toml contains no direct MoQ git dependencies",
        ));
    }
    Ok(dependencies)
}

fn dependency_string<'a>(item: &'a Item, key: &str) -> Option<&'a str> {
    item.as_value()
        .and_then(|value| value.as_inline_table())
        .and_then(|table| table.get(key))
        .and_then(|value| value.as_str())
        .or_else(|| item.as_table()?.get(key)?.as_str())
}

fn normalize_repository(repository: &str) -> Result<String, Error> {
    let repository = repository.trim_end_matches('/').trim_end_matches(".git");
    if !repository.starts_with("https://github.com/") || repository.contains(['?', '#', '@']) {
        return Err(Error::new(format!(
            "unsupported MoQ git repository URL: {repository}"
        )));
    }
    Ok(repository.to_owned())
}

fn github_identity(repository: &str) -> Result<String, Error> {
    let path = repository
        .strip_prefix("https://github.com/")
        .ok_or_else(|| Error::new(format!("unsupported GitHub repository: {repository}")))?;
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() != 2 || segments.iter().any(|segment| segment.is_empty()) {
        return Err(Error::new(format!(
            "invalid GitHub repository: {repository}"
        )));
    }
    Ok(path.to_owned())
}

fn validate_revision(revision: &str) -> Result<String, Error> {
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::new(format!(
            "revision must be a full 40-character commit SHA: {revision}"
        )));
    }
    Ok(revision.to_ascii_lowercase())
}

fn validate_lock(lock: &str, repository: &str, revision: &str) -> Result<(), Error> {
    let lock = lock
        .parse::<DocumentMut>()
        .map_err(|error| Error::new(format!("invalid Cargo.lock: {error}")))?;
    let packages = lock["package"]
        .as_array_of_tables()
        .ok_or_else(|| Error::new("Cargo.lock is missing package records"))?;
    let mut matched = 0_usize;
    for package in packages {
        let Some(source) = package.get("source").and_then(Item::as_str) else {
            continue;
        };
        let Some(parsed) = LockSource::parse(source)? else {
            continue;
        };
        if parsed.repository != repository {
            continue;
        }
        matched += 1;
        if parsed.requested_revision != revision || parsed.resolved_revision != revision {
            let name = package
                .get("name")
                .and_then(Item::as_str)
                .unwrap_or("unknown");
            return Err(Error::new(format!(
                "Cargo.lock package {name} resolves {} at {} instead of manifest revision {revision}",
                parsed.requested_revision, parsed.resolved_revision
            )));
        }
    }
    if matched == 0 {
        return Err(Error::new(format!(
            "Cargo.lock contains no packages from {repository}"
        )));
    }
    Ok(())
}

#[derive(Debug)]
struct LockSource {
    repository: String,
    requested_revision: String,
    resolved_revision: String,
}

impl LockSource {
    fn parse(source: &str) -> Result<Option<Self>, Error> {
        let Some(source) = source.strip_prefix("git+") else {
            return Ok(None);
        };
        let (request, resolved_revision) = source.rsplit_once('#').ok_or_else(|| {
            Error::new(format!(
                "git lock source has no resolved revision: {source}"
            ))
        })?;
        let (repository, query) = request.split_once('?').ok_or_else(|| {
            Error::new(format!("git lock source has no revision query: {source}"))
        })?;
        let requested_revision = query
            .split('&')
            .find_map(|part| part.strip_prefix("rev="))
            .ok_or_else(|| Error::new(format!("git lock source has no rev parameter: {source}")))?;
        Ok(Some(Self {
            repository: normalize_repository(repository)?,
            requested_revision: validate_revision(requested_revision)?,
            resolved_revision: validate_revision(resolved_revision)?,
        }))
    }
}

#[derive(Debug)]
struct SourceIdentity {
    identity: String,
    commit: String,
    state: &'static str,
}

impl SourceIdentity {
    fn resolve(repository_root: &Path, explicit: Option<&str>) -> Result<Self, Error> {
        if git_directory(repository_root)?.is_none() {
            return Ok(match explicit {
                Some(commit) => Self {
                    identity: commit.to_owned(),
                    commit: commit.to_owned(),
                    state: "provided",
                },
                None => Self::unknown(),
            });
        }

        let head = git(repository_root, &["rev-parse", "--verify", "HEAD"])?;
        let head = validate_revision(head.trim())?;
        if let Some(explicit) = explicit
            && explicit != head
        {
            return Err(Error::new(format!(
                "{SOURCE_COMMIT_ENV} {explicit} does not match repository HEAD {head}"
            )));
        }
        let status = git(
            repository_root,
            &["status", "--porcelain=v1", "--untracked-files=normal"],
        )?;
        let dirty = !status.trim().is_empty();
        Ok(Self {
            identity: if dirty {
                format!("{head}-dirty")
            } else {
                head.clone()
            },
            commit: head,
            state: if dirty { "dirty" } else { "clean" },
        })
    }

    fn unknown() -> Self {
        Self {
            identity: "unknown".to_owned(),
            commit: "unknown".to_owned(),
            state: "unknown",
        }
    }
}

fn git(repository_root: &Path, arguments: &[&str]) -> Result<String, Error> {
    let output = git_bytes(repository_root, arguments)?;
    String::from_utf8(output)
        .map_err(|error| Error::new(format!("git output was not UTF-8: {error}")))
}

fn git_bytes(repository_root: &Path, arguments: &[&str]) -> Result<Vec<u8>, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(arguments)
        .output()
        .map_err(|error| Error::new(format!("failed to run git: {error}")))?;
    if !output.status.success() {
        return Err(Error::new(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn git_directory(repository_root: &Path) -> Result<Option<PathBuf>, Error> {
    let dot_git = repository_root.join(".git");
    let metadata = match fs::metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error::new(format!(
                "failed to inspect {}: {error}",
                dot_git.display()
            )));
        }
    };
    if metadata.is_dir() {
        return Ok(Some(dot_git));
    }
    if !metadata.is_file() {
        return Err(Error::new(format!(
            "unsupported Git metadata entry: {}",
            dot_git.display()
        )));
    }
    let contents = read_utf8(&dot_git)?;
    let path = contents.trim().strip_prefix("gitdir: ").ok_or_else(|| {
        Error::new(format!(
            "invalid Git directory pointer: {}",
            dot_git.display()
        ))
    })?;
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        repository_root.join(path)
    };
    if !path.is_dir() {
        return Err(Error::new(format!(
            "Git directory pointer does not resolve to a directory: {}",
            path.display()
        )));
    }
    Ok(Some(path))
}

#[derive(Debug)]
struct Provenance {
    app_version: String,
    build_identity: String,
    source_identity: String,
    dependency_identity: String,
    source_commit: String,
    source_state: &'static str,
    moq_revision: String,
    target: String,
}

impl Provenance {
    fn text(&self) -> String {
        format!(
            "app_version={}\nbuild_identity={}\nsource_identity={}\ndependency_identity={}\nsource_commit={}\nsource_state={}\nmoq_revision={}\ntarget={}\n",
            self.app_version,
            self.build_identity,
            self.source_identity,
            self.dependency_identity,
            self.source_commit,
            self.source_state,
            self.moq_revision,
            self.target
        )
    }

    fn rust(&self) -> String {
        format!(
            "#[allow(dead_code)]\n\
             pub(crate) const GENERATED_APP_VERSION: &str = {app_version:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_BUILD_IDENTITY: &str = {build_identity:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_SOURCE_IDENTITY: &str = {source_identity:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_DEPENDENCY_IDENTITY: &str = {dependency_identity:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_SOURCE_COMMIT: &str = {source_commit:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_SOURCE_STATE: &str = {source_state:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_MOQ_REVISION: &str = {moq_revision:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_TARGET: &str = {target:?};\n\
             #[allow(dead_code)]\n\
             pub(crate) const GENERATED_RECORD: &str = include_str!(concat!(env!(\"OUT_DIR\"), \"/{GENERATED_TEXT_FILE}\"));\n",
            app_version = self.app_version,
            build_identity = self.build_identity,
            source_identity = self.source_identity,
            dependency_identity = self.dependency_identity,
            source_commit = self.source_commit,
            source_state = self.source_state,
            moq_revision = self.moq_revision,
            target = self.target,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REVISION: &str = "7458c85814e162dda90e87ad0dd21d600a586e09";
    const OTHER_REVISION: &str = "8458c85814e162dda90e87ad0dd21d600a586e09";

    #[test]
    fn reads_one_revision_from_manifest_and_lock() {
        let inputs = DependencyInputs::parse(&manifest(REVISION, REVISION), &lock(REVISION))
            .expect("matching inputs");
        assert_eq!(inputs.app_version, "0.5.0-dev.1");
        assert_eq!(inputs.repository_identity, "moq-dev/moq");
        assert_eq!(inputs.revision, REVISION);
    }

    #[test]
    fn rejects_mixed_manifest_revisions() {
        let error = DependencyInputs::parse(&manifest(REVISION, OTHER_REVISION), &lock(REVISION))
            .expect_err("mixed revisions must fail");
        assert!(error.to_string().contains("must use one revision"));
    }

    #[test]
    fn rejects_lock_revision_that_disagrees_with_manifest() {
        let error = DependencyInputs::parse(&manifest(REVISION, REVISION), &lock(OTHER_REVISION))
            .expect_err("stale lock must fail");
        assert!(error.to_string().contains("instead of manifest revision"));
    }

    #[test]
    fn source_archive_without_git_is_unknown() {
        let root = env::temp_dir().join(format!(
            "moqcast-source-archive-without-git-{}",
            std::process::id()
        ));
        let source = SourceIdentity::resolve(&root, None).expect("source archive is supported");
        assert_eq!(source.identity, "unknown");
        assert_eq!(source.commit, "unknown");
        assert_eq!(source.state, "unknown");
    }

    #[test]
    fn source_archive_with_explicit_commit_is_provided() {
        let root = env::temp_dir().join(format!(
            "moqcast-source-archive-with-explicit-commit-{}",
            std::process::id()
        ));
        let source = SourceIdentity::resolve(&root, Some(REVISION))
            .expect("explicit source archive revision is supported");
        assert_eq!(source.identity, REVISION);
        assert_eq!(source.commit, REVISION);
        assert_eq!(source.state, "provided");
    }

    #[test]
    fn generated_record_keeps_dirty_state_visible() {
        let provenance = Provenance {
            app_version: "0.5.0-dev.1".to_owned(),
            build_identity: "local".to_owned(),
            source_identity: format!("{REVISION}-dirty"),
            dependency_identity: format!("moq-dev/moq@{REVISION}"),
            source_commit: REVISION.to_owned(),
            source_state: "dirty",
            moq_revision: REVISION.to_owned(),
            target: "x86_64-pc-windows-msvc".to_owned(),
        };
        let record = provenance.text();
        assert!(record.contains(&format!("source_identity={REVISION}-dirty\n")));
        assert!(record.contains("source_state=dirty\n"));
    }

    fn manifest(first: &str, second: &str) -> String {
        format!(
            r#"
[package]
name = "fixture"
version = "0.5.0-dev.1"

[dependencies]
moq-tokio = {{ git = "https://github.com/moq-dev/moq", rev = "{first}" }}

[target.'cfg(target_os = "windows")'.dependencies]
hang = {{ git = "https://github.com/moq-dev/moq.git", rev = "{second}" }}
"#
        )
    }

    fn lock(revision: &str) -> String {
        format!(
            r#"
version = 4

[[package]]
name = "moq-tokio"
version = "0.1.0"
source = "git+https://github.com/moq-dev/moq?rev={revision}#{revision}"
"#
        )
    }
}
