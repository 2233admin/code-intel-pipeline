//! Issue #500: the architecture_graph stage (the external graph tool, its
//! provider adapter, the internal graph generator and the hospital's
//! missing-graph rung) is gone. These tests pin the observable absence: no
//! provider/route operation, no DAG node, no hospital verdict, and the doctor
//! observation keeps only the `graphProvider` fields the install-smoke gate
//! reads.
//!
//! The removed names are spelled through `concat!` so the repository-wide
//! "no leftover reference" greps of #500 stay meaningful.
mod common;
#[path = "support/schema.rs"]
mod schema_support;
#[path = "support/sha256.rs"]
mod sha256;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const GRAPH_ADAPT: &str = concat!("graph", "-adapt");
const GRAPH_NODE: &str = concat!("evidence", ".graph");
const REQUIRE_GRAPH_FLAG: &str = concat!("--require-", "understand-graph");
const DOCTOR_REQUIRE_FLAG: &str = concat!("--doctor-require-", "understand");
const BOOTSTRAP_REQUIRE_FLAG: &str = concat!("--require-", "understand");
const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-graph-removed-{tag}-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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

fn pipeline_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json_stdout(args: &[&str]) -> Value {
    let output = common::cli()
        .args(args)
        .current_dir(pipeline_root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn is_graph_operation(operation: &Value) -> bool {
    matches!(operation["provider"].as_str(), Some("understand" | "graph"))
        || operation["stage"] == "architecture_graph"
}

#[test]
fn provider_and_route_lists_carry_no_graph_or_understand_operation() {
    for (action, key) in [("provider", "operations"), ("route", "routes")] {
        let listed = json_stdout(&[action, "--action", "List", "--json"]);
        let operations = listed[key]
            .as_array()
            .unwrap_or_else(|| panic!("{action} List has operations: {listed}"));
        assert!(!operations.is_empty(), "{action} List is empty");
        let graph: Vec<&Value> = operations
            .iter()
            .filter(|op| is_graph_operation(op))
            .collect();
        assert!(graph.is_empty(), "{action} List still lists {graph:?}");
    }
    let validated = json_stdout(&["provider", "--action", "Validate", "--json"]);
    assert_eq!(validated["ok"], json!(true), "{validated}");
}

#[test]
fn removed_commands_and_flags_fail() {
    let temp = Temp::new("cli");
    for command in ["graph", "understand"] {
        let output = common::cli()
            .args([command, "--repo"])
            .arg(&temp.0)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{command} must be gone");
    }
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 0, "nothing written");

    let output = common::cli()
        .args(["provider", GRAPH_ADAPT, "--help"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "provider {GRAPH_ADAPT} must be gone"
    );

    for flag in [REQUIRE_GRAPH_FLAG, DOCTOR_REQUIRE_FLAG] {
        let output = common::cli()
            .args(["run", "dag-coordinate", "--repo"])
            .arg(&temp.0)
            .arg("--out")
            .arg(temp.0.join("out"))
            .args([flag, "false"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64), "{flag}");
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains(&format!("unknown DAG run argument: {flag}")));
    }

    let output = common::cli()
        .args(["doctor", "bootstrap", BOOTSTRAP_REQUIRE_FLAG, "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
        "unknown argument for doctor bootstrap: {BOOTSTRAP_REQUIRE_FLAG}"
    )));
}

#[test]
fn dag_plan_has_no_graph_node_in_any_profile() {
    let temp = Temp::new("dag");
    let repo = temp.0.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let runs: [&[&str]; 3] = [
        &["dag-coordinate"],
        &["execute", "--profile", "strict", "--mode", "full"],
        &["execute", "--profile", "default", "--mode", "lite"],
    ];
    for (index, args) in runs.iter().enumerate() {
        let out = temp.0.join(format!("out-{index}"));
        let mut command = common::cli();
        command
            .arg("run")
            .args(*args)
            .arg("--repo")
            .arg(&repo)
            .arg("--out")
            .arg(&out);
        if args[0] == "execute" {
            command
                .arg("--authority-root")
                .arg(temp.0.join(format!("authority-{index}")))
                .args(["--final-name", "graph-removed"]);
            fs::create_dir_all(temp.0.join(format!("authority-{index}"))).unwrap();
        }
        let output = command.output().unwrap();
        let stdout: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        let manifest = if stdout["manifest"].is_object() {
            &stdout["manifest"]
        } else {
            &stdout
        };
        let nodes = manifest["nodes"]
            .as_object()
            .unwrap_or_else(|| panic!("{args:?}: {stdout}"));
        assert!(nodes.contains_key("repo.snapshot"), "{args:?}");
        assert!(!nodes.contains_key(GRAPH_NODE), "{args:?}");
        assert!(!manifest.to_string().contains(GRAPH_ADAPT), "{args:?}");
        assert!(!out.join(format!("{GRAPH_NODE}.request.json")).exists());
    }
}

fn write_ref(root: &Path, path: &str, schema: &str, kind: &str, value: &Value) -> Value {
    let full = root.join(path);
    fs::write(&full, serde_json::to_vec(value).unwrap()).unwrap();
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":schema,
        "type":kind,
        "path":path,
        "sha256":sha256::sha256(&full),
        "consumedSnapshotIdentity":SNAPSHOT
    })
}

fn admission(root: &Path, name: &str, provider: &str, data: Value) -> Value {
    let payload = json!({"schema":"code-intel-evidence-payload.v1","data":data});
    let payload_ref = write_ref(
        root,
        &format!("{name}-payload.json"),
        "code-intel-evidence-payload.v1",
        "observed.evidence.payload",
        &payload,
    );
    let result = json!({
        "schema":"code-intel-evidence-admissibility-result.v1",
        "status":"admitted",
        "domainVerdict":"observed",
        "admissionIdentity":payload_ref["sha256"],
        "evidence":{
            "provider":{"id":provider},
            "consumedSnapshotIdentity":SNAPSHOT,
            "failure":{"kind":"none"},
            "payload":payload_ref
        },
        "verifiedPayload":{
            "sha256":payload_ref["sha256"],
            "artifactSchema":"code-intel-evidence-payload.v1",
            "type":"observed.evidence.payload",
            "consumedSnapshotIdentity":SNAPSHOT,
            "data":payload["data"]
        },
        "engineeringFacts":[]
    });
    write_ref(
        root,
        &format!("{name}-admission.json"),
        "code-intel-evidence-admissibility-result.v1",
        "evidence.admission",
        &result,
    )
}

fn passing_structural_data() -> Value {
    let policy = json!({"policyId":"evidence-quality-admission","policyVersion":1,
        "sha256":"f97cfed77d80be09acda1331bf449bb21e09ee58d91e19255313bbc5815d11f0"});
    let measurement: Value = serde_json::from_slice(include_bytes!(
        "../../../orchestration/sentrux-gate-policy.v1.json"
    ))
    .unwrap();
    let gate = json!({
        "schema":"code-intel-sentrux-gate-result.v1","policy":policy,
        "measurement":measurement["measurement"],
        "baseline":{"schema":"code-intel-sentrux-baseline.v6","sha256":"b".repeat(64),"sourceCommit":"base","scope":"."},
        "current":{"sourceCommit":"head","scope":"."},"ruleScope":"baseline_ratchet",
        "comparisons":[
            {"rule":"quality_degraded","before":9000,"after":9000,"disposition":"advisory","verdict":"pass"},
            {"rule":"coupling_increased","before":{"couplingScore":4,"importEdges":40},"after":{"couplingScore":4,"importEdges":40},"disposition":"blocking","verdict":"pass"},
            {"rule":"cycles_increased","before":0,"after":0,"disposition":"blocking","verdict":"pass"},
            {"rule":"god_files_increased","before":[],"after":[],"disposition":"blocking","verdict":"pass"}
        ],
        "blockingViolations":[],"advisories":[],"verdict":"pass"
    });
    let mut check = gate.clone();
    check["ruleScope"] = json!("static_and_ratchet");
    json!({"structuralEvidence":{
        "schema":"code-intel-structural-evidence-payload.v2",
        "gatePolicy":policy,"snapshotIdentity":SNAPSHOT,"completeness":"complete",
        "provenance":{"sourceRevision":"head"},
        "gateResults":[{"kind":"sentrux_gate","admission":gate},{"kind":"sentrux_check","admission":check}],
        "advisories":[],
        "rules":[
            {"kind":"sentrux_gate","status":"evaluated","verdict":"pass","failure":{"kind":"none"},"details":{"violations":[]}},
            {"kind":"sentrux_check","status":"evaluated","verdict":"pass","failure":{"kind":"none"},"details":{"violations":[]}}
        ]
    }})
}

fn hospital(root: &Path, inputs: Vec<Value>, name: &str) -> (i32, Option<Value>, String) {
    let registry: Value = serde_json::from_slice(
        &fs::read(pipeline_root().join("orchestration/integrations.json")).unwrap(),
    )
    .unwrap();
    let integration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "diagnosis.hospital")
        .unwrap();
    let request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"diagnosis.hospital",
        "contractVersion":1,
        "implementation":integration["capabilityDeclaration"]["implementation"],
        "snapshot":{
            "identity":SNAPSHOT,
            "repoIdentity":format!("content-v1:{}", "c".repeat(64)),
            "head":"unversioned",
            "workingTreePolicy":"explicit_overlay",
            "scope":["."],
            "inputDigest":"d".repeat(64)
        },
        "options":{},
        "inputs":inputs,
        "effectPolicy":{"allowedEffects":["local_write"]}
    });
    let request_path = root.join(format!("{name}-request.json"));
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let out = root.join(name);
    let output = common::cli()
        .args(["capability", "exec", "diagnosis.hospital", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--artifact-root")
        .arg(root)
        .output()
        .unwrap();
    let report_path = out.join("hospital-report.json");
    let report = fs::read(&report_path)
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap());
    if report.is_some() {
        assert!(
            schema_support::validates_against_schema(
                &report_path,
                &pipeline_root().join("orchestration/schemas/code-intel-hospital.v1.schema.json"),
            ),
            "hospital report must validate against its schema"
        );
    }
    (
        output.status.code().unwrap(),
        report,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const PRECEDENCE: [&str; 7] = [
    "local tool failure",
    "provider quota exhausted",
    "architecture gate failure",
    "authoritative structural evidence unavailable",
    "ungoverned structural scope",
    "known modernization debt",
    "clean snapshot",
];

#[test]
fn hospital_without_graph_evidence_is_not_a_missing_graph() {
    let temp = Temp::new("hospital");
    let structural = admission(
        &temp.0,
        "structural",
        "structural-evidence.sentrux",
        passing_structural_data(),
    );
    let native = admission(
        &temp.0,
        "native",
        "native-code-evidence",
        json!({"nativeCode":{"modernizationDebt":false,"topTarget":""}}),
    );
    let (exit, report, stderr) = hospital(&temp.0, vec![structural, native], "no-graph");
    assert_eq!(exit, 0, "{stderr}");
    let report = report.expect("hospital report");
    assert_eq!(report["triage"]["primary_diagnosis"], "clean snapshot");
    assert_eq!(report["policies"]["precedence"], json!(PRECEDENCE));
}

#[test]
fn hospital_ignores_architecture_graph_data_like_any_unknown_key() {
    let temp = Temp::new("hospital-legacy");
    let structural = admission(
        &temp.0,
        "structural",
        "structural-evidence.sentrux",
        passing_structural_data(),
    );
    let legacy = admission(
        &temp.0,
        "legacy-graph",
        "architecture-graph.internal",
        json!({"architectureGraph":{"completeness":"partial","graph":null}}),
    );
    let (exit, report, stderr) = hospital(&temp.0, vec![structural, legacy], "legacy-graph");
    assert_eq!(exit, 0, "{stderr}");
    let report = report.expect("hospital report");
    assert_ne!(
        report["triage"]["primary_diagnosis"],
        "architecture graph missing"
    );
    assert_eq!(report["policies"]["precedence"], json!(PRECEDENCE));
}

#[test]
fn doctor_observation_keeps_only_the_install_smoke_graph_provider_fields() {
    let temp = Temp::new("doctor");
    let output = common::cli()
        .args(["doctor", "bootstrap", "--pipeline-root"])
        .arg(pipeline_root())
        .arg("--repo-path")
        .arg(&temp.0)
        .args(["--no-require-repowise", "--json"])
        .output()
        .unwrap();
    let observation: Value = serde_json::from_slice(&output.stdout).unwrap();
    let checks = &observation["checks"];
    assert!(
        checks.get(concat!("understand", "Anything")).is_none(),
        "{checks}"
    );
    assert!(
        checks["repo"].get(concat!("understand", "Graph")).is_none(),
        "{checks}"
    );
    let mut keys: Vec<&String> = checks["graphProvider"]
        .as_object()
        .expect("graphProvider stays for install-smoke")
        .keys()
        .collect();
    keys.sort();
    assert_eq!(keys, ["binaryFound", "binaryPath"]);
    assert!(checks["graphProvider"]["binaryFound"].is_boolean());
    let providers = observation.to_string();
    assert!(!providers.contains("graph.code-intel"), "{observation}");
}
