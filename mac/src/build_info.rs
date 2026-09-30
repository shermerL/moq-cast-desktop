//! Build provenance generated from the manifest, lockfile, and source checkout.

include!(concat!(env!("OUT_DIR"), "/moqcast-build-info.rs"));

#[cfg(feature = "app")]
pub(crate) fn diagnostics() -> moqcast_diagnostics::BuildInfo {
    assert_eq!(GENERATED_APP_VERSION, env!("CARGO_PKG_VERSION"));
    moqcast_diagnostics::BuildInfo::new(GENERATED_APP_VERSION)
        .with_build_identity(GENERATED_BUILD_IDENTITY)
        .with_source_identity(GENERATED_SOURCE_IDENTITY)
        .with_dependency_identity(GENERATED_DEPENDENCY_IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::{Array, DocumentMut};

    const CARGO_CONFIG: &str = include_str!("../.cargo/config.toml");
    const INFO_PLIST: &str = include_str!("../packaging/Info.plist.in");
    const PACKAGE_SCRIPT: &str = include_str!("../scripts/package-app.sh");
    const MINIMUM_MACOS: &str = "14.2";

    #[test]
    fn generated_record_matches_the_embedded_build_fields() {
        assert!(GENERATED_RECORD.contains(&format!("app_version={GENERATED_APP_VERSION}\n")));
        assert!(GENERATED_RECORD.contains(&format!("build_identity={GENERATED_BUILD_IDENTITY}\n")));
        assert!(
            GENERATED_RECORD.contains(&format!("source_identity={GENERATED_SOURCE_IDENTITY}\n"))
        );
        assert!(GENERATED_RECORD.contains(&format!(
            "dependency_identity={GENERATED_DEPENDENCY_IDENTITY}\n"
        )));
        assert!(GENERATED_RECORD.contains(&format!("source_commit={GENERATED_SOURCE_COMMIT}\n")));
        assert!(GENERATED_RECORD.contains(&format!("source_state={GENERATED_SOURCE_STATE}\n")));
        assert!(GENERATED_RECORD.contains(&format!("moq_revision={GENERATED_MOQ_REVISION}\n")));
        assert!(GENERATED_RECORD.contains(&format!("target={GENERATED_TARGET}\n")));
        assert_eq!(
            GENERATED_DEPENDENCY_IDENTITY,
            format!("moq-dev/moq@{GENERATED_MOQ_REVISION}")
        );
    }

    #[test]
    fn feature_contract_is_unchanged() {
        let manifest = include_str!("../Cargo.toml")
            .parse::<DocumentMut>()
            .expect("valid macOS Cargo.toml");
        assert_feature(
            manifest["features"]["foundation"]
                .as_array()
                .expect("foundation feature array"),
            &["publish"],
        );
        assert_feature(
            manifest["features"]["publish"]
                .as_array()
                .expect("publish feature array"),
            &[
                "watch",
                "dep:objc2",
                "dep:objc2-core-graphics",
                "dep:objc2-foundation",
                "dep:objc2-screen-capture-kit",
                "moq-audio/capture",
                "moq-video/capture",
            ],
        );
        assert_feature(
            manifest["features"]["watch"]
                .as_array()
                .expect("watch feature array"),
            &[
                "network",
                "dep:hang",
                "dep:moq-audio",
                "dep:moq-mux",
                "dep:moq-video",
                "moq-audio/playback",
            ],
        );
        assert_feature(
            manifest["features"]["network"]
                .as_array()
                .expect("network feature array"),
            &["dep:moq-tokio", "dep:url"],
        );
    }

    #[test]
    fn packaging_uses_generated_provenance_and_minimum_macos() {
        assert!(CARGO_CONFIG.contains(&format!("MACOSX_DEPLOYMENT_TARGET = \"{MINIMUM_MACOS}\"")));
        assert!(INFO_PLIST.contains(&format!("<string>{MINIMUM_MACOS}</string>")));
        assert!(PACKAGE_SCRIPT.contains("<(grep -v '^target=' \"$arm64_provenance\")"));
        assert!(PACKAGE_SCRIPT.contains("<(grep -v '^target=' \"$x86_64_provenance\")"));
        assert!(PACKAGE_SCRIPT.contains("target=universal2-apple-darwin"));
        assert!(PACKAGE_SCRIPT.contains(&format!("minimum_macos={MINIMUM_MACOS}")));
        assert!(!PACKAGE_SCRIPT.contains("moq-dev/moq@"));
    }

    #[test]
    fn bundle_icon_is_wired_into_packaging_inputs() {
        assert!(INFO_PLIST.contains("<key>CFBundleIconFile</key>"));
        assert!(INFO_PLIST.contains("<string>MoQCast.icns</string>"));
        assert!(PACKAGE_SCRIPT.contains("assets/icons/MoQCast.icns"));
        assert!(PACKAGE_SCRIPT.contains("Contents/Resources/MoQCast.icns"));
    }

    fn assert_feature(feature: &Array, expected: &[&str]) {
        let actual = feature
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
