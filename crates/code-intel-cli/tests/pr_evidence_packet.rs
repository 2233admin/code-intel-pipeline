mod common;
mod pr_evidence_packet_cases;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-pr-evidence-{}-{stamp}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
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

fn fixture(name: &str) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pr-evidence");
    serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap()
}

fn run_request(temp: &Path, stem: &str, request: &Value) -> (std::process::Output, PathBuf) {
    let request_path = temp.join(format!("{stem}.request.json"));
    let output_path = temp.join(format!("{stem}.packet.json"));
    fs::write(&request_path, serde_json::to_vec_pretty(request).unwrap()).unwrap();
    let output = common::cli()
        .args(["pr", "evidence", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&output_path)
        .output()
        .unwrap();
    (output, output_path)
}

fn assert_success(output: &std::process::Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
