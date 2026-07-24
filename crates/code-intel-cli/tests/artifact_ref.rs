use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const IMPLEMENTATION_DIGEST: &str =
    "43ced9ef578e6484423468e059c93ef0bc5eeeb35d23271451b2d8f1a16f9bb6";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-a03-{label}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(repo: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "snapshot",
            "identity",
            "--repo",
            repo.to_str().unwrap(),
            "--working-tree-policy",
            "explicit_overlay",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["snapshot"].clone()
}

fn artifact_ref(path: &str, bytes: &[u8], snapshot_identity: &Value) -> Value {
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-file-inventory.v1",
        "type":"inventory.files",
        "path":path,
        "sha256":sha256(bytes),
        "consumedSnapshotIdentity":snapshot_identity
    })
}

fn sha256(bytes: &[u8]) -> String {
    assert_eq!(bytes, b"portable evidence\n");
    "924278019c18519b69088648b6d5b4f58fc96afa66204bab1274a5a4ee2bd2c2".to_string()
}

fn request(repo: &Path, input: Value) -> Value {
    let snapshot = snapshot(repo);
    json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"inventory.rg",
        "contractVersion":1,
        "implementation":{
            "id":"inventory.rg.compat",
            "version":"1.0.0",
            "toolchainDigests":[IMPLEMENTATION_DIGEST]
        },
        "snapshot":snapshot,
        "options":{"repoPath":repo},
        "inputs":[input],
        "effectPolicy":{"allowedEffects":["repo_read","local_write"]}
    })
}

fn request_with_inputs(repo: &Path, inputs: Vec<Value>) -> Value {
    let mut value = request(repo, inputs[0].clone());
    value["inputs"] = Value::Array(inputs);
    value
}

fn run(
    root: &Path,
    request: &Value,
    artifact_root: Option<&Path>,
    out: &Path,
) -> std::process::Output {
    let request_path = root.join(format!(
        "request-{}.json",
        out.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&request_path, serde_json::to_vec(request).unwrap()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    command
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(out);
    if let Some(artifact_root) = artifact_root {
        command.arg("--artifact-root").arg(artifact_root);
    }
    command.output().unwrap()
}

fn fixture() -> (TempTree, PathBuf, Value, Vec<u8>) {
    let root = TempTree::new("fixture");
    let repo = root.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("source.rs"), "fn main() {}\n").unwrap();
    let snapshot = snapshot(&repo);
    let bytes = b"portable evidence\n".to_vec();
    (root, repo, snapshot, bytes)
}

#[test]
fn artifact_ref_is_content_identified_and_relocation_safe() {
    let (root, repo, snapshot, bytes) = fixture();
    let first = root.0.join("artifacts-one");
    let second = root.0.join("artifacts-two");
    fs::create_dir_all(first.join("nested")).unwrap();
    fs::create_dir_all(second.join("nested")).unwrap();
    fs::write(first.join("nested/payload.bin"), &bytes).unwrap();
    fs::write(second.join("nested/payload.bin"), &bytes).unwrap();
    let reference = artifact_ref("nested/payload.bin", &bytes, &snapshot["identity"]);
    let request = request(&repo, reference);

    for (index, artifact_root) in [&first, &second].into_iter().enumerate() {
        let out = root.0.join(format!("out-{index}"));
        let output = run(&root.0, &request, Some(artifact_root), &out);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(out.join("files.txt").is_file());
    }
}

#[test]
fn artifact_ref_fails_closed_for_missing_tamper_snapshot_and_root_omission() {
    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    let reference = artifact_ref("payload.bin", &bytes, &snapshot["identity"]);

    let missing = run(
        &root.0,
        &request(&repo, reference.clone()),
        Some(&artifacts),
        &root.0.join("out-missing"),
    );
    assert_eq!(missing.status.code(), Some(65));

    fs::write(artifacts.join("payload.bin"), b"tampered\n").unwrap();
    let tampered = run(
        &root.0,
        &request(&repo, reference.clone()),
        Some(&artifacts),
        &root.0.join("out-tampered"),
    );
    assert_eq!(tampered.status.code(), Some(65));

    fs::write(artifacts.join("payload.bin"), &bytes).unwrap();
    let mut wrong_snapshot = reference.clone();
    wrong_snapshot["consumedSnapshotIdentity"] = json!("f".repeat(64));
    let wrong = run(
        &root.0,
        &request(&repo, wrong_snapshot),
        Some(&artifacts),
        &root.0.join("out-wrong"),
    );
    assert_eq!(wrong.status.code(), Some(65));

    let no_root = run(
        &root.0,
        &request(&repo, reference),
        None,
        &root.0.join("out-no-root"),
    );
    assert_eq!(no_root.status.code(), Some(65));
}

#[cfg(windows)]
#[test]
fn artifact_ref_maps_a_deny_share_lock_to_host_io_exit_74() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    let payload = artifacts.join("payload.bin");
    fs::write(&payload, &bytes).unwrap();
    let reference = artifact_ref("payload.bin", &bytes, &snapshot["identity"]);
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&payload)
        .unwrap();

    let output = run(
        &root.0,
        &request(&repo, reference),
        Some(&artifacts),
        &root.0.join("out-locked"),
    );
    assert_eq!(
        output.status.code(),
        Some(74),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    drop(lock);
}

#[test]
fn artifact_ref_preflights_all_collisions_before_missing_or_tampered_files() {
    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();

    for (label, first_path, second_path) in [
        ("exact", "dup.bin", "dup.bin"),
        ("casefold", "Ä.bin", "ä.bin"),
        ("nfc", "é.bin", "e\u{301}.bin"),
    ] {
        let request = request_with_inputs(
            &repo,
            vec![
                artifact_ref(first_path, &bytes, &snapshot["identity"]),
                artifact_ref(second_path, &bytes, &snapshot["identity"]),
            ],
        );
        let output = run(
            &root.0,
            &request,
            Some(&artifacts),
            &root.0.join(format!("out-preflight-{label}")),
        );
        assert_eq!(
            output.status.code(),
            Some(65),
            "{label}: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let first = artifacts.join("Ä.bin");
    fs::write(&first, &bytes).unwrap();
    let collision = request_with_inputs(
        &repo,
        vec![
            artifact_ref("Ä.bin", &bytes, &snapshot["identity"]),
            artifact_ref("ä.bin", &bytes, &snapshot["identity"]),
        ],
    );
    let output = run(
        &root.0,
        &collision,
        Some(&artifacts),
        &root.0.join("out-preflight-present"),
    );
    assert_eq!(output.status.code(), Some(65));

    fs::write(&first, b"tampered\n").unwrap();
    let output = run(
        &root.0,
        &collision,
        Some(&artifacts),
        &root.0.join("out-preflight-tampered"),
    );
    assert_eq!(output.status.code(), Some(65));
}

#[cfg(windows)]
#[test]
fn artifact_ref_preflights_casefold_collision_before_a_first_file_lock() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    let first = artifacts.join("Ä.bin");
    fs::write(&first, &bytes).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&first)
        .unwrap();
    let collision = request_with_inputs(
        &repo,
        vec![
            artifact_ref("Ä.bin", &bytes, &snapshot["identity"]),
            artifact_ref("ä.bin", &bytes, &snapshot["identity"]),
        ],
    );
    let output = run(
        &root.0,
        &collision,
        Some(&artifacts),
        &root.0.join("out-preflight-locked"),
    );
    assert_eq!(
        output.status.code(),
        Some(65),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    drop(lock);
}

#[test]
fn artifact_ref_rejects_absolute_parent_directory_and_link_payloads() {
    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    fs::write(artifacts.join("payload.bin"), &bytes).unwrap();

    for (index, path) in [
        "../payload.bin",
        artifacts.join("payload.bin").to_str().unwrap(),
        ".",
    ]
    .into_iter()
    .enumerate()
    {
        let reference = artifact_ref(path, &bytes, &snapshot["identity"]);
        let output = run(
            &root.0,
            &request(&repo, reference),
            Some(&artifacts),
            &root.0.join(format!("out-path-{index}")),
        );
        assert_eq!(
            output.status.code(),
            Some(65),
            "path={path} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let outside = root.0.join("outside.bin");
    fs::write(&outside, &bytes).unwrap();
    let link = artifacts.join("link.bin");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, &link).is_ok();
    if linked {
        let reference = artifact_ref("link.bin", &bytes, &snapshot["identity"]);
        let output = run(
            &root.0,
            &request(&repo, reference),
            Some(&artifacts),
            &root.0.join("out-link"),
        );
        assert_eq!(output.status.code(), Some(65));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn artifact_ref_maps_root_intermediate_and_leaf_links_to_contract_exit_65() {
    use std::os::unix::fs::symlink;
    let (root, repo, snapshot, bytes) = fixture();
    let real = root.0.join("artifacts-real");
    let nested = real.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("payload.bin"), &bytes).unwrap();
    let root_link = root.0.join("artifacts-root-link");
    symlink(&real, &root_link).unwrap();
    symlink(&nested, real.join("intermediate-link")).unwrap();
    symlink(nested.join("payload.bin"), real.join("leaf-link.bin")).unwrap();

    for (label, artifact_root, path) in [
        ("root", root_link.as_path(), "nested/payload.bin"),
        (
            "intermediate",
            real.as_path(),
            "intermediate-link/payload.bin",
        ),
        ("leaf", real.as_path(), "leaf-link.bin"),
    ] {
        let reference = artifact_ref(path, &bytes, &snapshot["identity"]);
        let output = run(
            &root.0,
            &request(&repo, reference),
            Some(artifact_root),
            &root.0.join(format!("out-linux-{label}")),
        );
        assert_eq!(
            output.status.code(),
            Some(65),
            "{label}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn artifact_ref_rejects_unregistered_contract_invalid_payload_and_duplicate_aliases() {
    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    fs::write(artifacts.join("payload.bin"), &bytes).unwrap();

    let mut unknown = artifact_ref("payload.bin", &bytes, &snapshot["identity"]);
    unknown["artifactSchema"] = json!("unknown.v1");
    let output = run(
        &root.0,
        &request(&repo, unknown),
        Some(&artifacts),
        &root.0.join("out-contract"),
    );
    assert_eq!(output.status.code(), Some(65));

    let invalid_bytes = b"z.rs\na.rs\n";
    fs::write(artifacts.join("invalid.bin"), invalid_bytes).unwrap();
    let invalid = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-file-inventory.v1",
        "type":"inventory.files",
        "path":"invalid.bin",
        "sha256":"e2e00e70528e4f58b153ed72c56e13fae6943bdde078ba7cec457cb425b0d8a4",
        "consumedSnapshotIdentity":snapshot["identity"]
    });
    let output = run(
        &root.0,
        &request(&repo, invalid),
        Some(&artifacts),
        &root.0.join("out-payload"),
    );
    assert_eq!(output.status.code(), Some(65));

    fs::write(artifacts.join("Payload.bin"), &bytes).unwrap();
    let first = artifact_ref("payload.bin", &bytes, &snapshot["identity"]);
    let second = artifact_ref("Payload.bin", &bytes, &snapshot["identity"]);
    let mut duplicate_request = request(&repo, first);
    duplicate_request["inputs"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let output = run(
        &root.0,
        &duplicate_request,
        Some(&artifacts),
        &root.0.join("out-duplicate"),
    );
    assert_eq!(output.status.code(), Some(65));
}

#[test]
fn artifact_ref_rejects_payload_over_the_registered_size_limit_before_reading() {
    let (root, repo, snapshot, _) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    let oversized = artifacts.join("large.bin");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let reference = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-file-inventory.v1",
        "type":"inventory.files",
        "path":"large.bin",
        "sha256":"0".repeat(64),
        "consumedSnapshotIdentity":snapshot["identity"]
    });
    let output = run(
        &root.0,
        &request(&repo, reference),
        Some(&artifacts),
        &root.0.join("out-large"),
    );
    assert_eq!(output.status.code(), Some(65));
}

#[test]
fn artifact_ref_rejects_hardlink_aliases_and_duplicate_root_authority() {
    let (root, repo, snapshot, bytes) = fixture();
    let artifacts = root.0.join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    fs::write(artifacts.join("one.bin"), &bytes).unwrap();
    fs::hard_link(artifacts.join("one.bin"), artifacts.join("two.bin")).unwrap();
    let first = artifact_ref("one.bin", &bytes, &snapshot["identity"]);
    let second = artifact_ref("two.bin", &bytes, &snapshot["identity"]);
    let mut request_value = request(&repo, first);
    request_value["inputs"].as_array_mut().unwrap().push(second);
    let output = run(
        &root.0,
        &request_value,
        Some(&artifacts),
        &root.0.join("out-alias"),
    );
    assert_eq!(output.status.code(), Some(65));

    let request_path = root.0.join("duplicate-root-request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(
            &repo,
            artifact_ref("one.bin", &bytes, &snapshot["identity"]),
        ))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(root.0.join("out-duplicate-root"))
        .arg("--artifact-root")
        .arg(&artifacts)
        .arg("--artifact-root")
        .arg(&artifacts)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
}
