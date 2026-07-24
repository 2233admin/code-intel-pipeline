use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-d04-{}-{nonce}-{sequence}",
            std::process::id()
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

fn sha256(path: &Path) -> String {
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

fn event(
    id: &str,
    kind: &str,
    subject: &str,
    start: u64,
    end: u64,
    mandatory: bool,
    coordination_need: Option<&str>,
    predecessor: Option<&str>,
) -> Value {
    event_with_predecessors(
        id,
        kind,
        subject,
        start,
        end,
        mandatory,
        coordination_need,
        predecessor.into_iter().collect(),
    )
}

fn event_with_predecessors(
    id: &str,
    kind: &str,
    subject: &str,
    start: u64,
    end: u64,
    mandatory: bool,
    coordination_need: Option<&str>,
    predecessors: Vec<&str>,
) -> Value {
    json!({
        "id":id,
        "kind":kind,
        "subject":subject,
        "startedAtMs":start,
        "completedAtMs":end,
        "mandatory":mandatory,
        "coordinationNeed":coordination_need,
        "predecessors":predecessors
    })
}

fn artifact_ref(schema: &str, artifact_type: &str, path: &str, sha256: &str) -> Value {
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":schema,
        "type":artifact_type,
        "path":path,
        "sha256":sha256,
        "consumedSnapshotIdentity":SNAPSHOT
    })
}

fn write_artifact(root: &Path, relative: &str, value: &Value) -> Value {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    json!({"path":relative.replace('\\', "/"),"sha256":sha256(&path)})
}

fn committed_trace(root: &Path, label: &str, run: &str, events: Vec<Value>) -> (Value, Vec<Value>) {
    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":run,
        "snapshotIdentity":SNAPSHOT,
        "outcome":"completed",
        "nodes":{"measurement":{"status":"succeeded","verdict":"pass","artifacts":[]}}
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let manifest_sha = sha256_hex(&manifest_bytes);
    let manifest_path = format!("objects/sha256/{manifest_sha}");
    fs::create_dir_all(root.join("objects/sha256")).unwrap();
    fs::write(root.join(&manifest_path), manifest_bytes).unwrap();
    let manifest_ref = artifact_ref(
        "code-intel-run-manifest.v1",
        "run.manifest",
        &manifest_path,
        &manifest_sha,
    );
    let commit = json!({
        "schema":"code-intel-run-commit.v1",
        "runIdentity":run,
        "snapshotIdentity":SNAPSHOT,
        "manifest":{"path":manifest_path,"sha256":manifest_sha}
    });
    let commit_meta = write_artifact(root, &format!("commits/{label}.json"), &commit);
    let commit_ref = artifact_ref(
        "code-intel-run-commit.v1",
        "run.commit",
        commit_meta["path"].as_str().unwrap(),
        commit_meta["sha256"].as_str().unwrap(),
    );
    (
        json!({"commitRef":commit_ref,"events":events}),
        vec![commit_ref, manifest_ref],
    )
}

fn method_inputs(root: &Path) -> Vec<Value> {
    [
        (
            "code-intel-method-catalog.v1",
            "method.catalog",
            "methods/catalog.v1.json",
            "../../orchestration/methods/catalog.v1.json",
        ),
        (
            "code-intel-method-card.v1",
            "method.card",
            "methods/cards/critical-path-pert.v1.json",
            "../../orchestration/methods/cards/critical-path-pert.v1.json",
        ),
        (
            "code-intel-method-card.v1",
            "method.card",
            "methods/cards/value-stream-queue-delay.v1.json",
            "../../orchestration/methods/cards/value-stream-queue-delay.v1.json",
        ),
    ]
    .into_iter()
    .map(|(schema, artifact_type, relative, source)| {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(source);
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(source, &target).unwrap();
        artifact_ref(schema, artifact_type, relative, &sha256(&target))
    })
    .collect()
}

fn request(inputs: Vec<Value>, snapshot: &str) -> Value {
    let registry: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/integrations.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let declaration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "delivery.light-speed-measure");
    let implementation = declaration
        .map(|entry| entry["capabilityDeclaration"]["implementation"].clone())
        .unwrap_or_else(|| {
            json!({"id":"delivery.light-speed-measure.compat","version":"1.0.0","toolchainDigests":["f".repeat(64)]})
        });
    json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"delivery.light-speed-measure",
        "contractVersion":1,
        "implementation":implementation,
        "snapshot":{
            "identity":snapshot,
            "repoIdentity":format!("content-v1:{}", "c".repeat(64)),
            "head":"d04-current",
            "workingTreePolicy":"explicit_overlay",
            "scope":["."],
            "inputDigest":"d".repeat(64)
        },
        "options":{},
        "inputs":inputs,
        "effectPolicy":{"allowedEffects":["local_write"]}
    })
}

fn execute(root: &Path, request: &Value, name: &str) -> (i32, Vec<u8>, Vec<u8>, PathBuf) {
    let request_path = root.join(format!("{name}-request.json"));
    fs::write(&request_path, serde_json::to_vec(request).unwrap()).unwrap();
    let out = root.join(format!("{name}-out"));
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "delivery.light-speed-measure",
            "--request",
        ])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--artifact-root")
        .arg(root)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(70),
        output.stdout,
        output.stderr,
        out,
    )
}

#[test]
fn committed_trace_attributes_queue_protects_mandatory_tests_and_reports_deterministic_delta() {
    let temp = Temp::new();
    let (baseline, mut inputs) = committed_trace(
        &temp.0,
        "baseline",
        &format!("dag-v1:{}", "1a".repeat(16)),
        vec![
            event(
                "b01",
                "technical_work",
                "implementation",
                0,
                100,
                false,
                None,
                None,
            ),
            event(
                "b02",
                "queue",
                "review-queue",
                100,
                150,
                false,
                None,
                Some("b01"),
            ),
            event(
                "b03",
                "handoff",
                "developer-reviewer",
                150,
                170,
                false,
                None,
                Some("b02"),
            ),
            event(
                "b04",
                "understanding",
                "core",
                170,
                200,
                false,
                None,
                Some("b03"),
            ),
            event(
                "b05",
                "understanding",
                "core",
                200,
                225,
                false,
                None,
                Some("b04"),
            ),
            event(
                "b06",
                "test",
                "mandatory-regression",
                225,
                265,
                true,
                None,
                Some("b05"),
            ),
            event(
                "b07",
                "rework",
                "defect-repair",
                265,
                280,
                false,
                None,
                Some("b06"),
            ),
            event(
                "b08",
                "coordination",
                "status-sync",
                280,
                290,
                false,
                Some("unnecessary"),
                Some("b07"),
            ),
            event(
                "b09",
                "coordination",
                "release-approval",
                290,
                295,
                true,
                Some("required"),
                Some("b08"),
            ),
        ],
    );
    let (current, current_inputs) = committed_trace(
        &temp.0,
        "current",
        &format!("dag-v1:{}", "2b".repeat(16)),
        vec![
            event(
                "c01",
                "technical_work",
                "implementation",
                0,
                110,
                false,
                None,
                None,
            ),
            event(
                "c02",
                "queue",
                "review-queue",
                110,
                130,
                false,
                None,
                Some("c01"),
            ),
            event(
                "c03",
                "handoff",
                "developer-reviewer",
                130,
                140,
                false,
                None,
                Some("c02"),
            ),
            event(
                "c04",
                "understanding",
                "core",
                140,
                165,
                false,
                None,
                Some("c03"),
            ),
            event(
                "c05",
                "understanding",
                "core",
                165,
                170,
                false,
                None,
                Some("c04"),
            ),
            event(
                "c06",
                "test",
                "mandatory-regression",
                170,
                220,
                true,
                None,
                Some("c05"),
            ),
            event(
                "c07",
                "rework",
                "defect-repair",
                220,
                225,
                false,
                None,
                Some("c06"),
            ),
            event(
                "c08",
                "coordination",
                "release-approval",
                225,
                230,
                true,
                Some("required"),
                Some("c07"),
            ),
        ],
    );
    let telemetry = json!({
        "schema":"code-intel-run-timing-events.v1",
        "measurementSnapshotIdentity":SNAPSHOT,
        "telemetry":{"mode":"local_opt_in","clock":"monotonic_elapsed_ms","externalPlatform":false},
        "baseline":baseline,
        "current":current
    });
    inputs.extend(current_inputs);
    inputs.extend(method_inputs(&temp.0));
    let telemetry_path = temp.0.join("timing-events.json");
    fs::write(&telemetry_path, serde_json::to_vec(&telemetry).unwrap()).unwrap();
    inputs.push(artifact_ref(
        "code-intel-run-timing-events.v1",
        "delivery.run-timing-events",
        "timing-events.json",
        &sha256(&telemetry_path),
    ));
    let valid_request = request(inputs.clone(), SNAPSHOT);

    let (left_exit, _, left_stderr, left_out) = execute(&temp.0, &valid_request, "left");
    assert_eq!(left_exit, 0, "{}", String::from_utf8_lossy(&left_stderr));
    let (right_exit, _, right_stderr, right_out) = execute(&temp.0, &valid_request, "right");
    assert_eq!(right_exit, 0, "{}", String::from_utf8_lossy(&right_stderr));
    let left_bytes = fs::read(left_out.join("light-speed-report.json")).unwrap();
    let right_bytes = fs::read(right_out.join("light-speed-report.json")).unwrap();
    assert_eq!(
        left_bytes, right_bytes,
        "same committed traces must replay byte-for-byte"
    );
    let report: Value = serde_json::from_slice(&left_bytes).unwrap();
    assert_eq!(report["schema"], "code-intel-delivery-light-speed.v1");
    assert_eq!(
        report["authority"],
        "derived_measurement_no_schedule_commitment"
    );
    assert_eq!(report["baseline"]["categories"]["queueMs"], 50);
    assert_eq!(
        report["baseline"]["categories"]["mandatoryVerificationMs"],
        40
    );
    assert_eq!(
        report["baseline"]["categories"]["repeatedUnderstandingMs"],
        25
    );
    assert_eq!(
        report["baseline"]["categories"]["unnecessaryCoordinationMs"],
        10
    );
    assert_eq!(report["baseline"]["avoidableDelayMs"], 120);
    assert_eq!(report["baseline"]["criticalPath"]["queueMs"], 50);
    assert_eq!(
        report["baseline"]["criticalPath"]["mandatoryVerificationMs"],
        40
    );
    assert_eq!(report["delta"]["avoidableDelayMs"], -80);
    assert_eq!(report["delta"]["leadTimeMs"], -65);
    assert_eq!(
        report["baseline"]["commitRef"],
        telemetry["baseline"]["commitRef"]
    );
    assert_eq!(
        report["method"]["provenance"]["methodCardSha256"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(report.get("schedulePromise").is_none());
    let queue_provenance = report["baseline"]["provenance"]["queueMs"]
        .as_object()
        .unwrap();
    assert_eq!(
        queue_provenance["sourceArtifactSha256"],
        sha256(&telemetry_path)
    );
    assert_eq!(queue_provenance["eventIds"], json!(["b02"]));
    assert!(report["rules"].as_array().unwrap().iter().any(|rule| {
        rule["id"] == "mandatory-verification-protection"
            && rule["methodCardIds"] == json!(["critical-path-pert", "value-stream-queue-delay"])
    }));

    let mismatch = request(inputs.clone(), &"9".repeat(64));
    let (mismatch_exit, _, _, mismatch_out) = execute(&temp.0, &mismatch, "mismatch");
    assert_eq!(mismatch_exit, 65);
    assert!(!mismatch_out.join("light-speed-report.json").exists());

    let mut unprotected = telemetry.clone();
    unprotected["current"]["events"][5]["mandatory"] = json!(false);
    let unprotected_path = temp.0.join("unprotected-timing-events.json");
    fs::write(&unprotected_path, serde_json::to_vec(&unprotected).unwrap()).unwrap();
    let mut unprotected_inputs = inputs.clone();
    let timing_input = unprotected_inputs.last_mut().unwrap();
    timing_input["path"] = json!("unprotected-timing-events.json");
    timing_input["sha256"] = json!(sha256(&unprotected_path));
    let unprotected_request = request(unprotected_inputs, SNAPSHOT);
    let (unprotected_exit, _, _, unprotected_out) =
        execute(&temp.0, &unprotected_request, "unprotected");
    assert_eq!(unprotected_exit, 65);
    assert!(!unprotected_out.join("light-speed-report.json").exists());

    let missing_commit_path = temp.0.join("commits/baseline.json");
    let missing_commit_bytes = fs::read(&missing_commit_path).unwrap();
    fs::remove_file(&missing_commit_path).unwrap();
    let (missing_exit, _, _, missing_out) = execute(&temp.0, &valid_request, "missing-commit");
    assert_eq!(missing_exit, 65);
    assert!(!missing_out.join("light-speed-report.json").exists());
    fs::write(&missing_commit_path, missing_commit_bytes).unwrap();

    let mut missing_manifest_inputs = inputs.clone();
    let manifest_index = missing_manifest_inputs
        .iter()
        .position(|input| input["artifactSchema"] == "code-intel-run-manifest.v1")
        .expect("A07 manifest Artifact Ref must be present in the valid request");
    missing_manifest_inputs.remove(manifest_index);
    let missing_manifest_request = request(missing_manifest_inputs, SNAPSHOT);
    let (missing_manifest_exit, _, _, missing_manifest_out) = execute(
        &temp.0,
        &missing_manifest_request,
        "missing-a07-manifest-object",
    );
    assert_eq!(missing_manifest_exit, 65);
    assert!(!missing_manifest_out
        .join("light-speed-report.json")
        .exists());
    assert!(!missing_manifest_out.join("light-speed-report.md").exists());

    let tampered_card = temp
        .0
        .join("methods/cards/value-stream-queue-delay.v1.json");
    let mut card: Value = serde_json::from_slice(&fs::read(&tampered_card).unwrap()).unwrap();
    card["deterministicSteps"][1]["action"] = json!("tampered but structurally valid formula");
    fs::write(&tampered_card, serde_json::to_vec(&card).unwrap()).unwrap();
    let mut tampered_inputs = inputs.clone();
    let card_ref = tampered_inputs
        .iter_mut()
        .find(|input| input["path"] == "methods/cards/value-stream-queue-delay.v1.json")
        .unwrap();
    card_ref["sha256"] = json!(sha256(&tampered_card));
    let tampered_request = request(tampered_inputs, SNAPSHOT);
    let (tampered_exit, _, _, tampered_out) = execute(&temp.0, &tampered_request, "tampered-card");
    assert_eq!(tampered_exit, 65);
    assert!(!tampered_out.join("light-speed-report.json").exists());
}

#[test]
fn critical_path_is_predecessor_closed_for_a_diamond_join() {
    let temp = Temp::new();
    let (baseline, mut inputs) = committed_trace(
        &temp.0,
        "diamond-baseline",
        &format!("dag-v1:{}", "3c".repeat(16)),
        vec![event(
            "base",
            "technical_work",
            "base",
            0,
            5,
            false,
            None,
            None,
        )],
    );
    let (current, current_inputs) = committed_trace(
        &temp.0,
        "diamond-current",
        &format!("dag-v1:{}", "4d".repeat(16)),
        vec![
            event("a", "technical_work", "root", 0, 10, false, None, None),
            event(
                "b",
                "technical_work",
                "left",
                10,
                30,
                false,
                None,
                Some("a"),
            ),
            event(
                "c",
                "technical_work",
                "right",
                10,
                25,
                false,
                None,
                Some("a"),
            ),
            event_with_predecessors(
                "d",
                "verification",
                "join",
                30,
                40,
                true,
                None,
                vec!["b", "c"],
            ),
        ],
    );
    inputs.extend(current_inputs);
    inputs.extend(method_inputs(&temp.0));
    let telemetry = json!({
        "schema":"code-intel-run-timing-events.v1",
        "measurementSnapshotIdentity":SNAPSHOT,
        "telemetry":{"mode":"local_opt_in","clock":"monotonic_elapsed_ms","externalPlatform":false},
        "baseline":baseline,
        "current":current
    });
    let timing = write_artifact(&temp.0, "diamond-timing.json", &telemetry);
    inputs.push(artifact_ref(
        "code-intel-run-timing-events.v1",
        "delivery.run-timing-events",
        timing["path"].as_str().unwrap(),
        timing["sha256"].as_str().unwrap(),
    ));
    let (exit, _, stderr, out) = execute(&temp.0, &request(inputs, SNAPSHOT), "diamond");
    assert_eq!(exit, 0, "{}", String::from_utf8_lossy(&stderr));
    let report: Value =
        serde_json::from_slice(&fs::read(out.join("light-speed-report.json")).unwrap()).unwrap();
    assert_eq!(
        report["current"]["criticalPath"]["eventIds"],
        json!(["a", "b", "c", "d"])
    );
    assert_eq!(report["current"]["criticalPath"]["durationMs"], 55);
}
