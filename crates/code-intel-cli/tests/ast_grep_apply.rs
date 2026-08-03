//! Contract tests for the plan-to-apply chain: `edit.ast-grep-plan` records
//! spans and digests, `edit.ast-grep-apply` executes them (#96 item 2, charter
//! gate G4 in #139).
//!
//! Two tests carry the contract. `changing_one_identifier_across_two_files...`
//! is G4's exit condition — the diff touches the identifier and nothing else.
//! `one_drifted_match_refuses_the_whole_plan...` is the reason the capability
//! is allowed to write at all: a plan applies as a unit or not at all, and a
//! refusal names which match moved.

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

const LIB: &str = "pub fn poll() {\n    let retry_budget = 3; // keep in sync with docs/retry.md\n    let _ = retry_budget;\n}\n";
const WORKER: &str = "pub fn work() {\n    let retry_budget = 5; // mirrors src/lib.rs\n    let _ = retry_budget;\n}\n";

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
            "code-intel-ast-grep-apply-{name}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(root.join("src/lib.rs"), LIB).expect("write lib fixture");
        fs::write(root.join("src/worker.rs"), WORKER).expect("write worker fixture");
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

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).expect("read fixture source")
    }

    fn write(&self, relative: &str, text: &str) {
        fs::write(self.root.join(relative), text).expect("overwrite fixture source");
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

fn ast_grep_available() -> bool {
    Command::new("ast-grep")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn declaration(capability: &str) -> Value {
    let registry: Value =
        serde_json::from_slice(&fs::read(manifest()).expect("read registry")).expect("registry");
    registry["integrations"]
        .as_array()
        .expect("integrations")
        .iter()
        .find(|entry| entry["id"] == capability)
        .unwrap_or_else(|| panic!("{capability} is registered"))["capabilityDeclaration"]
        .clone()
}

fn snapshot_for(repo: &Path, scope: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["snapshot", "identity", "--repo"])
        .arg(repo)
        .args([
            "--working-tree-policy",
            "explicit_overlay",
            "--scope",
            scope,
        ])
        .output()
        .expect("compute snapshot identity");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("snapshot JSON")
}

/// Run `edit.ast-grep-plan` through the envelope and return the published
/// plan, exactly as an agent would: one pattern, one rewrite, no bytes.
fn plan(fixture: &Fixture, rewrite: Option<&str>) -> PathBuf {
    let snapshot = snapshot_for(&fixture.root, "src");
    let declaration = declaration("edit.ast-grep-plan");
    let mut options = json!({
        "repoPath": fixture.root,
        "language": "rust",
        "pattern": "retry_budget",
        "paths": ["src"]
    });
    if let Some(rewrite) = rewrite {
        options["rewrite"] = json!(rewrite);
    }
    let request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"edit.ast-grep-plan",
        "contractVersion":1,
        "implementation":declaration["implementation"],
        "snapshot":snapshot["snapshot"],
        "options":options,
        "inputs":[],
        "effectPolicy":{"allowedEffects":declaration["allowedEffects"]}
    });
    let request_path = fixture.root.join("plan-request.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let staging = fixture.root.join("plan-staging");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "edit.ast-grep-plan", "--request"])
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
    staging.join("structured-edit-plan.json")
}

struct Applied {
    exit_code: i32,
    document: Value,
    stderr: String,
}

fn apply_plan(fixture: &Fixture, plan: &Path) -> Applied {
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["edit", "apply-plan", "--repo-path"])
        .arg(&fixture.root)
        .arg("--plan")
        .arg(plan)
        .arg("--manifest")
        .arg(manifest())
        .arg("--envelope")
        .output()
        .expect("run edit apply-plan");
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

/// G4's exit condition, over a plan the model never had to read: four call
/// sites in two files move, and every byte that is not the identifier —
/// including the trailing comments — is untouched.
#[test]
fn changing_one_identifier_across_two_files_touches_only_that_identifier() {
    if !ast_grep_available() {
        return;
    }
    let fixture = Fixture::create("unit");
    let plan_path = plan(&fixture, Some("retry_budget_ms"));

    let document: Value =
        serde_json::from_slice(&fs::read(&plan_path).expect("plan artifact")).expect("plan JSON");
    assert_eq!(document["apply"]["applicable"], json!(true));
    assert_eq!(document["apply"]["edits"].as_array().unwrap().len(), 4);
    // The plan carries an instruction, not bytes: a span, the digest those
    // bytes had, and the replacement — nothing that resembles file contents.
    let first = &document["apply"]["edits"][0];
    assert_eq!(first["sha256"], json!(sha256_hex(b"retry_budget")));
    assert_eq!(first["text"], json!("retry_budget"));
    assert_eq!(first["replacement"], json!("retry_budget_ms"));
    assert_eq!(first["span"]["startLine"], json!(2));

    let applied = apply_plan(&fixture, &plan_path);
    assert_eq!(
        applied.exit_code, 0,
        "stderr={} stdout={}",
        applied.stderr, applied.document
    );
    assert_eq!(applied.document["applied"], json!(true));
    assert_eq!(applied.document["summary"]["edits"], json!(4));
    assert_eq!(applied.document["summary"]["files"], json!(2));
    assert_eq!(
        applied.document["envelope"]["observedEffects"],
        json!(["repo_read", "local_write", "repo_mutation"])
    );

    assert_eq!(
        fixture.read("src/lib.rs"),
        LIB.replace("retry_budget", "retry_budget_ms")
    );
    assert_eq!(
        fixture.read("src/worker.rs"),
        WORKER.replace("retry_budget", "retry_budget_ms")
    );
    // Said the other way, because "the diff touches only the identifier" is
    // the claim under test: every byte outside the four spans is identical.
    assert!(fixture
        .read("src/lib.rs")
        .contains("let retry_budget_ms = 3; // keep in sync with docs/retry.md"));
    assert!(fixture
        .read("src/worker.rs")
        .contains("let retry_budget_ms = 5; // mirrors src/lib.rs"));
}

/// THE acceptance condition for a plan applying *as a unit*: one matched site
/// moves between plan and apply, and the whole plan is refused — the drifted
/// match is named, and the file that did not drift is byte-identical.
#[test]
fn one_drifted_match_refuses_the_whole_plan_and_names_it() {
    if !ast_grep_available() {
        return;
    }
    let fixture = Fixture::create("drift");
    let plan_path = plan(&fixture, Some("retry_budget_ms"));

    // The world moves on: one of the four planned sites is renamed by hand.
    fixture.write(
        "src/worker.rs",
        &WORKER.replacen("retry_budget", "attempt_budget", 1),
    );
    let worker_before_apply = fixture.read("src/worker.rs");

    let applied = apply_plan(&fixture, &plan_path);
    assert_eq!(
        applied.exit_code, 10,
        "stderr={} stdout={}",
        applied.stderr, applied.document
    );
    assert_eq!(applied.document["applied"], json!(false));
    assert_eq!(applied.document["summary"]["drifted"], json!(1));

    let drifted = applied.document["drifted"]
        .as_array()
        .expect("drifted rows");
    assert_eq!(drifted.len(), 1, "{}", applied.document);
    let row = &drifted[0];
    assert_eq!(row["verdict"], "digest_mismatch");
    assert_eq!(row["file"], "src/worker.rs");
    assert_eq!(row["span"]["startLine"], json!(2));
    assert_eq!(row["expectedSha256"], json!(sha256_hex(b"retry_budget")));
    assert_eq!(row["foundSha256"], json!(sha256_hex(b"attempt_budg")));
    // "What is there now" has to be readable, not just hashed: an agent that
    // only learns "some other digest" cannot recover without another round.
    assert_eq!(row["foundText"], json!("attempt_budg"));
    let refusal = applied.document["refusal"]
        .as_str()
        .expect("refusal diagnostic");
    assert!(refusal.contains("src/worker.rs"), "{refusal}");
    assert!(refusal.contains("nothing was written"), "{refusal}");

    // No partial application: the file that did not drift is untouched, and
    // the envelope's own effect ledger is the machine-checkable half of that
    // claim — no repo_mutation was observed.
    assert_eq!(fixture.read("src/lib.rs"), LIB);
    assert_eq!(fixture.read("src/worker.rs"), worker_before_apply);
    assert_eq!(
        applied.document["envelope"]["observedEffects"],
        json!(["repo_read", "local_write"])
    );
    assert_eq!(applied.document["envelope"]["domainVerdict"], "fail");
}

/// The hole a span digest alone leaves, found by dogfooding this on the
/// repository itself: renaming `retry_budget` to `retry_budget_ms` by hand
/// leaves the *planned* bytes reading `retry_budget`, so a span-only check
/// passes and the plan writes `retry_budget_ms_ms`. The plan was derived from
/// a parse of that line, so a changed line refuses it.
#[test]
fn a_site_whose_line_changed_is_refused_even_though_its_span_bytes_did_not() {
    if !ast_grep_available() {
        return;
    }
    let fixture = Fixture::create("extend");
    let plan_path = plan(&fixture, Some("retry_budget_ms"));

    // The hand edit *extends* the identifier: bytes 9..21 of line 2 still
    // read `retry_budget`, so the span digest alone would still match.
    fixture.write(
        "src/worker.rs",
        &WORKER.replacen("retry_budget", "retry_budget_now", 1),
    );
    let worker_before_apply = fixture.read("src/worker.rs");

    let applied = apply_plan(&fixture, &plan_path);
    assert_eq!(
        applied.exit_code, 10,
        "stderr={} stdout={}",
        applied.stderr, applied.document
    );
    assert_eq!(applied.document["applied"], json!(false));
    let drifted = applied.document["drifted"]
        .as_array()
        .expect("drifted rows");
    assert_eq!(drifted.len(), 1, "{}", applied.document);
    assert_eq!(drifted[0]["verdict"], "line_drift");
    // The span digest genuinely still matches: this is the case a
    // bytes-only guard would have written.
    assert_eq!(
        drifted[0]["foundSha256"],
        json!(sha256_hex(b"retry_budget"))
    );
    assert_eq!(drifted[0]["expectedSha256"], drifted[0]["foundSha256"]);
    assert!(drifted[0]["foundLineText"]
        .as_str()
        .expect("found line text")
        .contains("retry_budget_now"));

    assert_eq!(fixture.read("src/lib.rs"), LIB);
    assert_eq!(fixture.read("src/worker.rs"), worker_before_apply);
}

/// A plan without a rewrite is a search preview. It must be refused as an
/// instruction rather than applied as an empty one.
#[test]
fn a_preview_only_plan_is_refused_with_the_reason_the_plan_recorded() {
    if !ast_grep_available() {
        return;
    }
    let fixture = Fixture::create("preview");
    let plan_path = plan(&fixture, None);
    let document: Value =
        serde_json::from_slice(&fs::read(&plan_path).expect("plan artifact")).expect("plan JSON");
    assert_eq!(document["apply"]["applicable"], json!(false));
    assert_eq!(document["authority"]["repositoryMutation"], json!(false));

    let applied = apply_plan(&fixture, &plan_path);
    assert_eq!(applied.exit_code, 64);
    assert!(applied.stderr.contains("no rewrite"), "{}", applied.stderr);
    assert_eq!(fixture.read("src/lib.rs"), LIB);
}

/// The front door owns no write path of its own. Driving the capability
/// directly reaches the same adapter and the same guards, which is what makes
/// "route through the envelope" checkable rather than asserted in a comment.
#[test]
fn the_capability_envelope_is_the_only_write_path() {
    if !ast_grep_available() {
        return;
    }
    let fixture = Fixture::create("envelope");
    let plan_path = plan(&fixture, Some("retry_budget_ms"));
    let plan_document: Value =
        serde_json::from_slice(&fs::read(&plan_path).expect("plan artifact")).expect("plan JSON");
    let declaration = declaration("edit.ast-grep-apply");
    let snapshot = snapshot_for(&fixture.root, "src");
    let mut request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"edit.ast-grep-apply",
        "contractVersion":1,
        "implementation":declaration["implementation"],
        "snapshot":snapshot["snapshot"],
        "options":{"repoPath":fixture.root,"plan":plan_document},
        "inputs":[],
        "effectPolicy":{"allowedEffects":["repo_read","local_write"]}
    });

    // A request that does not ask for repo_mutation is refused before
    // anything is written, rather than audited after the fact.
    let unarmed_path = fixture.root.join("unarmed-request.json");
    fs::write(&unarmed_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "edit.ast-grep-apply", "--request"])
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
    assert_eq!(
        fixture.read("src/lib.rs"),
        LIB,
        "a refused effect must not write"
    );

    // A plan whose recorded digest does not hash its own recorded text is the
    // one shape that could aim a write at bytes nobody read. It is refused
    // even with the effect allowed.
    request["effectPolicy"]["allowedEffects"] = declaration["allowedEffects"].clone();
    request["options"]["plan"]["apply"]["edits"][0]["text"] = json!("something else");
    let forged_path = fixture.root.join("forged-request.json");
    fs::write(&forged_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "edit.ast-grep-apply", "--request"])
        .arg(&forged_path)
        .arg("--out")
        .arg(fixture.root.join("staging-forged"))
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
            .contains("internally inconsistent"),
        "{result}"
    );
    assert_eq!(fixture.read("src/lib.rs"), LIB);
    assert_eq!(fixture.read("src/worker.rs"), WORKER);
}
