//! Build and dependency provenance shown by the Windows application.

const MOQ_DEV_REVISION: &str = "81d39f7bf04c82aae324a9ee4251b7f8aa08fb53";
const MOQ_BASELINE: &str = "moq-dev dev@81d39f7b";
const MOQ_DEPENDENCY_IDENTITY: &str = "moq-dev/moq@81d39f7bf04c82aae324a9ee4251b7f8aa08fb53";

pub(crate) fn moq_baseline() -> &'static str {
    debug_assert!(MOQ_BASELINE.ends_with(&MOQ_DEV_REVISION[..8]));
    MOQ_BASELINE
}

pub(crate) fn dependency_identity() -> &'static str {
    debug_assert!(MOQ_DEPENDENCY_IDENTITY.ends_with(MOQ_DEV_REVISION));
    MOQ_DEPENDENCY_IDENTITY
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::{DocumentMut, Table};

    const MANIFEST: &str = include_str!("../Cargo.toml");
    const MOQ_REPOSITORY: &str = "https://github.com/moq-dev/moq";
    const MOQ_DEPENDENCIES: [&str; 5] = ["hang", "moq-audio", "moq-mux", "moq-tokio", "moq-video"];

    #[test]
    fn display_and_export_identities_share_the_pinned_revision() {
        assert_eq!(moq_baseline(), "moq-dev dev@81d39f7b");
        assert_eq!(
            dependency_identity(),
            "moq-dev/moq@81d39f7bf04c82aae324a9ee4251b7f8aa08fb53"
        );
        assert!(moq_baseline().ends_with(&MOQ_DEV_REVISION[..8]));
        assert!(dependency_identity().ends_with(MOQ_DEV_REVISION));
    }

    #[test]
    fn every_moq_dependency_uses_the_displayed_revision() {
        let manifest = MANIFEST
            .parse::<DocumentMut>()
            .expect("valid Windows Cargo.toml");
        let mut dependencies = moq_dependencies(
            manifest["dependencies"]
                .as_table()
                .expect("dependencies table"),
        );
        dependencies.extend(moq_dependencies(
            manifest["target"][r#"cfg(target_os = "windows")"#]["dependencies"]
                .as_table()
                .expect("Windows dependencies table"),
        ));
        dependencies.sort_unstable_by_key(|(name, _)| *name);

        assert_eq!(
            dependencies
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            MOQ_DEPENDENCIES,
            "update the expected MoQ dependency set"
        );
        assert!(
            dependencies
                .iter()
                .all(|(_, revision)| *revision == MOQ_DEV_REVISION),
            "all MoQ dependencies must match the displayed baseline"
        );
    }

    fn moq_dependencies(table: &Table) -> Vec<(&str, &str)> {
        table
            .iter()
            .filter_map(|(name, item)| {
                let dependency = item.as_value()?.as_inline_table()?;
                (dependency.get("git")?.as_str()? == MOQ_REPOSITORY).then(|| {
                    (
                        name,
                        dependency
                            .get("rev")
                            .and_then(toml_edit::Value::as_str)
                            .expect("MoQ dependency must have a rev pin"),
                    )
                })
            })
            .collect()
    }
}
