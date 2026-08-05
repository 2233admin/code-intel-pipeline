mod common;
#[path = "../src/content_contract.rs"]
mod content_contract;
#[path = "../src/evidence_outcome.rs"]
mod evidence_outcome;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use evidence_outcome::EvidenceOutcome;

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("code-intel-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(repo: &Path) {
    git(repo, &["init", "--quiet"]);
    git(repo, &["config", "user.name", "Repin Test"]);
    git(repo, &["config", "user.email", "repin@example.invalid"]);
    git(repo, &["config", "core.autocrlf", "false"]);
    // Isolate from the developer's global config: a global commit.gpgsign
    // with no usable key, or a global hooksPath, would fail or run
    // arbitrary code on every commit_all call in this file otherwise.
    // core.hooksPath="" (not a Unix-only /dev/null) disarms hooks the same
    // way hardened_git.rs does for every git invocation this pipeline makes.
    git(repo, &["config", "commit.gpgsign", "false"]);
    git(repo, &["config", "core.hooksPath", ""]);
}

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", message]);
}

fn sha256_of(path: &Path) -> String {
    content_contract::sha256_hex(&fs::read(path).unwrap())
}

fn repin(repo: &Path, extra_args: &[&str]) -> Output {
    let mut command = common::cli();
    command.arg("repin").arg("--repo").arg(repo).arg("--json");
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().unwrap()
}

/// Like `repin`, but without `--json` — for asserting on `print_human`'s
/// output, which the JSON-only `repin` helper never exercises.
fn repin_text(repo: &Path, extra_args: &[&str]) -> Output {
    let mut command = common::cli();
    command.arg("repin").arg("--repo").arg(repo);
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "repin output is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn clean_repo_reports_no_stale_pins() {
    let tree = TempTree::new("repin-clean");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init");

    let output = repin(&tree.0, &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["clean"], true);
    assert_eq!(report["filesChanged"], 0);
}

#[test]
fn report_mode_detects_a_stale_pin_without_writing() {
    let tree = TempTree::new("repin-detect");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");

    let head_digest = sha256_of(&tree.0.join("source.rs"));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{head_digest}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record");

    // Edit the source; record.json's pin is now stale.
    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    let before = fs::read_to_string(tree.0.join("record.json")).unwrap();

    let output = repin(&tree.0, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["clean"], false);
    assert_eq!(report["mode"], "report");
    assert_eq!(report["stalePins"][0]["file"], "record.json");
    assert_eq!(report["stalePins"][0]["sites"][0]["old"], head_digest);

    // Report mode must never touch disk.
    assert_eq!(
        fs::read_to_string(tree.0.join("record.json")).unwrap(),
        before
    );
}

#[test]
fn write_mode_fixes_a_stale_pin_and_a_self_referential_chain() {
    let tree = TempTree::new("repin-write");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    fs::write(tree.0.join("measurements.json"), "{\"placeholder\":true}\n").unwrap();
    commit_all(&tree.0, "init source and measurements");

    let source_head = sha256_of(&tree.0.join("source.rs"));
    let measurements_head = sha256_of(&tree.0.join("measurements.json"));
    // record-a pins BOTH source.rs and the shared measurements ledger;
    // record-b only pins the ledger. Editing both source files in one shot
    // means measurements.json's own digest is only known to have changed
    // once the first pass re-hashes it — propagating that to record-b
    // requires a second pass, not a single sweep.
    fs::write(
        tree.0.join("record-a.json"),
        format!(
            r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}},"measurement":"{measurements_head}"}}"#
        ),
    )
    .unwrap();
    fs::write(
        tree.0.join("record-b.json"),
        format!(r#"{{"measurement":"{measurements_head}"}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin records");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    fs::write(
        tree.0.join("measurements.json"),
        "{\"placeholder\":false}\n",
    )
    .unwrap();

    let output = repin(&tree.0, &["--write"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["mode"], "write");
    assert!(
        report["passes"].as_u64().unwrap() >= 2,
        "expected a multi-pass fixpoint: {report}"
    );

    let record_a: Value =
        serde_json::from_str(&fs::read_to_string(tree.0.join("record-a.json")).unwrap()).unwrap();
    let record_b: Value =
        serde_json::from_str(&fs::read_to_string(tree.0.join("record-b.json")).unwrap()).unwrap();
    let new_source = sha256_of(&tree.0.join("source.rs"));
    let new_measurements = sha256_of(&tree.0.join("measurements.json"));
    assert_eq!(record_a["source"]["sha256"], new_source);
    assert_eq!(record_a["measurement"], new_measurements);
    assert_eq!(record_b["measurement"], new_measurements);

    // A second run must find nothing left to do.
    let verify = json(&repin(&tree.0, &[]));
    assert_eq!(verify["clean"], true, "not clean after write: {verify}");
}

#[test]
fn retirements_directory_is_never_rewritten() {
    let tree = TempTree::new("repin-frozen");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");

    let source_head = sha256_of(&tree.0.join("source.rs"));
    fs::create_dir_all(tree.0.join("orchestration/retirements/e01")).unwrap();
    fs::write(
        tree.0.join("orchestration/retirements/e01/frozen.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "freeze packet");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    let before =
        fs::read_to_string(tree.0.join("orchestration/retirements/e01/frozen.json")).unwrap();

    let output = repin(&tree.0, &["--write"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after =
        fs::read_to_string(tree.0.join("orchestration/retirements/e01/frozen.json")).unwrap();
    assert_eq!(
        before, after,
        "frozen retirement packet must never be rewritten"
    );
}

#[test]
fn deleted_source_is_reported_as_an_orphaned_pin() {
    let tree = TempTree::new("repin-orphan");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");
    let source_head = sha256_of(&tree.0.join("source.rs"));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record");

    fs::remove_file(tree.0.join("source.rs")).unwrap();

    let output = repin(&tree.0, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["orphanedPins"][0]["file"], "record.json");
    assert_eq!(report["orphanedPins"][0]["deletedSourcePath"], "source.rs");
}

#[test]
fn write_mode_fixes_resolvable_pins_but_still_exits_nonzero_on_an_orphan() {
    let tree = TempTree::new("repin-write-orphan");
    init_repo(&tree.0);
    fs::write(tree.0.join("gone.rs"), "fn gone() {}\n").unwrap();
    fs::write(tree.0.join("kept.rs"), "fn kept() {}\n").unwrap();
    commit_all(&tree.0, "init two sources");

    let gone_head = sha256_of(&tree.0.join("gone.rs"));
    let kept_head = sha256_of(&tree.0.join("kept.rs"));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"gone":"{gone_head}","kept":"{kept_head}"}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record");

    fs::remove_file(tree.0.join("gone.rs")).unwrap();
    fs::write(tree.0.join("kept.rs"), "fn kept() { changed(); }\n").unwrap();

    let output = repin(&tree.0, &["--write"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an orphaned pin must fail the run even in --write: stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["orphanedPins"][0]["deletedSourcePath"], "gone.rs");

    // The resolvable half of the same file was still fixed.
    let record: Value =
        serde_json::from_str(&fs::read_to_string(tree.0.join("record.json")).unwrap()).unwrap();
    assert_eq!(record["gone"], gone_head, "orphaned pin must be left as-is");
    assert_eq!(record["kept"], sha256_of(&tree.0.join("kept.rs")));
}

#[test]
fn exclude_flag_protects_a_path_prefix_outside_the_default_list() {
    let tree = TempTree::new("repin-exclude");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");

    let source_head = sha256_of(&tree.0.join("source.rs"));
    fs::create_dir_all(tree.0.join("vendor/frozen")).unwrap();
    fs::write(
        tree.0.join("vendor/frozen/record.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record under vendor/");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    let before = fs::read_to_string(tree.0.join("vendor/frozen/record.json")).unwrap();

    // A backslash-style prefix (as a Windows caller would pass) must
    // normalize the same as a forward-slash one.
    let output = repin(&tree.0, &["--write", "--exclude", "vendor\\frozen"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = fs::read_to_string(tree.0.join("vendor/frozen/record.json")).unwrap();
    assert_eq!(before, after, "--exclude'd path must never be rewritten");

    // Proof the file is genuinely still stale, not just untouched because
    // there was nothing to fix: without --exclude, default-scope repin must
    // find it.
    let verify = json(&repin(&tree.0, &[]));
    assert_eq!(
        verify["clean"], false,
        "the excluded pin must still read as stale once back in scope: {verify}"
    );
}

#[test]
fn duplicate_head_content_is_reported_ambiguous_not_silently_rewritten() {
    let tree = TempTree::new("repin-ambiguous");
    init_repo(&tree.0);
    // Two distinct paths, byte-identical content at HEAD: they share one
    // digest, so nothing pinning that digest can be safely auto-resolved.
    fs::write(tree.0.join("twin-a.rs"), "// stub\n").unwrap();
    fs::write(tree.0.join("twin-b.rs"), "// stub\n").unwrap();
    commit_all(&tree.0, "init identical twins");

    let shared_head = sha256_of(&tree.0.join("twin-a.rs"));
    assert_eq!(shared_head, sha256_of(&tree.0.join("twin-b.rs")));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"shared":"{shared_head}"}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin the shared digest");

    // Only one twin changes; naively keying by digest would still resolve
    // the pin (wrongly) to whichever twin's new content wins the map.
    fs::write(tree.0.join("twin-a.rs"), "// changed\n").unwrap();

    let output = repin(&tree.0, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(
        report["filesChanged"], 0,
        "ambiguous digest must not be rewritten"
    );
    let paths: Vec<String> = report["ambiguousSources"][0]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&"twin-a.rs".to_string()));
    assert!(paths.contains(&"twin-b.rs".to_string()));

    // --write must not touch record.json either.
    let before = fs::read_to_string(tree.0.join("record.json")).unwrap();
    let write_output = repin(&tree.0, &["--write"]);
    assert_eq!(write_output.status.code(), Some(1));
    let after = fs::read_to_string(tree.0.join("record.json")).unwrap();
    assert_eq!(before, after, "ambiguous pin must never be auto-resolved");
}

#[test]
fn oversized_file_is_reported_skipped_not_silently_clean() {
    let tree = TempTree::new("repin-skipped");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");

    let source_head = sha256_of(&tree.0.join("source.rs"));
    // A record padded past the 4 MiB scan cap: repin must say it never
    // looked, not claim the repo is clean.
    let padding = "x".repeat(5 * 1024 * 1024);
    fs::write(
        tree.0.join("record.json"),
        format!(
            r#"{{"padding":"{padding}","source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#
        ),
    )
    .unwrap();
    commit_all(&tree.0, "pin record (oversized)");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();

    let output = repin(&tree.0, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["skipped"][0]["file"], "record.json");
    assert_eq!(report["skipped"][0]["gatesClean"], true);
}

#[test]
fn binary_file_is_skipped_but_does_not_gate_clean() {
    let tree = TempTree::new("repin-binary");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    // 0xFF can never begin a valid UTF-8 sequence, so this is binary under
    // any decoder — no text-editing tool could have produced it by accident.
    fs::write(tree.0.join("blob.bin"), [0xffu8, 0xfe, 0x00, 0x80]).unwrap();
    commit_all(&tree.0, "init source and a binary file");

    // Nothing pins source.rs's digest anywhere, but a stale digest is what
    // makes repin walk every tracked file looking for a reference to it —
    // with nothing stale at all, the scan short-circuits on pass one before
    // ever reaching blob.bin, and it would never be scanned or reported.
    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();

    let output = repin(&tree.0, &[]);
    assert!(
        output.status.success(),
        "a binary file must not gate clean: stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["clean"], true);
    assert_eq!(report["skipped"][0]["file"], "blob.bin");
    assert_eq!(report["skipped"][0]["gatesClean"], false);

    // The human-readable form must mark it as informational too, not just
    // the JSON `gatesClean` field.
    let text_output = repin_text(&tree.0, &[]);
    let stdout = String::from_utf8_lossy(&text_output.stdout);
    assert!(
        stdout.contains("skipped: blob.bin") && stdout.contains(", informational"),
        "stdout: {stdout}"
    );
}

#[test]
fn human_output_reports_ambiguous_skipped_and_summary_counts() {
    let tree = TempTree::new("repin-human");
    init_repo(&tree.0);
    // Two distinct paths, byte-identical content at HEAD: an ambiguous
    // digest, same as duplicate_head_content_is_reported_ambiguous_not_silently_rewritten.
    fs::write(tree.0.join("twin-a.rs"), "// stub\n").unwrap();
    fs::write(tree.0.join("twin-b.rs"), "// stub\n").unwrap();
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init identical twins and a source");

    let shared_head = sha256_of(&tree.0.join("twin-a.rs"));
    let source_head = sha256_of(&tree.0.join("source.rs"));
    // A record padded past the scan cap, pinning both the shared (ambiguous)
    // digest and the source digest: it can never be resynced (too large to
    // scan), so it lands in `skipped` regardless of the ambiguous pin inside.
    let padding = "x".repeat(5 * 1024 * 1024);
    fs::write(
        tree.0.join("record.json"),
        format!(
            r#"{{"padding":"{padding}","shared":"{shared_head}","source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#
        ),
    )
    .unwrap();
    commit_all(&tree.0, "pin the shared digest and an oversized record");

    // twin-a.rs's own digest is ambiguous, so changing it alone would never
    // populate the rewrite set (an ambiguous digest is excluded from it) and
    // the scan would short-circuit before ever reaching record.json. Also
    // changing source.rs — an unambiguous digest — is what forces the walk.
    fs::write(tree.0.join("twin-a.rs"), "// changed\n").unwrap();
    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();

    let output = repin_text(&tree.0, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ambiguous:") && stdout.contains("not resolved, resync by hand"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("skipped: record.json"), "stdout: {stdout}");
    assert!(
        stdout.contains("0 orphaned pin(s), 1 ambiguous digest(s), 1 skipped file(s)"),
        "summary line must surface both counts: {stdout}"
    );
}

// --- Gate G1 (#139 / #138): the honesty bit is computed, not asserted. ---
//
// Before this change, `--exclude`-ing every tracked file out of the scan
// surface (or pointing `--repo` at a tree with none at all) still printed
// `"clean": true, "filesChanged": 0` -- the exact same shape #133 reported
// as a real production bug: a report that is indistinguishable from a
// genuine clean pass, over a surface it never actually looked at.

#[test]
fn empty_scan_surface_is_reported_not_computed_not_clean() {
    let tree = TempTree::new("repin-empty-surface");
    init_repo(&tree.0);
    fs::write(tree.0.join("only.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init the only tracked file");

    // Excluding the only tracked file leaves nothing to scan at all --
    // previously reported as a vacuous "clean".
    let output = repin(&tree.0, &["--exclude", "only.rs"]);
    let report = json(&output);
    assert_eq!(
        report["scanCoverage"]["status"], "not-computed",
        "an empty scan surface must not claim any completeness status: {report}"
    );
    assert_eq!(
        report["clean"], false,
        "a scan that covered nothing must not be reported clean: {report}"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "an uncomputed scan coverage must fail the gate, not exit 0: stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn real_scan_reports_complete_with_its_scope() {
    let tree = TempTree::new("repin-complete-scope");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init");

    let report = json(&repin(&tree.0, &[]));
    assert_eq!(report["clean"], true);
    assert_eq!(report["scanCoverage"]["status"], "complete");
    let scanned: Vec<String> = report["scanCoverage"]["scope"]["scannedPaths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        scanned.contains(&"source.rs".to_string()),
        "a genuine complete scan must name the paths it covered: {report}"
    );
    assert_eq!(
        report["scanCoverage"]["scope"]["artifactTypes"][0],
        "tracked-text-file"
    );
}

#[test]
fn oversized_input_reports_partial_coverage_not_complete() {
    let tree = TempTree::new("repin-partial-scope");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    let source_head = sha256_of(&tree.0.join("source.rs"));
    let padding = "x".repeat(5 * 1024 * 1024);
    fs::write(
        tree.0.join("record.json"),
        format!(
            r#"{{"padding":"{padding}","source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#
        ),
    )
    .unwrap();
    commit_all(&tree.0, "init source and an oversized record");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();

    let report = json(&repin(&tree.0, &[]));
    assert_eq!(report["scanCoverage"]["status"], "partial", "{report}");
    assert_eq!(
        report["scanCoverage"]["partialReason"], "unreadable_input",
        "{report}"
    );
    let unreadable: Vec<String> = report["scanCoverage"]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(unreadable.contains(&"record.json".to_string()), "{report}");
}

/// The acceptance condition for G1, not a nice-to-have: take a report the
/// system honestly produced as `not-computed`, forge only its `status`
/// field to `"complete"` -- exactly what a careless writer, or a bad actor
/// hand-editing a handed-off artifact, would do to make an unchecked
/// surface look checked -- and prove parsing it back is rejected because
/// no `scope` was supplied. If this test failed to fail on the forgery,
/// G1 would not be built, no matter how the type looks.
#[test]
fn forged_complete_claim_without_scope_is_rejected() {
    let tree = TempTree::new("repin-forged-complete");
    init_repo(&tree.0);
    fs::write(tree.0.join("only.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init the only tracked file");

    // A real, honestly produced `not-computed` outcome (same setup as
    // `empty_scan_surface_is_reported_not_computed_not_clean`).
    let report = json(&repin(&tree.0, &["--exclude", "only.rs"]));
    let honest = report["scanCoverage"].clone();
    assert_eq!(honest["status"], "not-computed");

    // The system's own honest value parses back fine.
    assert!(
        EvidenceOutcome::from_json(&honest).is_ok(),
        "the honestly produced outcome must itself parse: {honest}"
    );

    // Forge: flip only `status` to `"complete"`, exactly as a bug (or a
    // careless hand-edit) that only ever touches a boolean/status field
    // would. No `scope` is added -- because the whole point is that
    // nothing on this path actually computed one.
    let forged = json!({ "status": "complete" });
    assert!(
        EvidenceOutcome::from_json(&forged).is_err(),
        "a complete claim forged without a scope must be rejected, not accepted"
    );

    // Also forge from a genuinely partial report the same way, to prove
    // this is not an artifact of the not-computed case specifically.
    let tree2 = TempTree::new("repin-forged-complete-from-partial");
    init_repo(&tree2.0);
    fs::write(tree2.0.join("source.rs"), "fn a() {}\n").unwrap();
    let source_head = sha256_of(&tree2.0.join("source.rs"));
    let padding = "x".repeat(5 * 1024 * 1024);
    fs::write(
        tree2.0.join("record.json"),
        format!(
            r#"{{"padding":"{padding}","source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#
        ),
    )
    .unwrap();
    commit_all(&tree2.0, "init source and an oversized record");
    fs::write(tree2.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();

    let partial_report = json(&repin(&tree2.0, &[]));
    let partial = partial_report["scanCoverage"].clone();
    assert_eq!(partial["status"], "partial");
    assert!(EvidenceOutcome::from_json(&partial).is_ok());

    // Forge by copying the real scope's *shape* but dropping one required
    // field -- the kind of partial refactor that would slip through review
    // if the parser were not strict about it.
    // The minimal, realistic forgery: flip *only* `status`. Leave the
    // Partial's real (well-formed) `scope` and its `partialReason`/`paths`
    // fields completely untouched -- this is the shape #133's own bug took
    // (one field changed, nothing else), and it is the case a naive
    // "does `scope` exist?" check would miss: the scope object is right
    // there, fully populated, just borrowed from a claim that never
    // actually achieved completeness.
    let mut forged_from_partial = partial.clone();
    forged_from_partial["status"] = json!("complete");
    assert!(
        EvidenceOutcome::from_json(&forged_from_partial).is_err(),
        "a `partial` report relabeled `complete` by touching only `status` \
         must be rejected even though its (borrowed) scope object is well-formed: {forged_from_partial}"
    );
}
