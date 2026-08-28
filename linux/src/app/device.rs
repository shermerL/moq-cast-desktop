//! Local display identity for this desktop instance.

const MAX_DEVICE_NAME_CHARS: usize = 64;

pub(super) fn name() -> String {
    normalize(&gethostname::gethostname().to_string_lossy()).unwrap_or_else(|| "Linux".to_owned())
}

fn normalize(value: &str) -> Option<String> {
    let sanitized: String = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DEVICE_NAME_CHARS)
        .collect();
    let sanitized = sanitized.trim();
    (!sanitized.is_empty()).then(|| sanitized.to_owned())
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn normalizes_local_device_names_for_display() {
        assert_eq!(
            normalize("  living-room\nlinux  ").as_deref(),
            Some("living-roomlinux")
        );
        assert_eq!(normalize("\n\t"), None);
        assert_eq!(normalize(&"a".repeat(80)).unwrap().chars().count(), 64);
    }
}
