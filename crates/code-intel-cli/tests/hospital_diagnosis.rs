use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-b09-{}-{nonce}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn artifact_ref(root: &Path, path: &str, schema: &str, kind: &str, value: &Value) -> Value {
    let bytes = serde_json::to_vec(value).unwrap();
    let full_path = root.join(path);
    fs::write(&full_path, &bytes).unwrap();
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":schema,
        "type":kind,
        "path":path,
        "sha256":file_sha256(&full_path),
        "consumedSnapshotIdentity":SNAPSHOT
    })
}

fn file_sha256(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = bytes.to_vec();
    let bits = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value);
        }
    }
    h.iter().map(|value| format!("{value:08x}")).collect()
}

fn admission(
    root: &Path,
    name: &str,
    provider: &str,
    domain_verdict: &str,
    failure_kind: &str,
    data: Value,
) -> Value {
    let payload = json!({"schema":"code-intel-evidence-payload.v1","data":data});
    let payload_ref = artifact_ref(
        root,
        &format!("{name}-payload.json"),
        "code-intel-evidence-payload.v1",
        "observed.evidence.payload",
        &payload,
    );
    let admission_seed = root.join(format!("{name}-identity.json"));
    fs::write(
        &admission_seed,
        serde_json::to_vec(&json!({"provider":provider,"name":name,"data":payload})).unwrap(),
    )
    .unwrap();
    let admission_identity = file_sha256(&admission_seed);
    let result = json!({
        "schema":"code-intel-evidence-admissibility-result.v1",
        "status":"admitted",
        "domainVerdict":domain_verdict,
        "admissionIdentity":admission_identity,
        "evidence":{
            "schema":"code-intel-observed-evidence.v1",
            "provider":{"id":provider,"implementation":{"id":format!("{provider}.fixture"),"version":"1.0.0","digest":"b".repeat(64)}},
            "source":{"revision":"fixture-r1"},
            "consumedSnapshotIdentity":SNAPSHOT,
            "observedAt":1700000000,
            "completeness":if domain_verdict == "observed" { "complete" } else { "partial" },
            "claimedComplete":domain_verdict == "observed",
            "payload":payload_ref,
            "provenance":{"collectionId":format!("fixture-{name}"),"command":"fixture","startedAt":1699999999,"completedAt":1700000000},
            "failure":if failure_kind == "none" { json!({"kind":"none"}) } else { json!({"kind":failure_kind,"message":failure_kind}) }
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
    artifact_ref(
        root,
        &format!("{name}-admission.json"),
        "code-intel-evidence-admissibility-result.v1",
        "evidence.admission",
        &result,
    )
}

fn snapshot() -> Value {
    json!({
        "identity":SNAPSHOT,
        "repoIdentity":format!("content-v1:{}", "c".repeat(64)),
        "head":"unversioned",
        "workingTreePolicy":"explicit_overlay",
        "scope":["."],
        "inputDigest":"d".repeat(64)
    })
}

fn registry_integration() -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry: Value =
        serde_json::from_slice(&fs::read(root.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "diagnosis.hospital")
        .unwrap()
        .clone()
}

fn run(root: &Path, inputs: Vec<Value>, out_name: &str) -> (i32, Value, PathBuf, String) {
    let integration = registry_integration();
    let request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"diagnosis.hospital",
        "contractVersion":1,
        "implementation":integration["capabilityDeclaration"]["implementation"],
        "snapshot":snapshot(),
        "options":{},
        "inputs":inputs,
        "effectPolicy":{"allowedEffects":["local_write"]}
    });
    let request_path = root.join(format!("{out_name}-request.json"));
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let out = root.join(out_name);
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "diagnosis.hospital", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--artifact-root")
        .arg(root)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (
        output.status.code().unwrap(),
        value,
        out,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn graph(root: &Path, current: bool) -> Value {
    admission(
        root,
        if current {
            "graph-current"
        } else {
            "graph-missing"
        },
        "architecture-graph.internal",
        if current { "observed" } else { "unknown" },
        if current { "none" } else { "domain_unknown" },
        json!({"architectureGraph":{
            "schema":"code-intel-architecture-graph-evidence.v1",
            "snapshotIdentity":SNAPSHOT,
            "completeness":if current { "complete" } else { "partial" },
            "graph":if current { json!({"nodes":[],"edges":[]}) } else { Value::Null }
        }}),
    )
}

fn structural(root: &Path, name: &str, verdict: Option<&str>, trusted: bool) -> Value {
    let rules = verdict
        .map(|value| json!([{"kind":"boundary_dependency","status":"evaluated","verdict":value,"failure":{"kind":"none"}}]))
        .unwrap_or_else(|| json!([]));
    admission(
        root,
        name,
        "structural-evidence.sentrux",
        if trusted { "observed" } else { "unknown" },
        if trusted { "none" } else { "domain_unknown" },
        json!({"structuralEvidence":{
            "schema":"code-intel-structural-evidence-payload.v1",
            "snapshotIdentity":SNAPSHOT,
            "completeness":if trusted { "complete" } else { "partial" },
            "rules":rules
        }}),
    )
}

fn native(root: &Path, debt: bool) -> Value {
    admission(
        root,
        if debt { "native-debt" } else { "native-clean" },
        "native-code-evidence",
        "observed",
        "none",
        json!({"nativeCode":{"modernizationDebt":debt,"topTarget":if debt { "src/legacy.rs" } else { "" }}}),
    )
}

fn diagnosis(root: &Path, inputs: Vec<Value>, name: &str) -> (i32, String, String, PathBuf) {
    let (exit, _, out, stderr) = run(root, inputs, name);
    if !out.join("hospital-report.json").is_file() {
        return (exit, stderr, String::new(), out);
    }
    let value: Value =
        serde_json::from_slice(&fs::read(out.join("hospital-report.json")).unwrap()).unwrap();
    (
        exit,
        value["triage"]["primary_diagnosis"]
            .as_str()
            .unwrap()
            .into(),
        value["triage"]["next_protocol"].as_str().unwrap().into(),
        out,
    )
}

#[test]
fn provider_quota_precedes_missing_current_graph_and_is_replay_stable() {
    let temp = Temp::new();
    let repowise = admission(
        &temp.0,
        "repowise-docs",
        "repowise.docs",
        "unknown",
        "provider_unavailable",
        json!({"repowise":{"channel":"docs","status":"quota"}}),
    );
    let graph = admission(
        &temp.0,
        "graph",
        "architecture-graph.internal",
        "unknown",
        "domain_unknown",
        json!({"architectureGraph":{"schema":"code-intel-architecture-graph-evidence.v1","snapshotIdentity":SNAPSHOT,"completeness":"partial","graph":null}}),
    );

    let (exit, envelope, out, stderr) =
        run(&temp.0, vec![repowise.clone(), graph.clone()], "first");
    assert_eq!(exit, 0, "{stderr}");
    assert_eq!(envelope["status"], "completed");
    assert_eq!(envelope["observedEffects"], json!(["local_write"]));
    let emitted = envelope["artifacts"]
        .as_array()
        .expect("B09 emits Artifact Refs");
    assert_eq!(emitted.len(), 4);
    assert_eq!(
        emitted
            .iter()
            .map(|artifact| artifact["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "diagnosis.hospital",
            "diagnosis.hospital-view",
            "diagnosis.surgery-plan",
            "diagnosis.surgery-plan-view",
        ]
    );
    let machine: Value =
        serde_json::from_slice(&fs::read(out.join("hospital-report.json")).unwrap()).unwrap();
    assert_eq!(machine["schema"], "code-intel-hospital.v1");
    assert_eq!(machine["domainVerdict"], "unknown");
    assert_eq!(machine["triage"]["status"], "unknown");
    assert_eq!(
        machine["triage"]["primary_diagnosis"],
        "provider quota exhausted"
    );
    assert_eq!(machine["triage"]["disposition"], "admit");
    assert_eq!(machine["triage"]["next_protocol"], "triage");
    assert!(machine["treatment"]["plan"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("provider quota")));
    assert_eq!(machine["surgery_plan"]["status"], "not_required");

    let (replay_exit, _, replay_out, replay_stderr) = run(&temp.0, vec![graph, repowise], "replay");
    assert_eq!(replay_exit, 0, "{replay_stderr}");
    assert_eq!(
        fs::read(out.join("hospital-report.json")).unwrap(),
        fs::read(replay_out.join("hospital-report.json")).unwrap(),
        "input order and output path must not change the machine diagnosis"
    );
    let markdown = fs::read_to_string(out.join("hospital.md")).unwrap();
    assert!(markdown.contains("Primary diagnosis: provider quota exhausted"));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-Command", "param($Document,$Schema); if (-not (Get-Content -Raw -LiteralPath $Document | Test-Json -SchemaFile $Schema -ErrorAction Stop)) { exit 1 }"])
        .arg(out.join("hospital-report.json"))
        .arg(repo.join("orchestration/schemas/code-intel-hospital.v1.schema.json"))
        .output()
        .unwrap();
    assert!(
        schema.status.success(),
        "{}",
        String::from_utf8_lossy(&schema.stderr)
    );
}

#[test]
fn provider_identity_spoofs_cannot_supply_any_admitted_modality() {
    let temp = Temp::new();
    let cases = [
        (
            "graph-spoof",
            "repowise.docs-graph-spoof",
            json!({"architectureGraph":{"schema":"code-intel-architecture-graph-evidence.v1","snapshotIdentity":SNAPSHOT,"completeness":"complete","graph":{"nodes":[],"edges":[]}}}),
        ),
        (
            "structural-spoof",
            "structural-evidence.sentrux-spoof",
            json!({"structuralEvidence":{"schema":"code-intel-structural-evidence-payload.v1","snapshotIdentity":SNAPSHOT,"completeness":"complete","rules":[{"kind":"boundary_dependency","status":"evaluated","verdict":"pass","failure":{"kind":"none"}}]}}),
        ),
        (
            "native-spoof",
            "native-code-evidence-spoof",
            json!({"nativeCode":{"modernizationDebt":false,"topTarget":""}}),
        ),
    ];
    for (name, provider, data) in cases {
        let spoof = admission(&temp.0, name, provider, "observed", "none", data);
        let (exit, _, out, _) = run(&temp.0, vec![spoof], &format!("{name}-out"));
        assert_eq!(exit, 65, "spoof provider {provider} must fail closed");
        assert!(!out.join("hospital-report.json").exists());
        assert!(!out.join("hospital.md").exists());
        assert!(!out.join("surgery-plan.json").exists());
        assert!(!out.join("surgery-plan.md").exists());
    }
}

#[test]
fn repowise_quota_signal_requires_an_exact_provider_identity() {
    let temp = Temp::new();
    let spoof = admission(
        &temp.0,
        "repowise-spoof",
        "repowise.docs-spoof",
        "unknown",
        "provider_unavailable",
        json!({"repowise":{"channel":"docs","status":"quota"}}),
    );
    let (exit, _, out, _) = run(&temp.0, vec![spoof, graph(&temp.0, false)], "quota-spoof");
    assert_eq!(exit, 65);
    assert!(!out.join("hospital-report.json").exists());
    assert!(!out.join("hospital.md").exists());
    assert!(!out.join("surgery-plan.json").exists());
    assert!(!out.join("surgery-plan.md").exists());
}

#[test]
fn precedence_matrix_matches_the_legacy_stable_diagnoses_and_fails_closed() {
    let temp = Temp::new();
    let cases = vec![
        (
            "local-first",
            vec![
                admission(
                    &temp.0,
                    "local",
                    "repowise.index",
                    "unknown",
                    "local_tool_error",
                    json!({"repowise":{"status":"unavailable"}}),
                ),
                admission(
                    &temp.0,
                    "quota-2",
                    "repowise.docs",
                    "unknown",
                    "provider_unavailable",
                    json!({"repowise":{"status":"quota"}}),
                ),
            ],
            "local tool failure",
            "triage",
        ),
        (
            "gate-before-graph",
            vec![
                graph(&temp.0, false),
                structural(&temp.0, "structural-fail", Some("fail"), true),
            ],
            "architecture gate failure",
            "govern",
        ),
        (
            "graph-missing",
            vec![
                graph(&temp.0, false),
                structural(&temp.0, "structural-pass-1", Some("pass"), true),
            ],
            "architecture graph missing",
            "diagnose",
        ),
        (
            "authoritative-missing",
            vec![graph(&temp.0, true), native(&temp.0, false)],
            "authoritative structural evidence unavailable",
            "diagnose",
        ),
        (
            "authoritative-untrusted",
            vec![
                graph(&temp.0, true),
                structural(&temp.0, "structural-unknown", Some("unknown"), false),
                native(&temp.0, false),
            ],
            "authoritative structural evidence unavailable",
            "diagnose",
        ),
        (
            "ungoverned",
            vec![
                graph(&temp.0, true),
                structural(&temp.0, "structural-empty", None, true),
            ],
            "ungoverned structural scope",
            "govern",
        ),
        (
            "modernization",
            vec![
                graph(&temp.0, true),
                structural(&temp.0, "structural-pass-2", Some("pass"), true),
                native(&temp.0, true),
            ],
            "known modernization debt",
            "surgery_plan",
        ),
        (
            "clean",
            vec![
                graph(&temp.0, true),
                structural(&temp.0, "structural-pass-3", Some("pass"), true),
                native(&temp.0, false),
            ],
            "clean snapshot",
            "post_op",
        ),
    ];
    for (name, inputs, expected_diagnosis, expected_protocol) in cases {
        let (exit, actual_diagnosis, actual_protocol, _) = diagnosis(&temp.0, inputs, name);
        let expected_exit = if matches!(expected_protocol, "govern" | "surgery_plan") {
            10
        } else {
            0
        };
        assert_eq!(exit, expected_exit, "case={name}: {actual_diagnosis}");
        assert_eq!(actual_diagnosis, expected_diagnosis, "case={name}");
        assert_eq!(actual_protocol, expected_protocol, "case={name}");
    }
}

#[test]
fn missing_or_non_admitted_authority_is_rejected_and_enrichment_never_overrides_it() {
    let temp = Temp::new();
    let (exit, diagnosis_text, protocol, _) = diagnosis(
        &temp.0,
        vec![graph(&temp.0, true), native(&temp.0, true)],
        "missing-authority",
    );
    assert_eq!(exit, 0);
    assert_eq!(
        diagnosis_text,
        "authoritative structural evidence unavailable"
    );
    assert_eq!(protocol, "diagnose");

    let (untrusted_exit, untrusted_diagnosis, untrusted_protocol, untrusted_out) = diagnosis(
        &temp.0,
        vec![
            graph(&temp.0, true),
            structural(&temp.0, "untrusted-with-target", Some("unknown"), false),
            native(&temp.0, true),
        ],
        "untrusted-authority-with-enrichment",
    );
    assert_eq!(untrusted_exit, 0);
    assert_eq!(
        untrusted_diagnosis,
        "authoritative structural evidence unavailable"
    );
    assert_eq!(untrusted_protocol, "diagnose");
    let untrusted_machine: Value =
        serde_json::from_slice(&fs::read(untrusted_out.join("hospital-report.json")).unwrap())
            .unwrap();
    assert_eq!(untrusted_machine["domainVerdict"], "unknown");
    assert_eq!(untrusted_machine["surgery_plan"]["status"], "not_required");

    let mut rejected = structural(&temp.0, "rejected-structural", Some("pass"), true);
    let path = temp.0.join(rejected["path"].as_str().unwrap());
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["status"] = json!("rejected");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    rejected["sha256"] = json!(file_sha256(&path));
    let (rejected_exit, _, out, stderr) = run(
        &temp.0,
        vec![graph(&temp.0, true), rejected, native(&temp.0, true)],
        "rejected-authority",
    );
    assert_eq!(rejected_exit, 65, "{stderr}");
    assert!(!out.join("hospital-report.json").exists());
}

#[test]
fn conflicting_or_provider_injected_modalities_fail_closed_independent_of_input_order() {
    let temp = Temp::new();
    let first = admission(
        &temp.0,
        "graph-provider-a",
        "architecture-graph.provider-a",
        "observed",
        "none",
        json!({"architectureGraph":{"completeness":"complete","graph":{"nodes":[],"edges":[]}}}),
    );
    let second = admission(
        &temp.0,
        "graph-provider-b",
        "architecture-graph.provider-b",
        "unknown",
        "domain_unknown",
        json!({"architectureGraph":{"completeness":"partial","graph":null}}),
    );
    let (forward_exit, forward, forward_out, forward_stderr) = run(
        &temp.0,
        vec![first.clone(), second.clone()],
        "conflict-forward",
    );
    let (reverse_exit, reverse, reverse_out, reverse_stderr) =
        run(&temp.0, vec![second, first], "conflict-reverse");
    assert_eq!(forward_exit, 65, "{forward_stderr}");
    assert_eq!(reverse_exit, 65, "{reverse_stderr}");
    assert_eq!(forward["status"], reverse["status"]);
    assert_eq!(forward["verdict"], reverse["verdict"]);
    assert_eq!(forward["diagnostics"], reverse["diagnostics"]);
    assert!(!forward_out.join("hospital-report.json").exists());
    assert!(!reverse_out.join("hospital-report.json").exists());

    let injected = admission(
        &temp.0,
        "injected-graph",
        "repowise.docs",
        "observed",
        "none",
        json!({"architectureGraph":{"completeness":"complete","graph":{"nodes":[],"edges":[]}}}),
    );
    let (injected_exit, _, injected_out, injected_stderr) =
        run(&temp.0, vec![injected], "provider-injected");
    assert_eq!(injected_exit, 65, "{injected_stderr}");
    assert!(!injected_out.join("hospital-report.json").exists());
}

#[test]
fn markdown_is_a_rebuildable_view_and_cannot_change_the_machine_verdict() {
    let temp = Temp::new();
    let inputs = vec![
        graph(&temp.0, true),
        structural(&temp.0, "render-pass", Some("pass"), true),
    ];
    let (exit, _, _, out) = diagnosis(&temp.0, inputs.clone(), "render-first");
    assert_eq!(exit, 0);
    let machine = fs::read(out.join("hospital-report.json")).unwrap();
    fs::write(
        out.join("hospital.md"),
        "# forged\nPrimary diagnosis: clean snapshot\n",
    )
    .unwrap();
    let (replay_exit, _, _, replay_out) = diagnosis(&temp.0, inputs, "render-rebuilt");
    assert_eq!(replay_exit, 0);
    assert_eq!(
        machine,
        fs::read(replay_out.join("hospital-report.json")).unwrap()
    );
    let rebuilt = fs::read_to_string(replay_out.join("hospital.md")).unwrap();
    assert!(rebuilt.contains("Primary diagnosis: clean snapshot"));
    assert!(!rebuilt.contains("# forged"));
}

fn dynamic_artifact_ref(
    root: &Path,
    path: &str,
    schema: &str,
    kind: &str,
    snapshot_identity: &str,
    value: &Value,
) -> Value {
    let full_path = root.join(path);
    fs::write(&full_path, serde_json::to_vec(value).unwrap()).unwrap();
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":schema,
        "type":kind,
        "path":path,
        "sha256":file_sha256(&full_path),
        "consumedSnapshotIdentity":snapshot_identity
    })
}

fn dynamic_admission(
    root: &Path,
    name: &str,
    provider: &str,
    snapshot_identity: &str,
    data: Value,
) -> Value {
    let payload = json!({"schema":"code-intel-evidence-payload.v1","data":data});
    let payload_ref = dynamic_artifact_ref(
        root,
        &format!("{name}-payload.json"),
        "code-intel-evidence-payload.v1",
        "observed.evidence.payload",
        snapshot_identity,
        &payload,
    );
    let result = json!({
        "schema":"code-intel-evidence-admissibility-result.v1",
        "status":"admitted",
        "domainVerdict":"observed",
        "admissionIdentity":payload_ref["sha256"],
        "evidence":{
            "provider":{"id":provider},
            "consumedSnapshotIdentity":snapshot_identity,
            "failure":{"kind":"none"},
            "payload":payload_ref
        },
        "verifiedPayload":{
            "sha256":payload_ref["sha256"],
            "artifactSchema":"code-intel-evidence-payload.v1",
            "type":"observed.evidence.payload",
            "consumedSnapshotIdentity":snapshot_identity,
            "data":payload["data"]
        },
        "engineeringFacts":[]
    });
    dynamic_artifact_ref(
        root,
        &format!("{name}-admission.json"),
        "code-intel-evidence-admissibility-result.v1",
        "evidence.admission",
        snapshot_identity,
        &result,
    )
}

#[test]
fn a09_seeded_path_executes_hospital_through_a01_and_rejects_snapshot_mismatch() {
    let temp = Temp::new();
    let repo = temp.0.join("repo");
    let seed = temp.0.join("seed");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&seed).unwrap();
    fs::write(repo.join("source.txt"), "stable fixture\n").unwrap();
    let snapshot_output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "snapshot",
            "identity",
            "--repo",
            repo.to_str().unwrap(),
            "--working-tree-policy",
            "explicit_overlay",
            "--scope",
            ".",
        ])
        .output()
        .unwrap();
    assert!(snapshot_output.status.success());
    let snapshot_document: Value = serde_json::from_slice(&snapshot_output.stdout).unwrap();
    let snapshot_identity = snapshot_document["snapshot"]["identity"].as_str().unwrap();
    let inputs = vec![
        dynamic_admission(
            &seed,
            "graph",
            "architecture-graph.internal",
            snapshot_identity,
            json!({"architectureGraph":{"completeness":"complete","graph":{"nodes":[],"edges":[]}}}),
        ),
        dynamic_admission(
            &seed,
            "structure",
            "structural-evidence.sentrux",
            snapshot_identity,
            json!({"structuralEvidence":{"completeness":"complete","rules":[{"verdict":"pass"}]}}),
        ),
    ];
    let inputs_path = temp.0.join("diagnosis-inputs.json");
    fs::write(&inputs_path, serde_json::to_vec(&inputs).unwrap()).unwrap();
    let out = temp.0.join("a09-run");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&out)
        .arg("--diagnosis-inputs")
        .arg(&inputs_path)
        .arg("--seed-artifact-root")
        .arg(&seed)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["schema"], "code-intel-run-manifest.v1");
    let hospital_path = out.join("diagnosis.hospital/hospital-report.json");
    let hospital: Value = serde_json::from_slice(&fs::read(hospital_path).unwrap()).unwrap();
    assert_eq!(hospital["triage"]["primary_diagnosis"], "clean snapshot");

    let mut mismatched = inputs;
    mismatched[0]["consumedSnapshotIdentity"] = json!("f".repeat(64));
    let mismatched_path = temp.0.join("mismatched-inputs.json");
    fs::write(&mismatched_path, serde_json::to_vec(&mismatched).unwrap()).unwrap();
    let failed_out = temp.0.join("a09-failed");
    let failed = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(&failed_out)
        .arg("--diagnosis-inputs")
        .arg(&mismatched_path)
        .arg("--seed-artifact-root")
        .arg(&seed)
        .output()
        .unwrap();
    assert_eq!(
        failed.status.code(),
        Some(65),
        "stdout={} stderr={} failed_out={} exists={}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr),
        failed_out.display(),
        failed_out.exists()
    );
    assert!(!failed_out
        .join("diagnosis.hospital/hospital-report.json")
        .exists());
}

#[test]
fn legacy_facade_and_rust_execute_the_same_fixture_with_stable_machine_parity() {
    let temp = Temp::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = fs::read_to_string(root.join("run-code-intel.ps1")).unwrap();
    let main = source
        .find("\n$configData = $null")
        .expect("legacy function boundary");
    let function_source = source[..main].replace(
        "$PSScriptRoot",
        &format!("'{}'", root.to_string_lossy().replace('\\', "/")),
    );
    let legacy_out = temp.0.join("legacy");
    fs::create_dir(&legacy_out).unwrap();
    let seam = format!(
        "{}\n{}",
        function_source,
        r#"
$out = $ArtifactRoot
$failureCounts = [ordered]@{ localToolError = 1; providerQuota = 0; sentruxFail = 0; graphMissing = 0 }
$steps = @(
  [pscustomobject]@{ name='git status'; status='failed'; output=''; error='fixture'; exitCode=1; durationMs=1 },
  [pscustomobject]@{ name='rg file inventory'; status='ok'; output='2'; error=''; exitCode=0; durationMs=1 },
  [pscustomobject]@{ name='understand graph'; status='ok'; output=''; error=''; exitCode=0; durationMs=1 },
  [pscustomobject]@{ name='repowise fixture'; status='ok'; output=''; error=''; exitCode=0; durationMs=1 },
  [pscustomobject]@{ name='sentrux check'; status='ok'; output=''; error=''; exitCode=0; durationMs=1 },
  [pscustomobject]@{ name='sentrux gate fixture'; status='ok'; output=''; error=''; exitCode=0; durationMs=1 }
)
$hospital = New-CodeIntelHospitalReport -RepoPath $RepoPath -Mode 'normal' -RunDir $out -ReportPath (Join-Path $out 'report.json') -SummaryPath (Join-Path $out 'summary.md') -UnderstandingPath (Join-Path $out 'understanding.md') -Steps $steps -FailureCounts $failureCounts -SentruxInsight ([ordered]@{}) -SentruxDsmSummary $null -SentruxFileDetailsSummary $null -SentruxHotspotsSummary $null -SentruxEvolutionSummary $null -SentruxWhatIfSummary $null -CodeNexusContextSummary $null -UnderstandCommand 'fixture' -ToolState ([ordered]@{}) -GitHubResearch $null
$surgery = New-CodeIntelSurgeryPlan -Hospital $hospital -RepoPath $RepoPath -SentruxTargetPath $RepoPath -HotspotsPath '' -WhatIfPath '' -CodeNexusPath ''
$hospital['surgery_plan'] = [ordered]@{ path=(Join-Path $out 'surgery-plan.json'); markdown=(Join-Path $out 'surgery-plan.md'); status=$surgery.status; primary_target='' }
$hospital | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $out 'hospital-report.json') -Encoding utf8NoBOM
Convert-HospitalReportToMarkdown $hospital | Set-Content -LiteralPath (Join-Path $out 'hospital.md') -Encoding utf8NoBOM
$surgery | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $out 'surgery-plan.json') -Encoding utf8NoBOM
Convert-SurgeryPlanToMarkdown $surgery | Set-Content -LiteralPath (Join-Path $out 'surgery-plan.md') -Encoding utf8NoBOM
[ordered]@{ primary_diagnosis=$hospital.triage.primary_diagnosis; disposition=$hospital.triage.disposition; next_protocol=$hospital.triage.next_protocol; surgery_status=$surgery.status } | ConvertTo-Json -Compress
"#,
    );
    let seam_path = temp.0.join("legacy-hospital-seam.ps1");
    fs::write(&seam_path, seam).unwrap();
    let legacy = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(&seam_path)
        .arg("-RepoPath")
        .arg(&temp.0)
        .arg("-ArtifactRoot")
        .arg(&legacy_out)
        .output()
        .unwrap();
    assert!(
        legacy.status.success(),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    let legacy_machine: Value =
        serde_json::from_slice(legacy.stdout.trim_ascii()).expect("legacy machine JSON");

    let local_failure = admission(
        &temp.0,
        "legacy-parity-local",
        "repowise.index",
        "unknown",
        "local_tool_error",
        json!({"repowise":{"status":"unavailable"}}),
    );
    let (exit, _, rust_out, stderr) = run(&temp.0, vec![local_failure], "rust-parity");
    assert_eq!(exit, 0, "{stderr}");
    let rust_machine: Value =
        serde_json::from_slice(&fs::read(rust_out.join("hospital-report.json")).unwrap()).unwrap();
    assert_eq!(
        legacy_machine["primary_diagnosis"],
        rust_machine["triage"]["primary_diagnosis"]
    );
    assert_eq!(
        legacy_machine["disposition"],
        rust_machine["triage"]["disposition"]
    );
    assert_eq!(
        legacy_machine["next_protocol"],
        rust_machine["triage"]["next_protocol"]
    );
    assert_eq!(
        legacy_machine["surgery_status"],
        rust_machine["surgery_plan"]["status"]
    );
    for file in [
        "hospital-report.json",
        "hospital.md",
        "surgery-plan.json",
        "surgery-plan.md",
    ] {
        assert!(legacy_out.join(file).is_file(), "legacy omitted {file}");
        assert!(rust_out.join(file).is_file(), "Rust omitted {file}");
    }
}
