//! Validates the Linux path dependencies without presenting local patches as upstream code.

use super::*;

#[derive(Debug)]
pub(super) struct Vendor {
    revision: String,
    libspa_version: String,
    features: String,
}

impl Vendor {
    pub(super) fn load(
        directory: &Path,
        manifest: &str,
        lock: &str,
        inputs: &DependencyInputs,
    ) -> Result<Option<Self>, Error> {
        let manifest = manifest
            .parse::<DocumentMut>()
            .map_err(|error| Error::new(error.to_string()))?;
        let tables = dependency_tables(&manifest);
        let video = tables
            .iter()
            .flat_map(|table| table.iter())
            .find_map(|(alias, item)| {
                (dependency_string(item, "package").unwrap_or(alias) == "moq-video").then_some(item)
            });
        if video.and_then(|item| dependency_string(item, "path")) != Some("vendor/moq-video") {
            return Ok(None);
        }
        let read = |relative: &str| {
            let path = directory.join(relative);
            println!("cargo:rerun-if-changed={}", path.display());
            read_utf8(&path)
        };
        let metadata = read("vendor/moq-video/VENDORED.md")?;
        let revision = validate_video_metadata(&metadata, inputs)?;
        let video_manifest = read("vendor/moq-video/Cargo.toml")?;
        let video_inputs = DependencyInputs::parse(&video_manifest, lock)?;
        if video_inputs.revision != revision
            || video_inputs.repository_identity != inputs.repository_identity
        {
            return Err(Error::new(
                "vendored moq-video dependencies do not match its recorded baseline",
            ));
        }
        validate_path_lock(lock, "moq-video", &video_inputs.app_version)?;

        if manifest
            .get("patch")
            .and_then(|item| item.get("crates-io"))
            .and_then(|item| item.get("libspa"))
            .and_then(|item| dependency_string(item, "path"))
            != Some("vendor/libspa")
        {
            return Err(Error::new("Linux libspa patch must use vendor/libspa"));
        }
        let libspa = read("vendor/libspa/Cargo.toml")?
            .parse::<DocumentMut>()
            .map_err(|error| Error::new(format!("invalid libspa manifest: {error}")))?;
        let libspa_version = libspa["package"]["version"]
            .as_str()
            .ok_or_else(|| Error::new("libspa manifest has no package.version"))?
            .to_owned();
        let libspa_metadata = read("vendor/libspa/VENDORED.md")?;
        if metadata_value(&libspa_metadata, "source_version")? != libspa_version {
            return Err(Error::new(
                "vendored libspa version does not match VENDORED.md",
            ));
        }
        validate_path_lock(lock, "libspa", &libspa_version)?;
        let mut features = Vec::new();
        for table in tables {
            for (name, item) in table {
                if !name.starts_with("moq-") {
                    continue;
                }
                if let Some(array) = item.get("features").and_then(Item::as_array) {
                    let values = array
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .ok_or_else(|| Error::new("dependency feature must be text"))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    features.push(format!("{name}:{}", values.join(",")));
                }
            }
        }
        features.sort();
        Ok(Some(Self {
            revision,
            libspa_version,
            features: features.join(";"),
        }))
    }

    pub(super) fn dependency_suffix(&self) -> String {
        format!(";vendored/moq-video@{}", self.revision)
    }

    pub(super) fn record(&self) -> String {
        format!(
            "moq_video_source=vendored\nmoq_video_revision={}\nlibspa_source=vendored-{}\ncargo_features={}\n",
            self.revision, self.libspa_version, self.features
        )
    }
}

fn validate_video_metadata(metadata: &str, inputs: &DependencyInputs) -> Result<String, Error> {
    let revision = validate_revision(metadata_value(metadata, "source_revision")?)?;
    let repository = normalize_repository(metadata_value(metadata, "source_repository")?)?;
    if github_identity(&repository)? != inputs.repository_identity || revision != inputs.revision {
        return Err(Error::new(
            "vendored moq-video baseline does not match the application MoQ source",
        ));
    }
    if metadata_value(metadata, "source_path")? != "rs/moq-video" {
        return Err(Error::new(
            "vendored moq-video source_path must be rs/moq-video",
        ));
    }
    Ok(revision)
}

fn metadata_value<'a>(contents: &'a str, key: &str) -> Result<&'a str, Error> {
    let prefix = format!("{key} = `");
    let mut values = contents.lines().filter_map(|line| {
        line.strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix('`'))
    });
    let value = values
        .next()
        .ok_or_else(|| Error::new(format!("vendor metadata missing {key}")))?;
    if value.is_empty() || values.next().is_some() {
        return Err(Error::new(format!(
            "vendor metadata has invalid or duplicate {key}"
        )));
    }
    Ok(value)
}

fn validate_path_lock(lock: &str, name: &str, version: &str) -> Result<(), Error> {
    let lock = lock
        .parse::<DocumentMut>()
        .map_err(|error| Error::new(error.to_string()))?;
    let packages = lock["package"]
        .as_array_of_tables()
        .ok_or_else(|| Error::new("lock has no packages"))?;
    let matches = packages
        .iter()
        .filter(|package| {
            package.get("name").and_then(Item::as_str) == Some(name)
                && package.get("version").and_then(Item::as_str) == Some(version)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0].contains_key("source") {
        return Err(Error::new(format!(
            "Cargo.lock must resolve {name} {version} from the local vendor path"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_linux_inputs_produce_vendor_and_feature_records() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../linux");
        let manifest = read_utf8(&directory.join("Cargo.toml")).unwrap();
        let lock = read_utf8(&directory.join("Cargo.lock")).unwrap();
        let inputs = DependencyInputs::parse(&manifest, &lock).unwrap();
        let vendor = Vendor::load(&directory, &manifest, &lock, &inputs)
            .unwrap()
            .unwrap();
        assert_eq!(vendor.revision, inputs.revision);
        assert!(vendor.record().contains("libspa_source=vendored-"));
        assert!(
            vendor
                .features
                .contains("moq-video:capture,nvidia,openh264,pipewire")
        );
    }

    #[test]
    fn rejects_self_consistent_but_stale_vendor_baseline() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../linux");
        let manifest = read_utf8(&directory.join("Cargo.toml")).unwrap();
        let lock = read_utf8(&directory.join("Cargo.lock")).unwrap();
        let inputs = DependencyInputs::parse(&manifest, &lock).unwrap();
        let metadata = read_utf8(&directory.join("vendor/moq-video/VENDORED.md")).unwrap();
        let stale = metadata.replace(&inputs.revision, &"a".repeat(40));
        let error = validate_video_metadata(&stale, &inputs).unwrap_err();
        assert!(error.to_string().contains("baseline does not match"));
    }

    #[test]
    fn rejects_registry_package_instead_of_vendor() {
        let lock = "[[package]]\nname = \"libspa\"\nversion = \"0.10.1\"\nsource = \"registry+https://example.test\"\n";
        assert!(validate_path_lock(lock, "libspa", "0.10.1").is_err());
        assert!(
            validate_path_lock(
                &lock.replace("source = \"registry+https://example.test\"\n", ""),
                "libspa",
                "0.10.1"
            )
            .is_ok()
        );
    }
    #[test]
    fn rejects_missing_and_duplicate_metadata() {
        assert!(metadata_value("", "source_revision").is_err());
        assert!(
            metadata_value(
                "source_revision = `abc`\nsource_revision = `def`\n",
                "source_revision"
            )
            .is_err()
        );
    }
}
