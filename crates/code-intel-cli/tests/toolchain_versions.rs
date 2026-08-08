//! Binding tests for `orchestration/toolchain-versions.v1.json`.
//!
//! The manifest exists so the consistency self-check reads one format instead
//! of five (TOML, pip, a shell variable inside a workflow, JSON, PowerShell).
//! A second source of truth that can drift from the first is worse than no
//! second source at all, so every entry is asserted against its real
//! declaration sites here.
//!
//! `declaredIn[].pattern` is checked against EVERY match in the file, not the
//! first: `ci.yml` installs ast-grep in two independent jobs, each with its own
//! hardcoded version, and a change that updates one and forgets the other is
//! exactly the drift this file is meant to catch. The pipeline's own entry is
//! deliberately a minimum: Cargo.toml owns the current version, while the
//! fixture is a historical floor rather than a second release-edit site.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root")
}

fn manifest() -> Value {
    let path = repo_root().join("orchestration/toolchain-versions.v1.json");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&text).expect("toolchain-versions.v1.json is valid JSON")
}

fn tools(doc: &Value) -> &Vec<Value> {
    doc["tools"].as_array().expect("tools array")
}

/// Minimal regex support for the shapes the manifest actually uses: a literal
/// prefix, one `([^X]+)` capture, and a literal suffix, optionally anchored
/// with `^`. Inside the negated class, `\s` expands to whitespace and `\\` to a
/// backslash; everything else is literal. Pulling in a regex crate for six
/// patterns would add a dependency to a crate that currently has exactly one.
fn find_all(text: &str, pattern: &str) -> Vec<String> {
    let anchored = pattern.starts_with('^');
    let body = pattern.strip_prefix('^').unwrap_or(pattern);

    let open = body.find("([^").expect("pattern has a capture group");
    let close = body[open..].find("]+)").expect("capture group is closed") + open;
    let prefix = unescape(&body[..open]);
    let terminators = expand_class(&body[open + 3..close]);
    let suffix = unescape(&body[close + 3..]);

    let normalized = text.replace("\\\"", "\"");
    let mut out = Vec::new();
    for line in normalized.lines() {
        let start = match (anchored, line.find(&prefix)) {
            (true, Some(0)) => 0,
            (true, _) => continue,
            (false, Some(index)) => index,
            (false, None) => continue,
        };
        let rest = &line[start + prefix.len()..];
        let end = rest
            .find(|c: char| terminators.contains(&c))
            .unwrap_or(rest.len());
        let captured = &rest[..end];
        if captured.is_empty() {
            continue;
        }
        if !suffix.is_empty() && !rest[end..].starts_with(&suffix) {
            continue;
        }
        out.push(captured.to_string());
    }
    out
}

/// Expands the character class inside `[^...]`. Without this, `\s` reads as the
/// two literal characters `\` and `s`, so `pyyaml==6.0.3 \` captures a trailing
/// space and every comparison fails on invisible whitespace.
fn expand_class(class: &str) -> Vec<char> {
    let mut out = Vec::new();
    let mut chars = class.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.extend([' ', '\t', '\r', '\n']),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn unescape(pattern: &str) -> String {
    pattern.replace("\\$", "$").replace("\\\\", "\\")
}

fn version_at_least(actual: &str, minimum: &str) -> bool {
    let parse = |value: &str| {
        value
            .split(['.', '-'])
            .take(3)
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let Some(actual) = parse(actual) else {
        return false;
    };
    let Some(minimum) = parse(minimum) else {
        return false;
    };
    (0..3)
        .map(|index| {
            (
                actual.get(index).copied().unwrap_or_default(),
                minimum.get(index).copied().unwrap_or_default(),
            )
        })
        .find(|(actual, minimum)| actual != minimum)
        .is_none_or(|(actual, minimum)| actual > minimum)
}

#[test]
fn the_manifest_matches_its_declared_schema_shape() {
    let doc = manifest();
    assert_eq!(doc["schema"], "code-intel-toolchain-versions.v1");

    for tool in tools(&doc) {
        let name = tool["name"].as_str().expect("name");
        for field in ["version", "scope", "rationale"] {
            assert!(
                tool[field].as_str().is_some_and(|s| !s.is_empty()),
                "{name}.{field} must be a non-empty string"
            );
        }
        assert!(
            tool["required"].is_boolean(),
            "{name}.required must be a boolean"
        );
        assert!(
            matches!(tool["scope"].as_str(), Some("build" | "ci" | "runtime")),
            "{name}.scope is not one of build/ci/runtime"
        );
        assert!(
            !tool["declaredIn"]
                .as_array()
                .expect("declaredIn")
                .is_empty(),
            "{name} declares no source location, so nothing binds it"
        );
    }
}

#[test]
fn every_declared_version_matches_its_real_declaration_sites() {
    let doc = manifest();
    let root = repo_root();

    for tool in tools(&doc) {
        let name = tool["name"].as_str().expect("name");
        let expected = tool["version"].as_str().expect("version");

        for declaration in tool["declaredIn"].as_array().expect("declaredIn") {
            let file = declaration["file"].as_str().expect("file");
            let pattern = declaration["pattern"].as_str().expect("pattern");
            let path = root.join(file);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{name}: read {}: {err}", path.display()));

            let found = find_all(&text, pattern);
            assert!(
                !found.is_empty(),
                "{name}: pattern `{pattern}` matched nothing in {file} — the declaration moved and this manifest is now stale"
            );
            for (index, actual) in found.iter().enumerate() {
                if tool["comparison"] == "minimum" {
                    assert!(
                        version_at_least(actual, expected),
                        "{name}: match #{index} in {file} is {actual}, below minimum {expected}"
                    );
                } else {
                    assert_eq!(
                        actual, expected,
                        "{name}: match #{index} in {file} is {actual}, manifest says {expected}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_file_that_declares_a_tool_twice_may_not_disagree_with_itself() {
    // ci.yml installs ast-grep in both the windows job and the cross-platform
    // matrix, each with its own `$version = '...'`. Updating one and forgetting
    // the other is a silent split; this asserts the manifest sees both.
    let doc = manifest();
    let root = repo_root();
    let ast_grep = tools(&doc)
        .iter()
        .find(|t| t["name"] == "ast-grep")
        .expect("ast-grep entry");
    let declaration = &ast_grep["declaredIn"][0];
    let text =
        fs::read_to_string(root.join(declaration["file"].as_str().unwrap())).expect("ci.yml");

    let found = find_all(&text, declaration["pattern"].as_str().unwrap());
    assert!(
        found.len() >= 2,
        "expected ast-grep to be declared in more than one ci.yml job, found {} — if the duplicate was consolidated, simplify this test rather than deleting it",
        found.len()
    );
}

#[test]
fn every_probe_is_executable_as_described() {
    // A probe the self-check cannot run is a tool the self-check silently skips
    // — the failure mode this whole manifest exists to remove.
    let doc = manifest();
    for tool in tools(&doc) {
        let name = tool["name"].as_str().expect("name");
        let probe = &tool["probe"];
        match probe["kind"].as_str() {
            Some("command") => assert!(
                probe["command"].as_str().is_some_and(|s| !s.is_empty()),
                "{name}: command probe needs a command"
            ),
            Some("python-module") => assert!(
                probe["module"].as_str().is_some_and(|s| !s.is_empty()),
                "{name}: python-module probe needs a module"
            ),
            Some("npm-package") => assert!(
                probe["package"].as_str().is_some_and(|s| !s.is_empty()),
                "{name}: npm-package probe needs a package"
            ),
            other => panic!("{name}: unknown probe kind {other:?}"),
        }
    }
}

#[test]
fn the_pipelines_own_version_is_the_crate_version() {
    // If this drifts, `code-intel --version` reports a number the manifest
    // does not know about, and the self-check compares against a stale target.
    let doc = manifest();
    let entry = tools(&doc)
        .iter()
        .find(|t| t["name"] == "code-intel")
        .expect("code-intel entry");
    assert_eq!(
        entry["version"].as_str().expect("version"),
        env!("CARGO_PKG_VERSION"),
        "manifest and Cargo.toml disagree about the pipeline's own version"
    );
}
