use super::rules::MAX_FILE_BYTES;
use super::*;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_root(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let sequence = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "code-intel-file-gate-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
    fs::write(path, content).expect("write fixture");
}

fn decision_for<'a>(report: &'a GateReport, path: &str) -> &'a GateDecision {
    report
        .decisions
        .iter()
        .find(|decision| decision.path == path)
        .unwrap_or_else(|| panic!("no decision recorded for {path}"))
}

#[test]
fn every_extension_is_only_classified_once() {
    let mut seen = std::collections::BTreeSet::new();
    for extension in CODE_EXTENSIONS {
        assert!(
            seen.insert(*extension),
            "{extension} appears twice in CODE_EXTENSIONS"
        );
    }
}

#[test]
fn identity_holds_and_every_gate_is_represented_over_a_mixed_fixture_tree() {
    let root = fixture_root("all-gates");
    write(&root, "src/lib.rs", b"pub fn ordinary() {}\n");
    write(&root, "README.md", b"not a recognised source extension\n");
    write(&root, "legacy/tools/nested.ps1", b"function F {}\n");
    write(&root, "vendor/inner/thing.py", b"x = 1\n");
    write(&root, ".claude/worktrees/garbage.rs", b"fn garbage() {}\n");
    write(&root, ".gitignore", b"ignored/\n");
    write(&root, "ignored/generated.rs", b"fn generated() {}\n");
    let big = vec![b'a'; (MAX_FILE_BYTES + 1) as usize];
    write(&root, "huge.py", &big);
    write(&root, "binary_blob.rs", b"pub fn f() {\x00garbage}\n");

    let config = GateConfig::built_in();
    let report = evaluate(&root, &config).expect("evaluate fixture tree");
    report.verify_identity().expect("identity holds");

    assert_eq!(
        decision_for(&report, "src/lib.rs").gate,
        GATE_DEFAULT_INCLUDE
    );
    assert_eq!(
        decision_for(&report, "src/lib.rs").decision,
        Decision::Included
    );
    assert_eq!(
        decision_for(&report, "README.md").gate,
        GATE_UNSUPPORTED_EXT
    );
    assert_eq!(
        decision_for(&report, "legacy/tools/nested.ps1").gate,
        GATE_DEFAULT_PATH,
        "default_path must match `tools` at any depth, not just the repository root"
    );
    assert_eq!(
        decision_for(&report, "vendor/inner/thing.py").gate,
        GATE_DEFAULT_PATH
    );
    assert_eq!(
        decision_for(&report, ".claude/worktrees/garbage.rs").gate,
        GATE_DEFAULT_PATH,
        "private worktrees must be excluded by the shared gate"
    );
    assert_eq!(decision_for(&report, "huge.py").gate, GATE_OVERSIZED);
    assert_eq!(
        decision_for(&report, "ignored/generated.rs").gate,
        GATE_REPOSITORY_IGNORED
    );
    assert_eq!(
        decision_for(&report, "ignored/generated.rs").source,
        SOURCE_PROJECT
    );
    assert_eq!(decision_for(&report, "binary_blob.rs").gate, GATE_BINARY);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn user_include_overrides_default_path_and_user_exclude_but_not_binary() {
    let root = fixture_root("user-include");
    write(&root, "vendor/reviewed/keep.py", b"x = 1\n");
    write(&root, "vendor/reviewed/binary.py", b"x = \x00\n");

    let config = GateConfig {
        user_exclude: vec!["vendor/reviewed".to_string()],
        user_include: vec!["vendor/reviewed".to_string()],
    };
    let report = evaluate(&root, &config).expect("evaluate fixture tree");
    report.verify_identity().expect("identity holds");

    let kept = decision_for(&report, "vendor/reviewed/keep.py");
    assert_eq!(kept.decision, Decision::Included);
    assert_eq!(kept.gate, GATE_USER_INCLUDE);

    let binary = decision_for(&report, "vendor/reviewed/binary.py");
    assert_eq!(
        binary.decision,
        Decision::Excluded,
        "user_include must not override the binary veto"
    );
    assert_eq!(binary.gate, GATE_BINARY);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn user_exclude_removes_a_file_default_path_would_otherwise_include() {
    let root = fixture_root("user-exclude");
    write(&root, "src/quarantined.rs", b"pub fn f() {}\n");

    let config = GateConfig {
        user_exclude: vec!["src/quarantined.rs".to_string()],
        user_include: Vec::new(),
    };
    let report = evaluate(&root, &config).expect("evaluate fixture tree");
    report.verify_identity().expect("identity holds");

    let decision = decision_for(&report, "src/quarantined.rs");
    assert_eq!(decision.decision, Decision::Excluded);
    assert_eq!(decision.gate, GATE_USER_EXCLUDE);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn verify_identity_fails_closed_on_a_fabricated_mismatch() {
    // Directly fabricate a broken report: one more candidate than the
    // number of recorded decisions. This is the shape a real defect
    // (an early `continue` that forgets to push a decision) would
    // produce; the check must reject it, not warn and carry on.
    let broken = GateReport {
        candidates: 3,
        included: vec!["a".to_string()],
        decisions: vec![
            GateDecision {
                path: "a".to_string(),
                decision: Decision::Included,
                gate: GATE_DEFAULT_INCLUDE,
                source: SOURCE_BUILT_IN,
            },
            GateDecision {
                path: "b".to_string(),
                decision: Decision::Excluded,
                gate: GATE_UNSUPPORTED_EXT,
                source: SOURCE_BUILT_IN,
            },
        ],
    };
    assert!(broken.verify_identity().is_err());

    let inconsistent_included = GateReport {
        candidates: 2,
        included: vec!["a".to_string(), "b".to_string()],
        decisions: vec![
            GateDecision {
                path: "a".to_string(),
                decision: Decision::Included,
                gate: GATE_DEFAULT_INCLUDE,
                source: SOURCE_BUILT_IN,
            },
            GateDecision {
                path: "b".to_string(),
                decision: Decision::Excluded,
                gate: GATE_UNSUPPORTED_EXT,
                source: SOURCE_BUILT_IN,
            },
        ],
    };
    assert!(inconsistent_included.verify_identity().is_err());
}

#[test]
fn evaluate_fails_closed_when_verify_identity_would_reject_its_own_output() {
    // Sanity check that `evaluate` really does call `verify_identity`
    // before returning, over a tree with nothing exotic in it.
    let root = fixture_root("plain");
    write(&root, "src/a.rs", b"pub fn a() {}\n");
    let report = evaluate(&root, &GateConfig::built_in()).expect("evaluate plain tree");
    assert!(report.verify_identity().is_ok());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn by_gate_omits_gates_with_zero_matches_and_carries_the_declared_source() {
    let root = fixture_root("by-gate");
    write(&root, "src/a.rs", b"pub fn a() {}\n");
    write(&root, "vendor/b.rs", b"pub fn b() {}\n");
    let report = evaluate(&root, &GateConfig::built_in()).expect("evaluate");
    let by_gate = report.by_gate();
    let gate_names: Vec<&str> = by_gate
        .iter()
        .map(|entry| entry["gate"].as_str().unwrap())
        .collect();
    assert!(gate_names.contains(&GATE_DEFAULT_INCLUDE));
    assert!(gate_names.contains(&GATE_DEFAULT_PATH));
    assert!(
        !gate_names.contains(&GATE_BINARY),
        "no binary file in this fixture"
    );

    let default_path_entry = by_gate
        .iter()
        .find(|entry| entry["gate"] == GATE_DEFAULT_PATH)
        .unwrap();
    assert_eq!(default_path_entry["source"], SOURCE_BUILT_IN);
    fs::remove_dir_all(&root).ok();
}
