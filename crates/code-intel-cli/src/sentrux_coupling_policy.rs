//! Shared reading of the `ignore_test_dependencies` coupling policy (#394)
//! and the single test-file rule behind it. The native gate and the what-if
//! `rule_hints` both read the policy here so they cannot disagree.

/// `true` only when `.sentrux/rules.toml` text sets `ignore_test_dependencies`
/// to the TOML boolean `true`; absent or unreadable values mean `false`.
/// Sections are ignored, like every other rule read from this file.
pub(crate) fn ignore_test_dependencies(rules: &str) -> bool {
    rules
        .lines()
        .find_map(|line| {
            let (key, rest) = line.trim().split_once('=')?;
            (key.trim() == "ignore_test_dependencies")
                .then(|| rest.split('#').next().unwrap_or("").trim() == "true")
        })
        .unwrap_or(false)
}

pub(crate) fn is_test_file(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    let leaf = lower.rsplit('/').next().unwrap_or(&lower);
    lower.starts_with("test/")
        || lower.starts_with("tests/")
        || lower.contains("/test/")
        || lower.contains("/tests/")
        || lower.contains("/__tests__/")
        || leaf.starts_with("test_")
        || leaf.contains(".test.")
        || leaf.contains(".spec.")
}
