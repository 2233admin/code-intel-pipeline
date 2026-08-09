mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::{json, Value};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-diaphora-{nonce}-{sequence}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    base_binary: PathBuf,
    candidate_binary: PathBuf,
    result_db: PathBuf,
    out: PathBuf,
}

fn fixture(root: &Path) -> Fixture {
    let base_binary = root.join("base.bin");
    let candidate_binary = root.join("candidate.bin");
    let result_db = root.join("comparison.diaphora");
    let out = root.join("observation.json");
    fs::write(&base_binary, b"base binary fixture\n").unwrap();
    fs::write(&candidate_binary, b"candidate binary fixture\n").unwrap();

    let connection = Connection::open(&result_db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE config (main_db TEXT, diff_db TEXT, version TEXT, date TEXT);
             CREATE TABLE results (type TEXT, line INTEGER, address TEXT, name TEXT, address2 TEXT, name2 TEXT, ratio REAL, nodes1 INTEGER, nodes2 INTEGER, description TEXT);
             CREATE TABLE unmatched (type TEXT, line INTEGER, address TEXT, name TEXT);
             INSERT INTO config VALUES ('base.sqlite', 'candidate.sqlite', '2.0', '2026-08-09');
             INSERT INTO results VALUES ('best', 1, '1000', 'base_best', '2000', 'candidate_best', 1.0, 5, 5, 'same bytes');
             INSERT INTO results VALUES ('partial', 2, '1010', 'base_partial', '2010', 'candidate_partial', 0.82, 4, 5, 'partial graph');
             INSERT INTO results VALUES ('unreliable', 3, '1020', 'base_unreliable', '2020', 'candidate_unreliable', 0.61, 3, 4, 'heuristic');
             INSERT INTO results VALUES ('multimatch', 4, '1030', 'base_multi', '2030', 'candidate_multi', 0.50, 2, 2, 'ambiguous');
             INSERT INTO unmatched VALUES ('primary', 1, '1040', 'only_base');
             INSERT INTO unmatched VALUES ('secondary', 2, '2040', 'only_candidate_a');
             INSERT INTO unmatched VALUES ('secondary', 3, '2050', 'only_candidate_b');",
        )
        .unwrap();
    Fixture {
        base_binary,
        candidate_binary,
        result_db,
        out,
    }
}

fn inspect(fixture: &Fixture) -> std::process::Output {
    common::cli()
        .args(["provider", "diaphora-inspect", "--result-db"])
        .arg(&fixture.result_db)
        .arg("--base-binary")
        .arg(&fixture.base_binary)
        .arg("--candidate-binary")
        .arg(&fixture.candidate_binary)
        .args([
            "--source-revision",
            "source-r1",
            "--provider-version",
            "3.1.0",
            "--observed-at",
            "1770000000",
            "--out",
        ])
        .arg(&fixture.out)
        .output()
        .unwrap()
}

fn document(fixture: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(&fixture.out).unwrap()).unwrap()
}

#[test]
fn imports_diaphora_result_database_and_binds_every_input() {
    let temp = Temp::new();
    let fixture = fixture(&temp.0);
    let output = inspect(&fixture);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let observation = document(&fixture);
    assert_eq!(observation["schema"], "code-intel-diaphora-observation.v1");
    assert_eq!(observation["status"], "observed");
    assert_eq!(observation["provider"]["id"], "diaphora");
    assert_eq!(observation["provider"]["resultSchemaVersion"], "2.0");
    assert_eq!(observation["summary"]["resultRows"], 4);
    assert_eq!(
        observation["summary"]["matches"],
        json!({"best":1,"partial":1,"unreliable":1,"multimatch":1})
    );
    assert_eq!(
        observation["summary"]["unmatched"],
        json!({"primary":1,"secondary":2})
    );
    assert_eq!(
        observation["summary"]["topMatches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["best", "partial", "unreliable", "multimatch"]
    );
    for field in [
        "identity",
        "baseBinarySha256",
        "candidateBinarySha256",
        "resultDatabaseSha256",
    ] {
        assert_eq!(observation["comparison"][field].as_str().unwrap().len(), 64);
    }
    assert_eq!(observation["authority"]["observationOnly"], true);
    assert_eq!(observation["authority"]["engineeringFacts"], json!([]));
    assert_eq!(observation["failure"]["kind"], "none");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        observation
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(temp.0.to_string_lossy().as_ref()),
        "paths must not be emitted"
    );
}

#[test]
fn missing_result_database_is_unavailable_not_a_clean_comparison() {
    let temp = Temp::new();
    let fixture = fixture(&temp.0);
    fs::remove_file(&fixture.result_db).unwrap();
    let output = inspect(&fixture);
    assert_eq!(output.status.code(), Some(69));
    let observation = document(&fixture);
    assert_eq!(observation["status"], "unavailable");
    assert_eq!(observation["comparison"], Value::Null);
    assert_eq!(observation["summary"], Value::Null);
    assert_eq!(observation["failure"]["kind"], "provider_unavailable");
}

#[test]
fn rejects_non_diaphora_sqlite_without_leaking_input_paths() {
    let temp = Temp::new();
    let fixture = fixture(&temp.0);
    fs::remove_file(&fixture.result_db).unwrap();
    Connection::open(&fixture.result_db)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (value TEXT);")
        .unwrap();

    let output = inspect(&fixture);
    assert_eq!(output.status.code(), Some(65));
    let observation = document(&fixture);
    assert_eq!(observation["status"], "rejected");
    assert_eq!(observation["comparison"], Value::Null);
    assert_eq!(observation["summary"], Value::Null);
    assert_eq!(observation["failure"]["kind"], "process_failure");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains(temp.0.to_string_lossy().as_ref()));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(temp.0.to_string_lossy().as_ref()));
}

#[test]
fn invalid_cli_is_rejected_before_an_artifact_is_written() {
    let temp = Temp::new();
    let fixture = fixture(&temp.0);
    let output = common::cli()
        .args(["provider", "diaphora-inspect", "--result-db"])
        .arg(&fixture.result_db)
        .arg("--base-binary")
        .arg(&fixture.base_binary)
        .arg("--candidate-binary")
        .arg(&fixture.candidate_binary)
        .args([
            "--source-revision",
            "source-r1",
            "--provider-version",
            "3.1.0",
            "--observed-at",
            "1770000000",
            "--out",
        ])
        .arg(&fixture.out)
        .arg("--unexpected")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(!fixture.out.exists());
}

#[test]
fn schema_and_docs_keep_the_boundary_advisory_and_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema: Value = serde_json::from_slice(
        &fs::read(
            root.join("orchestration/schemas/code-intel-diaphora-observation.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["$defs"]["summary"]["additionalProperties"], false);
    let docs = fs::read_to_string(root.join("docs/diaphora-provider-adapter.md")).unwrap();
    assert!(docs.contains("does not install, embed, invoke"));
    assert!(docs.contains("provider_unavailable"));
    assert!(docs.contains("cannot make a CI or governance gate green"));
}
