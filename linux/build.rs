//! Embeds validated Linux package provenance in the application binary.

#[path = "build_support/provenance.rs"]
mod provenance;

use std::env;
use std::path::PathBuf;

use provenance::PackageProvenance;

const PROVENANCE_FILE_ENV: &str = "MOQCAST_PROVENANCE_FILE";
const BUILD_IDENTITY_ENV: &str = "MOQCAST_EMBEDDED_BUILD_IDENTITY";
const SOURCE_IDENTITY_ENV: &str = "MOQCAST_EMBEDDED_SOURCE_IDENTITY";
const DEPENDENCY_IDENTITY_ENV: &str = "MOQCAST_EMBEDDED_DEPENDENCY_IDENTITY";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/provenance.rs");
    println!("cargo:rerun-if-env-changed={PROVENANCE_FILE_ENV}");

    let app_version =
        env::var("CARGO_PKG_VERSION").expect("Cargo must provide the package version");
    let build = match env::var_os(PROVENANCE_FILE_ENV) {
        Some(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            let provenance = PackageProvenance::read(&path, &app_version)
                .unwrap_or_else(|error| panic!("invalid Linux package provenance: {error}"));
            EmbeddedBuildInfo::from(provenance)
        }
        Some(_) => panic!("{PROVENANCE_FILE_ENV} must not be empty"),
        None => EmbeddedBuildInfo::local(),
    };

    println!(
        "cargo:rustc-env={BUILD_IDENTITY_ENV}={}",
        build.build_identity
    );
    println!(
        "cargo:rustc-env={SOURCE_IDENTITY_ENV}={}",
        build.source_identity
    );
    println!(
        "cargo:rustc-env={DEPENDENCY_IDENTITY_ENV}={}",
        build.dependency_identity
    );
}

#[derive(Debug, PartialEq, Eq)]
struct EmbeddedBuildInfo {
    build_identity: String,
    source_identity: String,
    dependency_identity: String,
}

impl EmbeddedBuildInfo {
    fn local() -> Self {
        Self {
            build_identity: "local".to_owned(),
            source_identity: "unknown".to_owned(),
            dependency_identity: "unknown".to_owned(),
        }
    }
}

impl From<PackageProvenance> for EmbeddedBuildInfo {
    fn from(provenance: PackageProvenance) -> Self {
        Self {
            build_identity: provenance.build_identity,
            source_identity: provenance.source_identity,
            dependency_identity: provenance.dependency_identity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_builds_use_conservative_identity_defaults() {
        assert_eq!(
            EmbeddedBuildInfo::local(),
            EmbeddedBuildInfo {
                build_identity: "local".to_owned(),
                source_identity: "unknown".to_owned(),
                dependency_identity: "unknown".to_owned(),
            }
        );
    }
}
