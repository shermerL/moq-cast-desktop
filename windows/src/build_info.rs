//! Build provenance generated from the manifest, lockfile, and source checkout.

use moqcast_diagnostics::BuildInfo;

include!(concat!(env!("OUT_DIR"), "/moqcast-build-info.rs"));

pub(crate) fn current() -> BuildInfo {
    assert_eq!(GENERATED_APP_VERSION, env!("CARGO_PKG_VERSION"));
    BuildInfo::new(GENERATED_APP_VERSION)
        .with_build_identity(GENERATED_BUILD_IDENTITY)
        .with_source_identity(GENERATED_SOURCE_IDENTITY)
        .with_dependency_identity(GENERATED_DEPENDENCY_IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
