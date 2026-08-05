//! Contract tests for the span-addressed write primitive: `edit.span-apply`
//! and its `code-intel edit apply` front door (#96 item 1, gate G4 in #139).
//!
//! The load-bearing test here is `a_drifted_span_is_refused_with_evidence`.
//! Everything else describes the primitive; that one is the reason it is
//! allowed to write at all.
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "support/sha256.rs"]
mod sha256_support;
use sha256_support::sha256_hex;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

const FIXTURE: &str = "pub fn poll() {\n    let retry_budget = 3; // keep in sync with docs/retry.md\n    let _ = retry_budget;\n}\n";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn create(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "code-intel-span-apply-{name}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(root.join("src/lib.rs"), FIXTURE).expect("write fixture source");
        for arguments in [
            vec!["init", "-q", "."],
            vec!["config", "user.email", "fixture@example.invalid"],
            vec!["config", "user.name", "fixture"],
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", "fixture"],
        ] {
            let output = Command::new("git")
                .args(&arguments)
                .current_dir(&root)
                .output()
                .expect("run git fixture command");
            assert!(
                output.status.success(),
                "git {arguments:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self { root }
    }

    fn source(&self) -> Vec<u8> {
        fs::read(self.root.join("src/lib.rs")).expect("read fixture source")
    }

    fn overwrite(&self, bytes: &[u8]) {
        fs::write(self.root.join("src/lib.rs"), bytes).expect("overwrite fixture source");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Git packs some objects read-only on Windows; a failed cleanup must
        // never be the reason a contract test reports red.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/integrations.json")
}

/// 1-based line, 1-based start column, exclusive end column, and the sha256 of
/// exactly those bytes — the shape a caller would read out of the index.
fn span_of(bytes: &[u8], needle: &str) -> (usize, usize, usize, String) {
    let offset = find(bytes, needle.as_bytes()).expect("fixture contains the needle");
    let line_start = bytes[..offset]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let line = bytes[..line_start].iter().filter(|b| **b == b'\n').count() + 1;
    let column = offset - line_start + 1;
    (
        line,
        column,
        column + needle.len(),
        sha256_hex(&bytes[offset..offset + needle.len()]),
    )
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

struct Applied {
    exit_code: i32,
    document: Value,
    stderr: String,
}

fn apply(repo: &Path, file: &str, spans: &[(usize, usize, usize, &str, &str)]) -> Applied {
    apply_with(repo, file, spans, &["--envelope"])
}

fn apply_with(
    repo: &Path,
    file: &str,
    spans: &[(usize, usize, usize, &str, &str)],
    extra: &[&str],
) -> Applied {
    let mut command = common::cli();
    command
        .args(["edit", "apply", "--repo-path"])
        .arg(repo)
        .args(["--file", file, "--manifest"])
        .arg(manifest())
        .args(extra);
    for (line, start, end, digest, replacement) in spans {
        command.args([
            "--span",
            &format!("{line}:{start}-{line}:{end}"),
            "--expect-sha256",
            digest,
            "--replacement",
            replacement,
        ]);
    }
    let output = command.output().expect("run edit apply");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Applied {
        exit_code: output.status.code().expect("exit code"),
        document: if stdout.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{error}: {stdout}"))
        },
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// THE acceptance condition (#96, G4): a digest recorded before the file moved
/// under the caller must refuse the write and hand back what it expected
/// against what it found — and must leave the bytes alone.
#[test]
fn a_drifted_span_is_refused_with_evidence() {
    let fixture = Fixture::create("drift");
    let original = fixture.source();
    let (line, start, end, stale_digest) = span_of(&original, "retry_budget");

    // The world moves on after the digest was recorded.
    let drifted = String::from_utf8(original.clone())
        .unwrap()
        .replacen("retry_budget", "attempt_budget", 1)
        .into_bytes();
    fixture.overwrite(&drifted);

    let applied = apply(
        &fixture.root,
        "src/lib.rs",
        &[(line, start, end, &stale_digest, "wrecked")],
    );

    assert_eq!(
        applied.exit_code, 10,
        "stderr={} stdout={}",
        applied.stderr, applied.document
    );
    assert_eq!(applied.document["applied"], json!(false));

    let span = &applied.document["spanEdit"]["spans"][0];
    assert_eq!(span["verdict"], "digest_mismatch");
    assert_eq!(span["expectedSha256"], json!(stale_digest));
    assert_ne!(span["foundSha256"], json!(stale_digest));
    assert_eq!(span["foundSha256"], json!(sha256_hex(b"attempt_budg")));
    // "What was found" has to be readable, not just hashed: an agent that
    // only learns "some other digest" cannot recover without another round.
    assert_eq!(span["foundText"], json!("attempt_budg"));
    let refusal = applied.document["spanEdit"]["refusal"]
        .as_str()
        .expect("refusal diagnostic");
    assert!(refusal.contains(&stale_digest), "{refusal}");
    assert!(refusal.contains("attempt_budg"), "{refusal}");

    // The envelope's own effect ledger is the machine-checkable half of the
    // claim: no repo_mutation was observed.
    assert_eq!(
        applied.document["envelope"]["observedEffects"],
        json!(["repo_read", "local_write"])
    );
    assert_eq!(applied.document["envelope"]["domainVerdict"], "fail");
    assert_eq!(
        applied.document["spanEdit"]["file"]["sha256After"],
        Value::Null
    );
    assert_eq!(fixture.source(), drifted, "a refusal must not write");
}

/// G4's exit condition: change one identifier, and the diff touches only that
/// identifier — the trailing comment on the same line is byte-identical.
#[test]
fn changing_one_identifier_leaves_the_rest_of_the_line_byte_identical() {
    let fixture = Fixture::create("g4");
    let before = fixture.source();
    let (line, start, end, digest) = span_of(&before, "retry_budget");

    let applied = apply(
        &fixture.root,
        "src/lib.rs",
        &[(line, start, end, &digest, "retry_budget_ms")],
    );
    assert_eq!(applied.exit_code, 0, "stderr={}", applied.stderr);
    assert_eq!(applied.document["applied"], json!(true));
    assert_eq!(
        applied.document["envelope"]["observedEffects"],
        json!(["repo_read", "local_write", "repo_mutation"])
    );

    let after = fixture.source();
    let offset = find(&before, b"retry_budget").unwrap();
    assert_eq!(before[..offset], after[..offset], "prefix must not move");
    assert_eq!(
        &before[offset + "retry_budget".len()..],
        &after[offset + "retry_budget_ms".len()..],
        "everything after the identifier, trailing comment included, must be byte-identical"
    );
    assert!(String::from_utf8(after)
        .unwrap()
        .contains("let retry_budget_ms = 3; // keep in sync with docs/retry.md"));
}

/// Two spans on one line are the case sequential single-span calls get wrong:
/// the first replacement shifts every later column. One call resolves both
/// against the same pre-edit bytes.
#[test]
fn two_spans_on_one_line_are_applied_against_the_same_pre_edit_bytes() {
    let fixture = Fixture::create("multi");
    let before = fixture.source();
    let (line, start, end, digest) = span_of(&before, "retry_budget");
    let comment_offset = find(&before, b"docs/retry.md").unwrap();
    let line_start = before[..comment_offset]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .unwrap()
        + 1;
    let comment_column = comment_offset - line_start + 1;

    let applied = apply(
        &fixture.root,
        "src/lib.rs",
        &[
            (line, start, end, &digest, "retry_budget_ms"),
            (
                line,
                comment_column,
                comment_column + "docs/retry.md".len(),
                &sha256_hex(b"docs/retry.md"),
                "docs/retry-budget.md",
            ),
        ],
    );

    assert_eq!(applied.exit_code, 0, "stderr={}", applied.stderr);
    let after = String::from_utf8(fixture.source()).unwrap();
    assert!(
        after.contains("let retry_budget_ms = 3; // keep in sync with docs/retry-budget.md"),
        "{after}"
    );
    // The untouched line keeps the old identifier: this primitive replaces
    // spans, it does not rename symbols.
    assert!(after.contains("let _ = retry_budget;"), "{after}");
}

/// A primitive that exists to make a small change cheap must not spend the
/// saving on its own answer. The default reply carries the evidence and
/// nothing else; the capability envelope is opt-in.
#[test]
fn the_default_reply_stays_small_and_still_carries_the_refusal_evidence() {
    let fixture = Fixture::create("terse");
    let before = fixture.source();
    let (line, start, end, digest) = span_of(&before, "retry_budget");

    let applied = apply_with(
        &fixture.root,
        "src/lib.rs",
        &[(line, start, end, &digest, "retry_budget_ms")],
        &[],
    );
    assert_eq!(applied.exit_code, 0, "stderr={}", applied.stderr);
    assert_eq!(applied.document["envelope"], Value::Null);
    assert_eq!(applied.document["applied"], json!(true));
    let rendered = serde_json::to_string(&applied.document).unwrap();
    assert!(
        rendered.len() < 1024,
        "a one-identifier edit answered in {} bytes: {rendered}",
        rendered.len()
    );

    // The same terseness must not cost the refusal its evidence. The span now
    // holds `retry_budget` again — the first twelve bytes of the identifier
    // just written — so a digest of anything else is a genuine mismatch.
    let refused = apply_with(
        &fixture.root,
        "src/lib.rs",
        &[(
            line,
            start,
            end,
            &sha256_hex(b"something else entirely"),
            "wrecked",
        )],
        &[],
    );
    assert_eq!(refused.exit_code, 10);
    assert_eq!(refused.document["envelope"], Value::Null);
    assert_eq!(
        refused.document["spanEdit"]["spans"][0]["foundText"],
        json!("retry_budget")
    );
}

/// Coordinates rot in the other direction too: an index that points past the
/// end of a shrunken file must refuse with the real line count, not panic and
/// not clamp.
#[test]
fn a_span_past_the_end_of_the_file_is_refused_with_the_line_count() {
    let fixture = Fixture::create("bounds");
    let before = fixture.source();
    let (_, start, end, digest) = span_of(&before, "retry_budget");

    let applied = apply(
        &fixture.root,
        "src/lib.rs",
        &[(900, start, end, &digest, "x")],
    );

    assert_eq!(applied.exit_code, 10, "stderr={}", applied.stderr);
    let span = &applied.document["spanEdit"]["spans"][0];
    assert_eq!(span["verdict"], "out_of_bounds");
    assert!(
        span["diagnostic"]
            .as_str()
            .expect("bounds diagnostic")
            .contains("has 4 line(s)"),
        "{span}"
    );
    assert_eq!(fixture.source(), before, "a refusal must not write");
}

/// Every exit the route contract declares must answer with something a caller
/// can parse — the anti-pattern #145 removed from `run execute` one commit
/// earlier, where a real failure exited with zero stdout bytes and a bare
/// sentence on stderr. These four exits are reached before there is a
/// capability envelope to report, which is exactly why they used to print
/// nothing at all.
#[test]
fn every_pre_envelope_exit_answers_with_a_parseable_document() {
    let fixture = Fixture::create("stdout");
    let before = fixture.source();
    let (line, start, end, digest) = span_of(&before, "retry_budget");
    let good_span = format!("{line}:{start}-{line}:{end}");
    let repo = fixture.root.display().to_string();
    let manifest_path = manifest().display().to_string();
    let missing_manifest = fixture.root.join("no-such-manifest.json");
    let missing_manifest = missing_manifest.display().to_string();

    let cases: [(&str, i32, &str, Vec<String>); 5] = [
        (
            "malformed --span",
            64,
            "arguments",
            vec![
                "--repo-path".into(),
                repo.clone(),
                "--file".into(),
                "src/lib.rs".into(),
                "--span".into(),
                "12:21".into(),
                "--expect-sha256".into(),
                digest.clone(),
                "--replacement".into(),
                "x".into(),
            ],
        ),
        (
            "--file escaping the repository",
            64,
            "arguments",
            vec![
                "--repo-path".into(),
                repo.clone(),
                "--file".into(),
                "../outside.txt".into(),
                "--span".into(),
                good_span.clone(),
                "--expect-sha256".into(),
                digest.clone(),
                "--replacement".into(),
                "x".into(),
            ],
        ),
        (
            "--repo-path that does not resolve",
            65,
            "repository",
            vec![
                "--repo-path".into(),
                fixture.root.join("no-such-checkout").display().to_string(),
                "--file".into(),
                "src/lib.rs".into(),
                "--span".into(),
                good_span.clone(),
                "--expect-sha256".into(),
                digest.clone(),
                "--replacement".into(),
                "x".into(),
            ],
        ),
        (
            "--file that is not in the checkout",
            65,
            "target",
            vec![
                "--repo-path".into(),
                repo.clone(),
                "--file".into(),
                "src/absent.rs".into(),
                "--span".into(),
                good_span.clone(),
                "--expect-sha256".into(),
                digest.clone(),
                "--replacement".into(),
                "x".into(),
            ],
        ),
        (
            "unreadable --manifest",
            69,
            "registry",
            vec![
                "--repo-path".into(),
                repo.clone(),
                "--file".into(),
                "src/lib.rs".into(),
                "--manifest".into(),
                missing_manifest.clone(),
                "--span".into(),
                good_span.clone(),
                "--expect-sha256".into(),
                digest.clone(),
                "--replacement".into(),
                "x".into(),
            ],
        ),
    ];

    for (name, expected_exit, stage, arguments) in cases {
        let mut command = common::cli();
        command.args(["edit", "apply"]).args(&arguments);
        // Only the registry case is about a missing manifest; every other
        // case must reach the real one.
        if !arguments.iter().any(|value| value == "--manifest") {
            command.args(["--manifest", &manifest_path]);
        }
        let output = command.output().expect("run edit apply");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "{name}: stdout={stdout} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let document: Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|error| panic!("{name}: {error}: stdout was {stdout:?}"));
        assert_eq!(document["schema"], "code-intel-edit-failure.v1", "{name}");
        assert_eq!(document["capability"], "edit.span-apply", "{name}");
        assert_eq!(document["exitCode"], json!(expected_exit), "{name}");
        assert_eq!(document["applied"], json!(false), "{name}");
        assert_eq!(document["stage"], json!(stage), "{name}");
        assert!(
            !document["diagnostic"]
                .as_str()
                .unwrap_or_default()
                .is_empty(),
            "{name}: {document}"
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).trim().is_empty(),
            "{name}: the human sentence stays on stderr"
        );
    }

    // The traversal used to be answered with raw git porcelain from the
    // snapshot builder; it must name the flag the caller got wrong instead.
    let output = common::cli()
        .args(["edit", "apply", "--repo-path"])
        .arg(&fixture.root)
        .args(["--file", "../outside.txt", "--span", &good_span])
        .args(["--expect-sha256", &digest, "--replacement", "x"])
        .args(["--manifest", &manifest_path])
        .output()
        .expect("run edit apply");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(stderr.contains("--file"), "{stderr}");
    assert!(!stderr.contains("git"), "{stderr}");
    assert_eq!(fixture.source(), before, "an argument error must not write");
}

/// The front door owns no write path of its own. Driving the capability
/// directly through `capability exec` reaches the same adapter and the same
/// guard, which is what makes "route through the envelope" checkable rather
/// than asserted in a comment.
#[test]
fn the_capability_envelope_is_the_only_write_path() {
    let fixture = Fixture::create("envelope");
    let before = fixture.source();
    let (line, start, end, digest) = span_of(&before, "retry_budget");

    let snapshot_output = common::cli()
        .args(["snapshot", "identity", "--repo"])
        .arg(&fixture.root)
        .args([
            "--working-tree-policy",
            "explicit_overlay",
            "--scope",
            "src/lib.rs",
        ])
        .output()
        .expect("compute snapshot identity");
    assert!(snapshot_output.status.success());
    let snapshot: Value = serde_json::from_slice(&snapshot_output.stdout).expect("snapshot JSON");

    let registry: Value =
        serde_json::from_slice(&fs::read(manifest()).expect("read registry")).expect("registry");
    let declaration = registry["integrations"]
        .as_array()
        .expect("integrations")
        .iter()
        .find(|entry| entry["id"] == "edit.span-apply")
        .expect("edit.span-apply is registered")["capabilityDeclaration"]
        .clone();

    let request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"edit.span-apply",
        "contractVersion":1,
        "implementation":declaration["implementation"],
        "snapshot":snapshot["snapshot"],
        "options":{
            "repoPath":fixture.root,
            "path":"src/lib.rs",
            "spans":[{
                "startLine":line,"startColumn":start,"endLine":line,"endColumn":end,
                "expectedSha256":digest,"replacement":"retry_budget_ms"
            }]
        },
        "inputs":[],
        "effectPolicy":{"allowedEffects":declaration["allowedEffects"]}
    });

    let staging = fixture.root.join("staging-allowed");
    let request_path = fixture.root.join("request.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = common::cli()
        .args(["capability", "exec", "edit.span-apply", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&staging)
        .arg("--manifest")
        .arg(manifest())
        .output()
        .expect("run capability exec");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(fixture.source())
        .unwrap()
        .contains("retry_budget_ms = 3; // keep in sync with docs/retry.md"));

    // ...and a request that does not ask for repo_mutation is refused before
    // anything is written, rather than audited after the fact.
    let mutated = fixture.source();
    let (line, start, end, digest) = span_of(&mutated, "retry_budget_ms");
    let mut unarmed = request.clone();
    unarmed["effectPolicy"]["allowedEffects"] = json!(["repo_read", "local_write"]);
    unarmed["options"]["spans"] = json!([{
        "startLine":line,"startColumn":start,"endLine":line,"endColumn":end,
        "expectedSha256":digest,"replacement":"wrecked"
    }]);
    let unarmed_path = fixture.root.join("unarmed-request.json");
    fs::write(&unarmed_path, serde_json::to_vec(&unarmed).unwrap()).unwrap();
    let output = common::cli()
        .args(["capability", "exec", "edit.span-apply", "--request"])
        .arg(&unarmed_path)
        .arg("--out")
        .arg(fixture.root.join("staging-unarmed"))
        .arg("--manifest")
        .arg(manifest())
        .output()
        .expect("run capability exec");
    assert_eq!(output.status.code(), Some(65));
    let result: Value = serde_json::from_slice(&output.stdout).expect("result envelope");
    assert!(
        result["diagnostics"][0]
            .as_str()
            .unwrap_or_default()
            .contains("repo_mutation"),
        "{result}"
    );
    assert_eq!(fixture.source(), mutated, "a refused effect must not write");
}
