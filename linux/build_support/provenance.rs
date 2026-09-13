//! Parses and validates the provenance record shared by Linux packages and diagnostics.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Validated identities embedded in a packaged Linux application.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PackageProvenance {
    pub(crate) build_identity: String,
    pub(crate) source_identity: String,
    pub(crate) dependency_identity: String,
}

impl PackageProvenance {
    /// Read and validate one generated package provenance record.
    pub(crate) fn read(path: &Path, expected_app_version: &str) -> Result<Self, String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Self::parse(&contents, expected_app_version)
    }

    fn parse(contents: &str, expected_app_version: &str) -> Result<Self, String> {
        let mut fields = BTreeMap::new();
        for (index, line) in contents.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {} is not key=value", index + 1))?;
            if key.is_empty() || value.is_empty() {
                return Err(format!("line {} has an empty key or value", index + 1));
            }
            if fields.insert(key.to_owned(), value.to_owned()).is_some() {
                return Err(format!("duplicate provenance key: {key}"));
            }
        }

        let app_version = required(&fields, "app_version")?;
        if app_version != expected_app_version {
            return Err(format!(
                "app_version {app_version} does not match Cargo package version {expected_app_version}"
            ));
        }

        let source_commit = required(&fields, "source_commit")?;
        validate_revision("source_commit", &source_commit)?;
        let source_identity = required(&fields, "source_identity")?;
        if source_identity != source_commit {
            return Err("source_identity does not match source_commit".to_owned());
        }

        let package_variant = required(&fields, "package_variant")?;
        validate_package_variant(&package_variant)?;
        let build_identity = required(&fields, "build_identity")?;
        if build_identity != package_variant {
            return Err("build_identity does not match package_variant".to_owned());
        }

        let moq_revision = required(&fields, "moq_revision")?;
        validate_revision("moq_revision", &moq_revision)?;
        let moq_video_revision = required(&fields, "moq_video_revision")?;
        validate_revision("moq_video_revision", &moq_video_revision)?;
        let dependency_identity = required(&fields, "dependency_identity")?;
        let expected_dependency_identity =
            format!("moq-dev/moq@{moq_revision};vendored/moq-video@{moq_video_revision}");
        if dependency_identity != expected_dependency_identity {
            return Err(
                "dependency_identity does not match MoQ and vendored moq-video revisions"
                    .to_owned(),
            );
        }

        Ok(Self {
            build_identity,
            source_identity,
            dependency_identity,
        })
    }
}

fn required(fields: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    fields
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing provenance key: {key}"))
}

fn validate_revision(key: &str, value: &str) -> Result<(), String> {
    if (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!(
            "{key} is not a 7-64 character hexadecimal revision"
        ))
    }
}

fn validate_package_variant(value: &str) -> Result<(), String> {
    let mut bytes = value.bytes();
    let valid_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    let valid_rest = bytes.all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    });
    if valid_first && valid_rest {
        Ok(())
    } else {
        Err("package_variant contains unsupported characters".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_VERSION: &str = "0.5.0-dev.1";
    const MOQ_REVISION: &str = "81d39f7bf04c82aae324a9ee4251b7f8aa08fb53";
    const SOURCE_COMMIT: &str = "0123456789ab";

    fn build_info() -> String {
        format!(
            "app_version={APP_VERSION}\n\
             build_identity=linux-x86_64-ubuntu24.04-glibc2.39\n\
             source_identity={SOURCE_COMMIT}\n\
             dependency_identity=moq-dev/moq@{MOQ_REVISION};vendored/moq-video@{MOQ_REVISION}\n\
             source_commit={SOURCE_COMMIT}\n\
             moq_revision={MOQ_REVISION}\n\
             moq_video_revision={MOQ_REVISION}\n\
             package_variant=linux-x86_64-ubuntu24.04-glibc2.39\n\
             build_distribution=ubuntu-24.04\n"
        )
    }

    #[test]
    fn parses_the_identity_fields_from_one_package_record() {
        let provenance = PackageProvenance::parse(&build_info(), APP_VERSION).unwrap();

        assert_eq!(
            provenance,
            PackageProvenance {
                build_identity: "linux-x86_64-ubuntu24.04-glibc2.39".to_owned(),
                source_identity: SOURCE_COMMIT.to_owned(),
                dependency_identity: format!(
                    "moq-dev/moq@{MOQ_REVISION};vendored/moq-video@{MOQ_REVISION}"
                ),
            }
        );
    }

    #[test]
    fn rejects_a_package_record_for_another_app_version() {
        let error = PackageProvenance::parse(&build_info(), "0.5.0-dev.2").unwrap_err();

        assert!(error.contains("does not match Cargo package version"));
    }

    #[test]
    fn parses_crlf_package_metadata() {
        let build_info = build_info().replace('\n', "\r\n");

        let provenance = PackageProvenance::parse(&build_info, APP_VERSION).unwrap();

        assert_eq!(provenance.source_identity, SOURCE_COMMIT);
    }

    #[test]
    fn rejects_a_package_record_missing_a_required_identity() {
        let build_info = build_info().replace(&format!("source_commit={SOURCE_COMMIT}\n"), "");

        let error = PackageProvenance::parse(&build_info, APP_VERSION).unwrap_err();

        assert!(error.contains("missing provenance key: source_commit"));
    }

    #[test]
    fn rejects_identity_aliases_that_disagree_with_their_source_fields() {
        let build_info = build_info().replace(
            &format!(
                "dependency_identity=moq-dev/moq@{MOQ_REVISION};vendored/moq-video@{MOQ_REVISION}"
            ),
            "dependency_identity=moq-dev/moq@fffffff;vendored/moq-video@fffffff",
        );

        let error = PackageProvenance::parse(&build_info, APP_VERSION).unwrap_err();

        assert!(error.contains("dependency_identity does not match"));
    }

    #[test]
    fn rejects_source_identity_that_disagrees_with_the_source_commit() {
        let build_info = build_info().replace(
            &format!("source_identity={SOURCE_COMMIT}"),
            "source_identity=abcdef0",
        );

        let error = PackageProvenance::parse(&build_info, APP_VERSION).unwrap_err();

        assert!(error.contains("source_identity does not match"));
    }

    #[test]
    fn rejects_build_identity_that_disagrees_with_the_package_variant() {
        let build_info = build_info().replace(
            "build_identity=linux-x86_64-ubuntu24.04-glibc2.39",
            "build_identity=linux-x86_64-debian12-glibc2.36",
        );

        let error = PackageProvenance::parse(&build_info, APP_VERSION).unwrap_err();

        assert!(error.contains("build_identity does not match"));
    }

    #[test]
    fn rejects_duplicate_provenance_keys() {
        let build_info = format!("{}source_commit=abcdef0\n", build_info());

        let error = PackageProvenance::parse(&build_info, APP_VERSION).unwrap_err();

        assert!(error.contains("duplicate provenance key"));
    }
}
