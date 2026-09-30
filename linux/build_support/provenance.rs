//! Build identities shared with the package's generated provenance record.

include!(concat!(env!("OUT_DIR"), "/moqcast-build-info.rs"));

pub(crate) fn diagnostics() -> moqcast_diagnostics::BuildInfo {
    moqcast_diagnostics::BuildInfo::new(GENERATED_APP_VERSION)
        .with_build_identity(GENERATED_BUILD_IDENTITY)
        .with_source_identity(GENERATED_SOURCE_IDENTITY)
        .with_dependency_identity(GENERATED_DEPENDENCY_IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_and_package_share_vendor_identity() {
        assert_eq!(GENERATED_APP_VERSION, env!("CARGO_PKG_VERSION"));
        assert!(GENERATED_RECORD.contains(&format!(
            "dependency_identity={GENERATED_DEPENDENCY_IDENTITY}\n"
        )));
        assert!(GENERATED_RECORD.contains("moq_video_source=vendored\n"));
        assert!(GENERATED_DEPENDENCY_IDENTITY.contains(";vendored/moq-video@"));
    }
}
