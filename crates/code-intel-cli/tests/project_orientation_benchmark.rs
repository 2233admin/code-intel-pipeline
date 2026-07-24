use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("code-intel-d02-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
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

#[test]
fn representative_corpus_measures_quality_latency_and_rejects_fast_provenance_free_output() {
    let temp = Temp::new();
    let out = temp.0.join("benchmark");
    let measured = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "benchmark",
            "orientation",
            "--out",
            out.to_str().unwrap(),
            "--repetitions",
            "2",
        ])
        .output()
        .unwrap();
    assert_eq!(
        measured.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&measured.stdout),
        String::from_utf8_lossy(&measured.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(
        report["schema"],
        "code-intel-project-orientation-benchmark.v1"
    );
    assert_eq!(report["verdict"], "pass");
    assert_eq!(report["corpus"]["fixtureCount"], 9);
    assert_eq!(
        report["corpus"]["typicalDefinition"],
        "small_and_medium_all_conditions"
    );
    assert_eq!(report["method"]["repetitionsPerTemperature"], 2);
    assert_eq!(report["method"]["llm"], "disabled");
    assert_eq!(report["quality"]["fieldCorrectness"], 1.0);
    assert_eq!(report["quality"]["unknownPrecision"], 1.0);
    assert_eq!(report["quality"]["unresolvedCoverage"], 1.0);
    assert_eq!(report["quality"]["unsupportedCoverage"], 1.0);
    assert_eq!(report["quality"]["deterministicReplayRate"], 1.0);
    assert_eq!(report["quality"]["provenanceCompleteness"], 1.0);
    assert!(report["artifactSize"]["typical"]["p50Bytes"]
        .as_u64()
        .is_some_and(|bytes| bytes > 0));
    assert!(report["artifactSize"]["all"]["maxBytes"]
        .as_u64()
        .is_some_and(|bytes| bytes > 0));
    assert!(report["latency"]["typical"]["p50WallTimeMs"].is_u64());
    assert!(
        report["latency"]["typical"]["p95WallTimeMs"]
            .as_u64()
            .unwrap()
            <= 60_000
    );
    assert!(report["costCenters"].as_array().unwrap().len() >= 2);
    assert_eq!(report["environment"]["cleanMachine"], false);
    assert!(report["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("clean machine")));

    let observations_path = out.join("observations.json");
    let observations: Value =
        serde_json::from_slice(&fs::read(&observations_path).unwrap()).unwrap();
    for size in ["small", "medium", "large"] {
        let by_condition = |condition: &str| {
            observations["fixtures"]
                .as_array()
                .unwrap()
                .iter()
                .find(|fixture| fixture["size"] == size && fixture["condition"] == condition)
                .unwrap()
        };
        let provider = |fixture: &Value| {
            fixture["samples"]["warm"][0]["orientation"]["evidenceAvailability"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["evidence"] == "benchmark_provider")
                .cloned()
                .unwrap()
        };
        let clean = by_condition("clean");
        let dirty = by_condition("dirty");
        let missing = by_condition("provider_missing");
        for fixture in [clean, dirty, missing] {
            assert_eq!(
                fixture["samples"]["warm"][0]["coverage"]["unsupportedFiles"],
                json!(["Cargo.toml", "README.md"])
            );
            assert!(fixture["samples"]["warm"][0]["artifact"]["bytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0));
            assert_eq!(
                fixture["samples"]["warm"][0]["artifact"]["sha256"],
                fixture["samples"]["warm"][1]["artifact"]["sha256"]
            );
        }
        assert_eq!(provider(clean)["status"], "available");
        assert_eq!(provider(dirty)["status"], "available");
        assert_eq!(provider(missing)["status"], "unavailable");
        assert_ne!(
            provider(clean)["provenance"][0]["artifactSha256"],
            provider(missing)["provenance"][0]["artifactSha256"],
            "provider condition must alter committed evidence for {size}"
        );
        assert!(!clean["samples"]["warm"][0]["orientation"]["risks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|risk| risk["code"] == "structural_evidence_unavailable"));
        assert!(missing["samples"]["warm"][0]["orientation"]["risks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|risk| risk["code"] == "structural_evidence_unavailable"));
    }
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
        .find(|entry| entry["id"] == "project.orientation-benchmark")
        .unwrap();
    let snapshot = observations["snapshotIdentity"]
        .as_str()
        .unwrap()
        .to_string();
    let request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"project.orientation-benchmark",
        "contractVersion":1,
        "implementation":declaration["capabilityDeclaration"]["implementation"],
        "snapshot":{
            "identity":&snapshot,
            "repoIdentity":format!("content-v1:{}", "c".repeat(64)),
            "head":"benchmark-corpus-v1",
            "workingTreePolicy":"explicit_overlay",
            "scope":["."],
            "inputDigest":"d".repeat(64)
        },
        "options":{},
        "inputs":[{
            "schema":"code-intel-artifact-ref.v1",
            "artifactSchema":"code-intel-project-orientation-benchmark-observations.v1",
            "type":"benchmark.orientation-observations",
            "path":"benchmark/observations.json",
            "sha256":sha256(&observations_path),
            "consumedSnapshotIdentity":&snapshot
        }],
        "effectPolicy":{"allowedEffects":["local_write"]}
    });
    let valid_request_path = temp.0.join("valid-request.json");
    fs::write(&valid_request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let valid_out = temp.0.join("a01-valid");
    let valid = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "project.orientation-benchmark",
            "--request",
        ])
        .arg(&valid_request_path)
        .arg("--out")
        .arg(&valid_out)
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(
        valid.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&valid.stdout),
        String::from_utf8_lossy(&valid.stderr)
    );
    assert!(valid_out.join("report.json").exists());
    assert!(valid_out.join("report.md").exists());

    let mut wrong_count_observations = observations.clone();
    let expected_count = wrong_count_observations["fixtures"][0]["expected"]["fileCount"]
        .as_u64()
        .unwrap();
    wrong_count_observations["fixtures"][0]["expected"]["fileCount"] = json!(expected_count + 1);
    let wrong_count_path = temp.0.join("wrong-count-observations.json");
    fs::write(
        &wrong_count_path,
        serde_json::to_vec(&wrong_count_observations).unwrap(),
    )
    .unwrap();
    let mut wrong_count_request = request.clone();
    wrong_count_request["inputs"][0]["path"] = json!("wrong-count-observations.json");
    wrong_count_request["inputs"][0]["sha256"] = json!(sha256(&wrong_count_path));
    let wrong_count_request_path = temp.0.join("wrong-count-request.json");
    fs::write(
        &wrong_count_request_path,
        serde_json::to_vec(&wrong_count_request).unwrap(),
    )
    .unwrap();
    let wrong_count_out = temp.0.join("wrong-count-out");
    let wrong_count = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "project.orientation-benchmark",
            "--request",
        ])
        .arg(&wrong_count_request_path)
        .arg("--out")
        .arg(&wrong_count_out)
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(wrong_count.status.code(), Some(0));
    let wrong_count_report: Value =
        serde_json::from_slice(&fs::read(wrong_count_out.join("report.json")).unwrap()).unwrap();
    assert_eq!(wrong_count_report["verdict"], "fail");
    assert!(
        wrong_count_report["quality"]["fieldCorrectness"]
            .as_f64()
            .unwrap()
            < 1.0
    );

    let mut forged_observations = observations;
    forged_observations["fixtures"][0]["samples"]["warm"][0]["orientation"]["identity"]
        ["provenance"] = json!([]);
    let forged_path = temp.0.join("forged-observations.json");
    fs::write(
        &forged_path,
        serde_json::to_vec(&forged_observations).unwrap(),
    )
    .unwrap();
    let mut forged_request = request;
    forged_request["inputs"][0]["path"] = json!("forged-observations.json");
    forged_request["inputs"][0]["sha256"] = json!(sha256(&forged_path));
    let request_path = temp.0.join("forged-request.json");
    fs::write(&request_path, serde_json::to_vec(&forged_request).unwrap()).unwrap();
    let rejected_out = temp.0.join("rejected");
    let rejected = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "project.orientation-benchmark",
            "--request",
        ])
        .arg(&request_path)
        .arg("--out")
        .arg(&rejected_out)
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(65));
    assert!(!rejected_out.join("report.json").exists());
}

#[test]
fn historical_clean_machine_attestation_is_closed_and_cannot_claim_current_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let evidence_path =
        root.join("orchestration/evidence/d02-clean-machine-verifier-attestation.v1.json");
    let schema_path = root
        .join("orchestration/schemas/code-intel-clean-machine-verifier-attestation.v1.schema.json");
    let powershell = if cfg!(windows) { "powershell" } else { "pwsh" };
    let output = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "param($Document,$Schema); if (-not (Get-Content -Raw -LiteralPath $Document | Test-Json -SchemaFile $Schema -ErrorAction Stop)) { exit 1 }",
        ])
        .arg(&evidence_path)
        .arg(&schema_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let evidence: Value = serde_json::from_slice(&fs::read(&evidence_path).unwrap()).unwrap();
    assert_eq!(evidence["environment"]["cleanMachine"], true);
    assert_eq!(evidence["environment"]["mountCount"], 0);
    assert_eq!(evidence["benchmark"]["fixtureCount"], 9);
    assert_eq!(evidence["benchmark"]["sampleCount"], 54);
    assert_eq!(evidence["productionReport"]["cleanMachine"], false);
    assert_eq!(
        evidence["productionReport"]["externalVerificationComplete"],
        false
    );
    assert_ne!(
        evidence["source"]["sha256"],
        sha256(&root.join("crates/code-intel-cli/src/project_orientation_benchmark.rs"))
    );
    assert!(evidence["productionReport"]["statement"]
        .as_str()
        .unwrap()
        .contains("requires a fresh independent clean-machine rerun"));
}
