mod common;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
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
            "code-intel-flash-{}-{nonce}-{sequence}",
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

fn attempts(values: &[f64]) -> Vec<Value> {
    values
        .iter()
        .map(|value| json!({"ok": true, "value": value}))
        .collect()
}

fn samples(primary: Vec<Value>, paired: Option<(Vec<Value>, Vec<&str>)>) -> Value {
    let mut body = json!({
        "schema": "code-intel-flash-samples.v1",
        "operation": "cargo-test",
        "metric": "duration-ms",
        "primary": primary,
    });
    if let Some((attempts, order)) = paired {
        body["paired"] = json!({"attempts": attempts, "order": order});
    }
    body
}

fn write_input(root: &Path, name: &str, body: &Value, schema: &str, artifact_type: &str) -> Value {
    let relative = format!("{name}.json");
    let path = root.join(&relative);
    fs::write(&path, serde_json::to_vec(body).unwrap()).unwrap();
    json!({
        "schema": "code-intel-artifact-ref.v1",
        "artifactSchema": schema,
        "type": artifact_type,
        "path": relative,
        "sha256": sha256_hex(&fs::read(&path).unwrap()),
        "consumedSnapshotIdentity": SNAPSHOT,
    })
}

fn request(root: &Path, inputs: Vec<Value>) -> Value {
    let registry: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/integrations.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let implementation = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "measurement.flash-ratchet")
        .unwrap()["capabilityDeclaration"]["implementation"]
        .clone();
    json!({
        "schema": "code-intel-capability-request.v1",
        "capability": "measurement.flash-ratchet",
        "contractVersion": 1,
        "implementation": implementation,
        "snapshot": {
            "identity": SNAPSHOT,
            "repoIdentity": format!("content-v1:{}", "c".repeat(64)),
            "head": "flash",
            "workingTreePolicy": "explicit_overlay",
            "scope": ["."],
            "inputDigest": "d".repeat(64)
        },
        "options": {},
        "inputs": inputs,
        "effectPolicy": {"allowedEffects": ["local_write"]}
    })
}

fn execute(root: &Path, request: &Value) -> Value {
    let request_path = root.join("request.json");
    fs::write(&request_path, serde_json::to_vec(request).unwrap()).unwrap();
    let out = root.join("out");
    let output = common::cli()
        .args([
            "capability",
            "exec",
            "measurement.flash-ratchet",
            "--request",
        ])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--artifact-root")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&fs::read(out.join("flash-ratchet.json")).unwrap()).unwrap()
}

fn ten(start: f64) -> Vec<f64> {
    (0..10).map(|index| start + index as f64).collect()
}

#[test]
fn init_records_the_primary_p75_and_excludes_failed_attempts() {
    let temp = Temp::new();
    let mut attempts = attempts(&ten(100.0));
    attempts.push(json!({"ok": false, "reason": "timed out"}));
    let input = write_input(
        &temp.0,
        "samples",
        &samples(attempts, None),
        "code-intel-flash-samples.v1",
        "measurement.flash-samples",
    );
    let report = execute(&temp.0, &request(&temp.0, vec![input]));
    assert_eq!(report["ratchet"]["state"], "initialized");
    assert_eq!(report["primary"]["samples"], 10);
    assert_eq!(report["primary"]["failed"], 1);
    assert_eq!(
        report["ratchet"]["ceiling"]["p75"],
        report["primary"]["p75"]
    );
    assert_eq!(report["ratchet"]["tolerance"], 0.05);
    assert!(report["paired"].is_null());
}

#[test]
fn a_p75_inside_tolerance_holds_the_submitted_ceiling() {
    let temp = Temp::new();
    let samples = write_input(
        &temp.0,
        "samples",
        &samples(attempts(&ten(100.0)), None),
        "code-intel-flash-samples.v1",
        "measurement.flash-samples",
    );
    let ceiling = write_input(
        &temp.0,
        "ceiling",
        &json!({
            "schema": "code-intel-flash-ratchet-ceiling.v1", "operation": "cargo-test",
            "metric": "duration-ms", "direction": "lower", "tolerance": 0.05, "p75": 106.0
        }),
        "code-intel-flash-ratchet-ceiling.v1",
        "measurement.flash-ratchet-ceiling",
    );
    let report = execute(&temp.0, &request(&temp.0, vec![samples, ceiling]));
    assert_eq!(report["ratchet"]["state"], "held");
    assert_eq!(report["ratchet"]["ceiling"]["p75"], 106.0);
}

#[test]
fn a_lower_p75_tightens_the_ceiling_without_rewriting_the_submitted_record() {
    let temp = Temp::new();
    let samples = write_input(
        &temp.0,
        "samples",
        &samples(attempts(&ten(100.0)), None),
        "code-intel-flash-samples.v1",
        "measurement.flash-samples",
    );
    let ceiling_body = json!({
        "schema": "code-intel-flash-ratchet-ceiling.v1", "operation": "cargo-test",
        "metric": "duration-ms", "direction": "lower", "tolerance": 0.05, "p75": 200.0
    });
    let ceiling = write_input(
        &temp.0,
        "ceiling",
        &ceiling_body,
        "code-intel-flash-ratchet-ceiling.v1",
        "measurement.flash-ratchet-ceiling",
    );
    let report = execute(&temp.0, &request(&temp.0, vec![samples, ceiling]));
    assert_eq!(report["ratchet"]["state"], "tightened");
    assert!(report["ratchet"]["ceiling"]["p75"].as_f64().unwrap() < 200.0);
    let reread: Value =
        serde_json::from_slice(&fs::read(temp.0.join("ceiling.json")).unwrap()).unwrap();
    assert_eq!(reread, ceiling_body);
}

#[test]
fn a_p75_beyond_tolerance_is_a_domain_failure() {
    let temp = Temp::new();
    let samples = write_input(
        &temp.0,
        "samples",
        &samples(attempts(&ten(300.0)), None),
        "code-intel-flash-samples.v1",
        "measurement.flash-samples",
    );
    let ceiling = write_input(
        &temp.0,
        "ceiling",
        &json!({
            "schema": "code-intel-flash-ratchet-ceiling.v1", "operation": "cargo-test",
            "metric": "duration-ms", "direction": "lower", "tolerance": 0.05, "p75": 100.0
        }),
        "code-intel-flash-ratchet-ceiling.v1",
        "measurement.flash-ratchet-ceiling",
    );
    let request_path = temp.0.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&temp.0, vec![samples, ceiling])).unwrap(),
    )
    .unwrap();
    let output = common::cli()
        .args([
            "capability",
            "exec",
            "measurement.flash-ratchet",
            "--request",
        ])
        .arg(&request_path)
        .arg("--out")
        .arg(temp.0.join("out"))
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["domainVerdict"], "fail");
    let report: Value =
        serde_json::from_slice(&fs::read(temp.0.join("out").join("flash-ratchet.json")).unwrap())
            .unwrap();
    assert_eq!(report["ratchet"]["state"], "regressed");
    assert_eq!(report["ratchet"]["ceiling"]["p75"], 100.0);
}

#[test]
fn a_non_alternating_pair_is_rejected() {
    let temp = Temp::new();
    let order = vec![
        "A", "A", "B", "B", "A", "B", "A", "B", "A", "B", "A", "B", "A", "B", "A", "B", "A", "B",
        "A", "B",
    ];
    let body = samples(attempts(&ten(100.0)), Some((attempts(&ten(80.0)), order)));
    let input = write_input(
        &temp.0,
        "samples",
        &body,
        "code-intel-flash-samples.v1",
        "measurement.flash-samples",
    );
    let request_path = temp.0.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&temp.0, vec![input])).unwrap(),
    )
    .unwrap();
    let output = common::cli()
        .args([
            "capability",
            "exec",
            "measurement.flash-ratchet",
            "--request",
        ])
        .arg(&request_path)
        .arg("--out")
        .arg(temp.0.join("out"))
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a non-alternating order must not produce a reduction"
    );
    assert!(!temp.0.join("out").join("flash-ratchet.json").exists());
}

#[test]
fn an_alternating_pair_reports_the_fractional_p75_drop() {
    let temp = Temp::new();
    let order: Vec<&str> = (0..20)
        .map(|index| if index % 2 == 0 { "A" } else { "B" })
        .collect();
    let body = samples(attempts(&ten(200.0)), Some((attempts(&ten(100.0)), order)));
    let input = write_input(
        &temp.0,
        "samples",
        &body,
        "code-intel-flash-samples.v1",
        "measurement.flash-samples",
    );
    let report = execute(&temp.0, &request(&temp.0, vec![input]));
    assert!(report["paired"]["p75Reduction"].as_f64().unwrap() > 0.0);
}
