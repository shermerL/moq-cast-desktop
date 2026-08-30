//! Build and dependency provenance shown by the macOS application.

const MOQ_DEV_REVISION: &str = "81d39f7bf04c82aae324a9ee4251b7f8aa08fb53";
const MOQ_BASELINE: &str = "moq-dev dev@81d39f7b";
const MOQ_DEPENDENCY_IDENTITY: &str = "moq-dev/moq@81d39f7bf04c82aae324a9ee4251b7f8aa08fb53";
pub(crate) const MINIMUM_MACOS: &str = "14.2";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildInfo {
    pub(crate) version: &'static str,
    pub(crate) build_identity: &'static str,
    pub(crate) source_identity: &'static str,
    pub(crate) dependency_identity: &'static str,
    pub(crate) moq_baseline: &'static str,
    pub(crate) target: String,
}

impl BuildInfo {
    pub(crate) fn current() -> Self {
        debug_assert!(MOQ_BASELINE.ends_with(&MOQ_DEV_REVISION[..8]));
        debug_assert!(MOQ_DEPENDENCY_IDENTITY.ends_with(MOQ_DEV_REVISION));
        Self {
            version: env!("CARGO_PKG_VERSION"),
            build_identity: option_env!("MOQCAST_BUILD_IDENTITY").unwrap_or("local"),
            source_identity: option_env!("MOQCAST_SOURCE_COMMIT").unwrap_or("unknown"),
            dependency_identity: MOQ_DEPENDENCY_IDENTITY,
            moq_baseline: MOQ_BASELINE,
            target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::{Array, DocumentMut, Table};

    const MANIFEST: &str = include_str!("../Cargo.toml");
    const CARGO_CONFIG: &str = include_str!("../.cargo/config.toml");
    const INFO_PLIST: &str = include_str!("../packaging/Info.plist.in");
    const PACKAGE_SCRIPT: &str = include_str!("../scripts/package-app.sh");
    const MOQ_REPOSITORY: &str = "https://github.com/moq-dev/moq";
    const MOQ_DEPENDENCIES: [&str; 5] = ["hang", "moq-audio", "moq-mux", "moq-tokio", "moq-video"];

    #[test]
    fn displayed_provenance_matches_every_locked_moq_dependency() {
        let build = BuildInfo::current();
        assert_eq!(build.moq_baseline, "moq-dev dev@81d39f7b");
        assert!(build.dependency_identity.ends_with(MOQ_DEV_REVISION));

        let manifest = MANIFEST
            .parse::<DocumentMut>()
            .expect("valid macOS Cargo.toml");
        let dependencies = moq_dependencies(
            manifest["target"][r#"cfg(target_os = "macos")"#]["dependencies"]
                .as_table()
                .expect("macOS dependencies table"),
        );
        assert_eq!(
            dependencies
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            MOQ_DEPENDENCIES
        );
        assert!(
            dependencies
                .iter()
                .all(|(_, revision)| *revision == MOQ_DEV_REVISION)
        );

        let foundation = manifest["features"]["foundation"]
            .as_array()
            .expect("foundation feature array");
        assert_foundation_feature(foundation);
        assert!(PACKAGE_SCRIPT.contains(MOQ_DEPENDENCY_IDENTITY));
    }

    #[test]
    fn minimum_macos_matches_build_and_packaging_inputs() {
        assert!(CARGO_CONFIG.contains(&format!("MACOSX_DEPLOYMENT_TARGET = \"{MINIMUM_MACOS}\"")));
        assert!(INFO_PLIST.contains(&format!("<string>{MINIMUM_MACOS}</string>")));
        assert!(PACKAGE_SCRIPT.contains(&format!("minimum_macos={MINIMUM_MACOS}")));
    }

    fn moq_dependencies(table: &Table) -> Vec<(&str, &str)> {
        let mut dependencies = table
            .iter()
            .filter_map(|(name, item)| {
                let dependency = item.as_value()?.as_inline_table()?;
                (dependency.get("git")?.as_str()? == MOQ_REPOSITORY).then(|| {
                    (
                        name,
                        dependency
                            .get("rev")
                            .and_then(toml_edit::Value::as_str)
                            .expect("MoQ dependency has a revision"),
                    )
                })
            })
            .collect::<Vec<_>>();
        dependencies.sort_unstable_by_key(|(name, _)| *name);
        dependencies
    }

    fn assert_foundation_feature(feature: &Array) {
        let actual = feature
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        let expected = MOQ_DEPENDENCIES
            .iter()
            .map(|dependency| format!("dep:{dependency}"))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
