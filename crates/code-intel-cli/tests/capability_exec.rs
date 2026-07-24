use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const IMPLEMENTATION_DIGEST: &str =
    "43ced9ef578e6484423468e059c93ef0bc5eeeb35d23271451b2d8f1a16f9bb6";
static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let sequence = TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "code-intel-a01-{name}-{}-{nonce}-{sequence}",
        std::process::id(),
    ))
}

fn request(repo: &Path, capability: &str) -> Value {
    request_with_policy_scopes(repo, capability, "explicit_overlay", &["."])
}

fn request_with_policy_scopes(
    repo: &Path,
    capability: &str,
    policy: &str,
    scopes: &[&str],
) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    command
        .args(["snapshot", "identity", "--repo"])
        .arg(repo)
        .args(["--working-tree-policy", policy]);
    for scope in scopes {
        command.args(["--scope", scope]);
    }
    let snapshot_output = command.output().expect("compute A02 request snapshot");
    assert!(
        snapshot_output.status.success(),
        "snapshot stderr={}",
        String::from_utf8_lossy(&snapshot_output.stderr)
    );
    let snapshot_document: Value =
        serde_json::from_slice(&snapshot_output.stdout).expect("snapshot JSON");
    json!({
        "schema": "code-intel-capability-request.v1",
        "capability": capability,
        "contractVersion": 1,
        "implementation": {
            "id": "inventory.rg.compat",
            "version": "1.0.0",
            "toolchainDigests": [IMPLEMENTATION_DIGEST]
        },
        "snapshot": snapshot_document["snapshot"],
        "options": {
            "repoPath": repo
        },
        "inputs": [],
        "effectPolicy": { "allowedEffects": ["repo_read", "local_write"] }
    })
}

fn run_with_request_file(
    request: &Value,
    request_path: &Path,
    out: &Path,
    cli_capability: &str,
) -> std::process::Output {
    fs::write(
        request_path,
        serde_json::to_vec(request).expect("serialize request"),
    )
    .expect("write request");
    Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", cli_capability, "--request"])
        .arg(request_path)
        .arg("--out")
        .arg(out)
        .output()
        .expect("run capability executor")
}

fn base_command(out: &Path, cli_capability: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    command
        .args(["capability", "exec", cli_capability, "--request"])
        .arg("-")
        .arg("--out")
        .arg(out);
    command
}

fn request_with_scopes(repo: &Path, capability: &str, scopes: &[&str]) -> Value {
    request_with_policy_scopes(repo, capability, "explicit_overlay", scopes)
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git fixture output is UTF-8")
        .trim()
        .to_string()
}

fn create_file_symlink(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

fn legacy_inventory(repo: &Path) -> Vec<u8> {
    let excludes = [
        "!**/.git/**",
        "!**/node_modules/**",
        "!**/.repowise/**",
        "!**/.understand-anything/**",
        "!**/.sentrux/**",
        "!**/target/**",
        "!**/dist/**",
        "!**/build/**",
        "!**/.venv/**",
        "!**/__pycache__/**",
    ];
    let mut command = Command::new(if cfg!(windows) { "rg.exe" } else { "rg" });
    command.args(["--files", "--hidden"]);
    for pattern in excludes {
        command.args(["-g", pattern]);
    }
    let output = command
        .arg(".")
        .current_dir(repo)
        .output()
        .expect("run legacy rg inventory");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("legacy rg UTF-8");
    let mut files = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let normalized = line.replace('\\', "/");
            normalized
                .strip_prefix("./")
                .unwrap_or(&normalized)
                .to_string()
        })
        .collect::<Vec<_>>();
    files.sort_unstable();
    files.dedup();
    let normalized = files.join("\n");
    if normalized.is_empty() {
        Vec::new()
    } else {
        format!("{normalized}\n").into_bytes()
    }
}

#[test]
fn inventory_rg_exec_emits_one_result_and_stable_real_rg_artifact() {
    let root = temp_dir("success");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).expect("create fixture repo");
    fs::create_dir_all(repo.join("target")).expect("create excluded target");
    fs::create_dir_all(repo.join("node_modules")).expect("create excluded dependency");
    fs::write(repo.join("README.md"), "fixture\n").expect("write README");
    fs::write(repo.join("src").join("lib.rs"), "pub fn fixture() {}\n").expect("write source");
    fs::write(repo.join(".hidden"), "hidden\n").expect("write hidden file");
    fs::write(repo.join("space & quote' 文.rs"), "unicode\n").expect("write special path");
    fs::write(repo.join("target").join("ignored.rs"), "ignored\n").expect("write excluded target");
    fs::write(repo.join("node_modules").join("ignored.js"), "ignored\n")
        .expect("write excluded dependency");
    let out = root.join("out");
    let artifact = out.join("files.txt");
    let request_path = root.join("request.json");

    let request_value = request(&repo, "inventory.rg");
    let expected_snapshot = request_value["snapshot"]["identity"].clone();
    let output = run_with_request_file(&request_value, &request_path, &out, "inventory.rg");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let mut stream = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let result = stream
        .next()
        .expect("one result")
        .expect("valid result JSON");
    assert!(
        stream.next().is_none(),
        "stdout must contain exactly one JSON document"
    );
    assert_eq!(result["schema"], "code-intel-capability-result.v1");
    assert_eq!(result["capability"], "inventory.rg");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["verdict"], "pass");
    assert_eq!(result["exitCode"], 0);
    assert_eq!(
        result["declaredEffects"],
        json!(["repo_read", "local_write"])
    );
    assert_eq!(
        result["observedEffects"],
        json!(["repo_read", "local_write"])
    );
    assert_eq!(
        result["artifacts"].as_array().expect("artifact refs").len(),
        1
    );
    assert_eq!(
        result["artifacts"][0]["consumedSnapshotIdentity"],
        expected_snapshot
    );
    assert_eq!(result["artifacts"][0]["path"], "files.txt");
    assert!(result["artifacts"][0]["sha256"]
        .as_str()
        .is_some_and(|value| value.len() == 64));
    assert!(
        output.stderr.is_empty(),
        "success diagnostics belong in result, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let files = fs::read(&artifact).expect("inventory artifact");
    assert_eq!(
        files,
        legacy_inventory(&repo),
        "new executor must preserve legacy rg inventory bytes"
    );
    let files_text = String::from_utf8(files).expect("inventory UTF-8");
    assert!(files_text.contains(".hidden"));
    assert!(files_text.contains("space & quote' 文.rs"));
    assert!(!files_text.contains("target"));
    assert!(!files_text.contains("node_modules"));

    let second_out = root.join("out-2");
    let second_artifact = second_out.join("files.txt");
    let second = run_with_request_file(
        &request(&repo, "inventory.rg"),
        &root.join("request-2.json"),
        &second_out,
        "inventory.rg",
    );
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(
        fs::read(&artifact).unwrap(),
        fs::read(&second_artifact).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn inventory_rg_ignores_repository_ignored_workspace_churn() {
    let root = temp_dir("ignored-workspace-churn");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("work").join("nested")).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Inventory Test"]);
    git(
        &repo,
        &["config", "user.email", "inventory@example.invalid"],
    );
    fs::write(repo.join(".gitignore"), "work/\n").unwrap();
    fs::write(repo.join("kept.txt"), "kept\n").unwrap();
    git(&repo, &["add", ".gitignore", "kept.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "baseline"]);
    for index in 0..128 {
        fs::write(
            repo.join("work")
                .join("nested")
                .join(format!("generated-{index}.o")),
            "generated\n",
        )
        .unwrap();
    }

    let out = root.join("out");
    let output = run_with_request_file(
        &request(&repo, "inventory.rg"),
        &root.join("request.json"),
        &out,
        "inventory.rg",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = fs::read_to_string(out.join("files.txt")).unwrap();
    assert!(files.lines().any(|path| path == ".gitignore"));
    assert!(files.lines().any(|path| path == "kept.txt"));
    assert!(!files.contains("generated-"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn inventory_rejects_repository_changes_after_snapshot_and_honors_snapshot_scope() {
    let root = temp_dir("snapshot-lease");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("docs")).unwrap();
    fs::write(repo.join("src/lib.rs"), "one\n").unwrap();
    fs::write(repo.join("docs/readme.md"), "docs\n").unwrap();

    let stale = request_with_scopes(&repo, "inventory.rg", &["src"]);
    fs::write(repo.join("src/lib.rs"), "two\n").unwrap();
    let stale_out = root.join("stale-out");
    let rejected = run_with_request_file(
        &stale,
        &root.join("stale-request.json"),
        &stale_out,
        "inventory.rg",
    );
    assert_eq!(rejected.status.code(), Some(65));
    assert!(
        !stale_out.exists(),
        "mismatched snapshot must publish nothing"
    );
    let failure: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(failure["status"], "failed");
    assert_eq!(failure["exitCode"], 65);

    let current = request_with_scopes(&repo, "inventory.rg", &["src", "src/nested"]);
    let current_out = root.join("current-out");
    let accepted = run_with_request_file(
        &current,
        &root.join("current-request.json"),
        &current_out,
        "inventory.rg",
    );
    assert_eq!(
        accepted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let inventory = String::from_utf8(fs::read(current_out.join("files.txt")).unwrap()).unwrap();
    assert!(inventory.contains("src"));
    assert!(!inventory.contains("docs"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn head_only_inventory_rejects_untracked_paths_outside_the_head_manifest() {
    let root = temp_dir("head-only-untracked");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Inventory Test"]);
    git(
        &repo,
        &["config", "user.email", "inventory@example.invalid"],
    );
    fs::write(repo.join("tracked.txt"), "tracked\n").unwrap();
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "fixture"]);
    fs::write(repo.join("untracked.txt"), "not in HEAD\n").unwrap();

    let value = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    let out = root.join("out");
    let output = run_with_request_file(&value, &root.join("request.json"), &out, "inventory.rg");

    assert_eq!(output.status.code(), Some(65));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "failed");
    assert_eq!(result["exitCode"], 65);
    assert!(!out.exists(), "manifest mismatch must publish nothing");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn relocated_repositories_emit_identical_inventory_bytes_and_digest() {
    let root = temp_dir("relocated-inventory");
    let left = root.join("left repo 空格");
    let right = root.join("另一个 path");
    for repo in [&left, &right] {
        fs::create_dir_all(repo.join("src/子 目录")).unwrap();
        fs::write(repo.join("README.md"), "same\n").unwrap();
        fs::write(repo.join("src/子 目录/lib.rs"), "pub fn same() {}\n").unwrap();
    }

    let left_out = root.join("left-out");
    let right_out = root.join("right-out");
    let mut left_request = request(&left, "inventory.rg");
    left_request["options"]["inventoryExclude"] = json!(["!*.md"]);
    let mut right_request = request(&right, "inventory.rg");
    right_request["options"]["inventoryExclude"] = json!(["!*.md"]);
    let left_result = run_with_request_file(
        &left_request,
        &root.join("left-request.json"),
        &left_out,
        "inventory.rg",
    );
    let right_result = run_with_request_file(
        &right_request,
        &root.join("right-request.json"),
        &right_out,
        "inventory.rg",
    );
    assert_eq!(left_result.status.code(), Some(0));
    assert_eq!(right_result.status.code(), Some(0));
    assert_eq!(
        fs::read(left_out.join("files.txt")).unwrap(),
        fs::read(right_out.join("files.txt")).unwrap()
    );
    assert!(!fs::read_to_string(left_out.join("files.txt"))
        .unwrap()
        .contains("README.md"));
    let left_json: Value = serde_json::from_slice(&left_result.stdout).unwrap();
    let right_json: Value = serde_json::from_slice(&right_result.stdout).unwrap();
    assert_eq!(
        left_json["artifacts"][0]["sha256"],
        right_json["artifacts"][0]["sha256"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn inventory_rejects_simulated_rg_extra_path_without_publication() {
    let root = temp_dir("rg-extra-path");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("kept.txt"), "kept\n").unwrap();
    let request_path = root.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&repo, "inventory.rg")).unwrap(),
    )
    .unwrap();
    let out = root.join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .env("CODE_INTEL_TEST_RG_EXTRA_PATH", "transient/renamed.rs")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(65));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["exitCode"], 65);
    assert!(!out.exists(), "set mismatch must publish nothing");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn coherence_failure_exits_64_without_partial_artifact() {
    let root = temp_dir("coherence");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("create fixture repo");
    fs::write(repo.join("README.md"), "fixture\n").expect("write README");
    let out = root.join("out");
    let artifact = out.join("files.txt");

    let output = run_with_request_file(
        &request(&repo, "inventory.changed"),
        &root.join("request.json"),
        &out,
        "inventory.rg",
    );

    assert_eq!(output.status.code(), Some(64));
    let result: Value = serde_json::from_slice(&output.stdout).expect("failure result JSON");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["verdict"], "unknown");
    assert_eq!(result["exitCode"], 64);
    assert!(result["artifacts"]
        .as_array()
        .expect("artifacts")
        .is_empty());
    assert!(
        !artifact.exists(),
        "coherence failure must not leave a partial result artifact"
    );
    assert!(
        !output.stderr.is_empty(),
        "human-readable diagnostic must use stderr"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stdin_rejects_more_than_one_request_with_one_failure_result() {
    let root = temp_dir("stream");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("create fixture repo");
    let out = root.join("out");
    let artifact = out.join("files.txt");
    let value = request(&repo, "inventory.rg");
    let mut child = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "inventory.rg",
            "--request",
            "-",
            "--out",
        ])
        .arg(&out)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn executor");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin");
        write!(stdin, "{}\n{}", value, value).expect("write two requests");
    }
    let output = child.wait_with_output().expect("wait executor");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        output.stdout.is_empty(),
        "multi-document input is a pre-envelope failure"
    );
    assert!(!output.stderr.is_empty());
    assert!(!artifact.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn pre_envelope_failures_have_typed_exit_stderr_and_empty_stdout() {
    let root = temp_dir("pre-envelope");
    let missing = root.join("missing.json");
    let out = root.join("out");
    let unreadable = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&missing)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run unreadable request");
    assert_eq!(unreadable.status.code(), Some(74));
    assert!(unreadable.stdout.is_empty());
    assert!(!unreadable.stderr.is_empty());

    let mut child = base_command(&out, "inventory.rg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn malformed request");
    {
        use std::io::Write;
        child.stdin.as_mut().unwrap().write_all(b"{").unwrap();
    }
    let malformed = child.wait_with_output().unwrap();
    assert_eq!(malformed.status.code(), Some(64));
    assert!(malformed.stdout.is_empty());
    assert!(!malformed.stderr.is_empty());
    assert!(!out.exists());
}

#[test]
fn stdin_accepts_exactly_one_request_and_keeps_stdout_pure() {
    let root = temp_dir("stdin-one");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("文 & 'file.txt"), "fixture\n").unwrap();
    let out = root.join("out");
    let mut child = base_command(&out, "inventory.rg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        write!(
            child.stdin.as_mut().unwrap(),
            "{}",
            request(&repo, "inventory.rg")
        )
        .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let mut values = serde_json::Deserializer::from_slice(&output.stdout).into_iter::<Value>();
    assert_eq!(values.next().unwrap().unwrap()["exitCode"], 0);
    assert!(values.next().is_none());
    assert_eq!(
        fs::read(out.join("files.txt")).unwrap(),
        legacy_inventory(&repo)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn registry_drift_fails_closed_with_schema_shaped_result_and_no_output_tree() {
    let root = temp_dir("registry-drift");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    let out = root.join("out");
    let request_path = root.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&repo, "inventory.rg")).unwrap(),
    )
    .unwrap();
    let registry_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("orchestration")
        .join("integrations.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(registry_path).unwrap()).unwrap();
    let declaration = registry["integrations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == "inventory.rg")
        .unwrap();
    declaration["capabilityDeclaration"]["implementation"]["version"] = json!("9.9.9");
    let drift_path = root.join("integrations.json");
    fs::write(&drift_path, serde_json::to_vec(&registry).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--manifest")
        .arg(&drift_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema"], "code-intel-capability-result.v1");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["verdict"], "unknown");
    assert_eq!(result["exitCode"], 64);
    assert!(result["artifacts"].as_array().unwrap().is_empty());
    assert!(!out.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unavailable_rg_maps_to_69_without_partial_output() {
    let root = temp_dir("rg-unavailable");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let out = root.join("out");
    let request_path = root.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&repo, "inventory.rg")).unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(69));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["exitCode"],
        69
    );
    assert!(!out.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn request_cannot_select_artifact_escape_and_out_must_be_unique() {
    let root = temp_dir("write-boundary");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    let out = root.join("out");
    let mut escaping = request(&repo, "inventory.rg");
    escaping["options"]["artifactPath"] = json!("../escape.txt");
    let escaped = run_with_request_file(
        &escaping,
        &root.join("escape-request.json"),
        &out,
        "inventory.rg",
    );
    assert_eq!(escaped.status.code(), Some(64));
    assert!(!out.exists());
    assert!(!root.join("escape.txt").exists());

    fs::create_dir(&out).unwrap();
    fs::write(out.join("sentinel.txt"), "owned elsewhere").unwrap();
    let existing = run_with_request_file(
        &request(&repo, "inventory.rg"),
        &root.join("existing-request.json"),
        &out,
        "inventory.rg",
    );
    assert_eq!(existing.status.code(), Some(74));
    assert_eq!(
        fs::read_to_string(out.join("sentinel.txt")).unwrap(),
        "owned elsewhere"
    );
    assert!(!out.join("files.txt").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn capability_parser_rejects_missing_unknown_duplicate_and_conflicting_arguments() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["capability", "exec", "inventory.rg", "--request"],
        vec![
            "capability",
            "exec",
            "inventory.rg",
            "--bogus",
            "x",
            "--request",
            "x",
            "--out",
            "y",
        ],
        vec![
            "capability",
            "exec",
            "inventory.rg",
            "--request",
            "a",
            "--request",
            "b",
            "--out",
            "y",
        ],
        vec![
            "capability",
            "exec",
            "inventory.rg",
            "--request",
            "a",
            "--out",
            "x",
            "--out",
            "y",
        ],
        vec![
            "capability",
            "exec",
            "inventory.rg",
            "--request",
            "a",
            "--out",
            "y",
            "--manifest",
            "a",
            "--manifest",
            "b",
        ],
        vec![
            "capability",
            "exec",
            "inventory.rg",
            "--request-file",
            "a",
            "--out",
            "y",
        ],
        vec![
            "capability",
            "exec",
            "inventory.rg",
            "--capability",
            "other",
            "--request",
            "a",
            "--out",
            "y",
        ],
    ];
    for args in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64), "args={args:?}");
        assert!(output.stdout.is_empty(), "args={args:?}");
        assert!(!output.stderr.is_empty(), "args={args:?}");
    }
    let other_command = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["doctor", "--request", "x"])
        .output()
        .unwrap();
    assert_eq!(other_command.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&other_command.stderr).contains("unknown argument for doctor"));
}

#[test]
fn duplicate_json_keys_are_rejected_for_request_and_registry() {
    let root = temp_dir("duplicate-json");
    fs::create_dir_all(&root).unwrap();
    let out = root.join("out");
    let request_path = root.join("request.json");
    fs::write(&request_path, "\u{feff}{\"schema\":\"code-intel-capability-request.v1\",\"schema\":\"code-intel-capability-request.v1\"}").unwrap();
    let request_output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(request_output.status.code(), Some(64));
    assert!(request_output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&request_output.stderr).contains("duplicate JSON object key"));

    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&repo, "inventory.rg")).unwrap(),
    )
    .unwrap();
    let registry_path = root.join("registry.json");
    fs::write(
        &registry_path,
        "\u{feff}{\"integrations\":[],\"integrations\":[]}",
    )
    .unwrap();
    let registry_output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--manifest")
        .arg(&registry_path)
        .output()
        .unwrap();
    assert_eq!(registry_output.status.code(), Some(65));
    let result: Value = serde_json::from_slice(&registry_output.stdout).unwrap();
    assert_eq!(result["exitCode"], 65);
    assert!(String::from_utf8_lossy(&registry_output.stderr).contains("duplicate JSON object key"));
    assert!(!out.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_registered_declarations_fail_closed() {
    let root = temp_dir("duplicate-declaration");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let request_path = root.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&repo, "inventory.rg")).unwrap(),
    )
    .unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("orchestration")
        .join("integrations.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(source).unwrap()).unwrap();
    let duplicate = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "inventory.rg")
        .unwrap()
        .clone();
    registry["integrations"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    let registry_path = root.join("registry.json");
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let out = root.join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--manifest")
        .arg(&registry_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("duplicate registered capability declaration"));
    assert!(!out.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_repository_is_a_successful_empty_inventory() {
    let root = temp_dir("empty");
    let repo = root.join("empty repo & 文");
    fs::create_dir_all(&repo).unwrap();
    let out = root.join("out");
    let output = run_with_request_file(
        &request(&repo, "inventory.rg"),
        &root.join("request.json"),
        &out,
        "inventory.rg",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(out.join("files.txt")).unwrap(), Vec::<u8>::new());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["exitCode"],
        0
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn inventory_exclude_is_applied_without_shell_interpretation() {
    let root = temp_dir("custom-exclude");
    let repo = root.join("repo & 文");
    fs::create_dir_all(repo.join("custom excluded")).unwrap();
    fs::write(repo.join("keep.txt"), "keep").unwrap();
    fs::write(repo.join("custom excluded").join("drop.txt"), "drop").unwrap();
    let mut value = request(&repo, "inventory.rg");
    value["options"]["inventoryExclude"] = json!(["!**/custom excluded/**"]);
    let out = root.join("out");
    let output = run_with_request_file(&value, &root.join("request.json"), &out, "inventory.rg");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = fs::read_to_string(out.join("files.txt")).unwrap();
    assert!(files.contains("keep.txt"));
    assert!(!files.contains("drop.txt"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn tracked_symlink_is_snapshot_bound_but_omitted_from_inventory_for_both_policies() {
    let root = temp_dir("tracked-symlink");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Symlink Test"]);
    git(&repo, &["config", "user.email", "symlink@example.invalid"]);
    git(&repo, &["config", "core.symlinks", "true"]);
    fs::write(repo.join("target-one.txt"), "one\n").unwrap();
    fs::write(repo.join("target-two.txt"), "two\n").unwrap();
    let link = repo.join("link.txt");
    if !create_file_symlink(Path::new("target-one.txt"), &link) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "tracked symlink"]);

    let real = Command::new(if cfg!(windows) { "rg.exe" } else { "rg" })
        .args(["--files", "--hidden", "--no-ignore"])
        .current_dir(&repo)
        .output()
        .expect("run real rg symlink fixture");
    assert!(real.status.success());
    assert!(
        !String::from_utf8_lossy(&real.stdout)
            .lines()
            .any(|path| path == "link.txt"),
        "ripgrep without -L must not enumerate a symlink"
    );

    let head_request = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    let explicit_before =
        request_with_policy_scopes(&repo, "inventory.rg", "explicit_overlay", &["."]);
    for (name, request) in [
        ("head", &head_request),
        ("explicit-before", &explicit_before),
    ] {
        let out = root.join(format!("{name}-out"));
        let output = run_with_request_file(
            request,
            &root.join(format!("{name}-request.json")),
            &out,
            "inventory.rg",
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let files = fs::read_to_string(out.join("files.txt")).unwrap();
        assert!(!files.lines().any(|path| path == "link.txt"));
        assert!(files.lines().any(|path| path == "target-one.txt"));
        assert!(files.lines().any(|path| path == "target-two.txt"));
    }

    fs::remove_file(&link).unwrap();
    assert!(create_file_symlink(Path::new("target-two.txt"), &link));
    let explicit_after =
        request_with_policy_scopes(&repo, "inventory.rg", "explicit_overlay", &["."]);
    assert_ne!(
        explicit_before["snapshot"]["identity"], explicit_after["snapshot"]["identity"],
        "explicit snapshot identity must bind the symlink target bytes"
    );
    let head_after = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    assert_eq!(
        head_request["snapshot"]["identity"], head_after["snapshot"]["identity"],
        "head-only identity must remain bound to the committed symlink target"
    );
    let after_out = root.join("explicit-after-out");
    let after = run_with_request_file(
        &explicit_after,
        &root.join("explicit-after-request.json"),
        &after_out,
        "inventory.rg",
    );
    assert_eq!(
        after.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&after.stderr)
    );
    let after_files = fs::read_to_string(after_out.join("files.txt")).unwrap();
    assert!(!after_files.lines().any(|path| path == "link.txt"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn populated_gitlink_is_literal_excluded_and_oid_bound_for_both_policies() {
    let root = temp_dir("populated-gitlink");
    let child = root.join("child");
    let repo = root.join("repo");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&repo).unwrap();
    git(&child, &["init", "--quiet"]);
    git(&child, &["config", "user.name", "Submodule Test"]);
    git(
        &child,
        &["config", "user.email", "submodule@example.invalid"],
    );
    fs::write(child.join("inside.txt"), "one\n").unwrap();
    git(&child, &["add", "."]);
    git(&child, &["commit", "--quiet", "-m", "child one"]);

    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Superproject Test"]);
    git(&repo, &["config", "user.email", "super@example.invalid"]);
    fs::write(repo.join("root.txt"), "root\n").unwrap();
    let gitlink = "vendor/sub[glob]{x}!";
    git(
        &repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            child.to_str().unwrap(),
            gitlink,
        ],
    );
    git(&repo, &["add", "."]);
    git(
        &repo,
        &["commit", "--quiet", "-m", "add populated submodule"],
    );

    let real = Command::new(if cfg!(windows) { "rg.exe" } else { "rg" })
        .args(["--files", "--hidden", "--no-ignore"])
        .current_dir(&repo)
        .output()
        .expect("run real rg populated submodule fixture");
    assert!(real.status.success());
    assert!(
        String::from_utf8_lossy(&real.stdout)
            .lines()
            .any(|path| path.replace('\\', "/").starts_with(gitlink)),
        "fixture must prove that unfiltered real rg descends into the populated gitlink"
    );

    let head_before = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    let explicit_before =
        request_with_policy_scopes(&repo, "inventory.rg", "explicit_overlay", &["."]);
    for (name, request) in [
        ("gitlink-head-before", &head_before),
        ("gitlink-explicit-before", &explicit_before),
    ] {
        let out = root.join(format!("{name}-out"));
        let output = run_with_request_file(
            request,
            &root.join(format!("{name}-request.json")),
            &out,
            "inventory.rg",
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let files = fs::read_to_string(out.join("files.txt")).unwrap();
        assert!(files.lines().any(|path| path == ".gitmodules"));
        assert!(files.lines().any(|path| path == "root.txt"));
        assert!(!files.lines().any(|path| path.starts_with(gitlink)));
    }

    fs::write(child.join("inside.txt"), "two\n").unwrap();
    git(&child, &["add", "inside.txt"]);
    git(&child, &["commit", "--quiet", "-m", "child two"]);
    let second_oid = git_stdout(&child, &["rev-parse", "HEAD"]);
    let populated = repo.join(gitlink);
    git(&populated, &["fetch", "--quiet", "origin"]);
    git(&populated, &["checkout", "--quiet", &second_oid]);
    git(&repo, &["add", gitlink]);

    let explicit_after =
        request_with_policy_scopes(&repo, "inventory.rg", "explicit_overlay", &["."]);
    assert_ne!(
        explicit_before["snapshot"]["identity"], explicit_after["snapshot"]["identity"],
        "staging a different gitlink OID must change explicit snapshot identity"
    );
    let staged_head = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    assert_eq!(
        head_before["snapshot"]["identity"], staged_head["snapshot"]["identity"],
        "head-only identity must ignore an uncommitted gitlink update"
    );
    let explicit_out = root.join("gitlink-explicit-after-out");
    let explicit_output = run_with_request_file(
        &explicit_after,
        &root.join("gitlink-explicit-after-request.json"),
        &explicit_out,
        "inventory.rg",
    );
    assert_eq!(
        explicit_output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&explicit_output.stderr)
    );
    assert!(!fs::read_to_string(explicit_out.join("files.txt"))
        .unwrap()
        .lines()
        .any(|path| path.starts_with(gitlink)));

    git(&repo, &["commit", "--quiet", "-m", "advance gitlink"]);
    let head_after = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    assert_ne!(
        head_before["snapshot"]["identity"], head_after["snapshot"]["identity"],
        "committing a different gitlink OID must change head-only snapshot identity"
    );
    let head_out = root.join("gitlink-head-after-out");
    let head_output = run_with_request_file(
        &head_after,
        &root.join("gitlink-head-after-request.json"),
        &head_out,
        "inventory.rg",
    );
    assert_eq!(
        head_output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&head_output.stderr)
    );
    assert!(!fs::read_to_string(head_out.join("files.txt"))
        .unwrap()
        .lines()
        .any(|path| path.starts_with(gitlink)));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn inventory_exclude_uses_ripgrep_glob_semantics_for_basename_brace_class_and_segment() {
    let root = temp_dir("ripgrep-glob-semantics");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("nested/generated")).unwrap();
    fs::create_dir_all(repo.join("nested/kept")).unwrap();
    for (path, content) in [
        ("README.md", "markdown"),
        ("nested/guide.md", "nested markdown"),
        ("main.rs", "rust"),
        ("notes.txt", "text"),
        ("alpha.classcase", "class a"),
        ("beta.classcase", "class b"),
        ("charlie.classcase", "not class"),
        ("nested/generated/data.json", "segment"),
        ("nested/kept/data.json", "kept"),
    ] {
        fs::write(repo.join(path), content).unwrap();
    }
    let mut value = request(&repo, "inventory.rg");
    value["options"]["inventoryExclude"] = json!([
        "!*.md",
        "!*.{rs,txt}",
        "![ab]*.classcase",
        "!**/generated/**"
    ]);
    let out = root.join("out");
    let output = run_with_request_file(&value, &root.join("request.json"), &out, "inventory.rg");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalized_lines(&fs::read_to_string(out.join("files.txt")).unwrap()),
        vec![
            "charlie.classcase".to_string(),
            "nested/kept/data.json".to_string()
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn snapshot_ignore_control_bytes_drive_root_and_nested_ripgrep_semantics() {
    let root = temp_dir("snapshot-ignore-controls");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("git-rule")).unwrap();
    fs::create_dir_all(repo.join("ignore-rule")).unwrap();
    fs::create_dir_all(repo.join("rgignore-rule")).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Ignore Test"]);
    git(&repo, &["config", "user.email", "ignore@example.invalid"]);
    fs::write(repo.join("tracked.foo"), "tracked first\n").unwrap();
    fs::write(repo.join("kept.foo"), "kept\n").unwrap();
    fs::write(repo.join(".gitignore"), "tracked.foo\n").unwrap();
    fs::write(repo.join("git-rule/.gitignore"), "drop.foo\n").unwrap();
    fs::write(repo.join("git-rule/drop.foo"), "drop\n").unwrap();
    fs::write(repo.join("git-rule/keep.foo"), "keep\n").unwrap();
    fs::write(repo.join("ignore-rule/.ignore"), "drop.foo\n").unwrap();
    fs::write(repo.join("ignore-rule/drop.foo"), "drop\n").unwrap();
    fs::write(repo.join("ignore-rule/keep.foo"), "keep\n").unwrap();
    fs::write(repo.join("rgignore-rule/.rgignore"), "drop.foo\n").unwrap();
    fs::write(repo.join("rgignore-rule/drop.foo"), "drop\n").unwrap();
    fs::write(repo.join("rgignore-rule/keep.foo"), "keep\n").unwrap();
    git(&repo, &["add", "-f", "."]);
    git(&repo, &["commit", "--quiet", "-m", "ignore controls"]);

    let out = root.join("out");
    let output = run_with_request_file(
        &request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]),
        &root.join("request.json"),
        &out,
        "inventory.rg",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = fs::read_to_string(out.join("files.txt")).unwrap();
    assert!(files.contains("kept.foo"));
    assert!(files.contains("git-rule/keep.foo"));
    assert!(files.contains("ignore-rule/keep.foo"));
    assert!(files.contains("rgignore-rule/keep.foo"));
    assert!(!files.contains("tracked.foo"));
    assert!(!files.contains("git-rule/drop.foo"));
    assert!(!files.contains("ignore-rule/drop.foo"));
    assert!(!files.contains("rgignore-rule/drop.foo"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn head_only_inventory_uses_frozen_ignore_bytes_for_same_and_different_worktree_semantics() {
    let root = temp_dir("frozen-head-ignore");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Ignore Test"]);
    git(&repo, &["config", "user.email", "ignore@example.invalid"]);
    fs::write(repo.join("hidden.txt"), "hidden\n").unwrap();
    fs::write(repo.join("other.txt"), "other\n").unwrap();
    fs::write(repo.join(".gitignore"), "hidden.txt\n").unwrap();
    git(&repo, &["add", "-f", "."]);
    git(&repo, &["commit", "--quiet", "-m", "ignore control"]);
    let value = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["."]);
    fs::write(repo.join(".gitignore"), "hidden.txt\n# same semantics\n").unwrap();
    let same_out = root.join("same-out");
    let same = run_with_request_file(
        &value,
        &root.join("same-request.json"),
        &same_out,
        "inventory.rg",
    );
    assert_eq!(
        same.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&same.stderr)
    );
    let same_bytes = fs::read(same_out.join("files.txt")).unwrap();
    let same_files = String::from_utf8(same_bytes.clone()).unwrap();
    assert!(!same_files.contains("hidden.txt"));
    assert!(same_files.contains("other.txt"));

    fs::write(repo.join(".gitignore"), "other.txt\n").unwrap();
    let different_out = root.join("different-out");
    let different = run_with_request_file(
        &value,
        &root.join("different-request.json"),
        &different_out,
        "inventory.rg",
    );
    assert_eq!(
        different.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&different.stderr)
    );
    assert_eq!(
        same_bytes,
        fs::read(different_out.join("files.txt")).unwrap(),
        "head_only artifact must depend on HEAD ignore bytes, not live worktree bytes"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_overlay_rejects_same_semantics_ignore_byte_change_as_stale() {
    let root = temp_dir("stale-overlay-ignore");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("hidden.txt"), "hidden\n").unwrap();
    fs::write(repo.join(".gitignore"), "hidden.txt\n").unwrap();
    let value = request(&repo, "inventory.rg");
    fs::write(repo.join(".gitignore"), "hidden.txt\n# same semantics\n").unwrap();
    let out = root.join("out");
    let output = run_with_request_file(&value, &root.join("request.json"), &out, "inventory.rg");
    assert_eq!(output.status.code(), Some(65));
    assert!(!out.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scoped_inventory_disables_ancestor_ignore_and_binds_nested_controls() {
    let root = temp_dir("scoped-ancestor-ignore");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src/nested")).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Ignore Test"]);
    git(&repo, &["config", "user.email", "ignore@example.invalid"]);
    fs::write(repo.join(".gitignore"), "src/drop.foo\n").unwrap();
    fs::write(repo.join("src/drop.foo"), "drop\n").unwrap();
    fs::write(repo.join("src/keep.foo"), "keep\n").unwrap();
    fs::write(repo.join("src/nested/.ignore"), "nested-drop.foo\n").unwrap();
    fs::write(repo.join("src/nested/nested-drop.foo"), "drop\n").unwrap();
    fs::write(repo.join("src/nested/nested-keep.foo"), "keep\n").unwrap();
    git(&repo, &["add", "-f", "."]);
    git(
        &repo,
        &["commit", "--quiet", "-m", "scoped ignore controls"],
    );
    let value = request_with_policy_scopes(&repo, "inventory.rg", "head_only", &["src"]);
    let out = root.join("out");
    let output = run_with_request_file(&value, &root.join("request.json"), &out, "inventory.rg");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = fs::read_to_string(out.join("files.txt")).unwrap();
    assert!(files.contains("src/keep.foo"));
    assert!(files.contains("src/nested/nested-keep.foo"));
    assert!(
        files.contains("src/drop.foo"),
        "--no-ignore-parent excludes the root control from a src-scoped traversal"
    );
    assert!(!files.contains("src/nested/nested-drop.foo"));
    assert!(
        !files.contains(".gitignore"),
        "ancestor control is input, not scoped output"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn inventory_disables_info_parent_and_ripgrep_config_ignore_sources() {
    let root = temp_dir("external-ignore-sources");
    let repo = root.join("parent").join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(root.join("parent/.ignore"), "parent-hidden.txt\n").unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Ignore Test"]);
    git(&repo, &["config", "user.email", "ignore@example.invalid"]);
    fs::write(repo.join("tracked.txt"), "tracked\n").unwrap();
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "fixture"]);
    fs::write(repo.join("info-hidden.txt"), "info\n").unwrap();
    fs::write(repo.join("parent-hidden.txt"), "parent\n").unwrap();
    fs::write(repo.join("config-hidden.txt"), "config\n").unwrap();
    fs::write(repo.join(".git/info/exclude"), "info-hidden.txt\n").unwrap();
    let config = root.join("ripgrep.conf");
    fs::write(&config, "--glob=!config-hidden.txt\n").unwrap();
    let value = request(&repo, "inventory.rg");
    let request_path = root.join("request.json");
    fs::write(&request_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let out = root.join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .env("RIPGREP_CONFIG_PATH", &config)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = fs::read_to_string(out.join("files.txt")).unwrap();
    assert!(files.contains("info-hidden.txt"));
    assert!(files.contains("parent-hidden.txt"));
    assert!(files.contains("config-hidden.txt"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn normalized_inventory_matches_real_legacy_runner_with_custom_exclude() {
    let root = temp_dir("real-legacy");
    let repo = root.join("repo & 文");
    let artifacts = root.join("legacy artifacts");
    fs::create_dir_all(repo.join("custom excluded")).unwrap();
    fs::create_dir_all(repo.join("target")).unwrap();
    fs::write(repo.join("keep.txt"), "keep").unwrap();
    fs::write(repo.join(".hidden"), "hidden").unwrap();
    fs::write(repo.join("custom excluded").join("drop.txt"), "drop").unwrap();
    fs::write(repo.join("target").join("built.txt"), "built").unwrap();
    let runner = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("run-code-intel.ps1");
    let legacy = Command::new("pwsh")
        .args(["-NoProfile", "-File"])
        .arg(&runner)
        .arg("-RepoPath")
        .arg(&repo)
        .arg("-Mode")
        .arg("lite")
        .arg("-SkipOpenSpec")
        .arg("-ArtifactRoot")
        .arg(&artifacts)
        .arg("-InventoryExclude")
        .arg("!**/custom excluded/**")
        .output()
        .unwrap();
    assert!(
        legacy.status.success(),
        "legacy stderr={} stdout={}",
        String::from_utf8_lossy(&legacy.stderr),
        String::from_utf8_lossy(&legacy.stdout)
    );
    let legacy_files = find_named_file(&artifacts, "files.txt").expect("legacy runner files.txt");

    let mut value = request(&repo, "inventory.rg");
    value["options"]["inventoryExclude"] = json!(["!**/custom excluded/**"]);
    let out = root.join("new staging");
    let new = run_with_request_file(&value, &root.join("request.json"), &out, "inventory.rg");
    assert_eq!(
        new.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&new.stderr)
    );
    assert_eq!(
        normalized_repo_lines(&fs::read_to_string(legacy_files).unwrap(), &repo),
        normalized_lines(&fs::read_to_string(out.join("files.txt")).unwrap())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn advisory_workflow_recommend_runs_through_a01_with_zero_effects_and_facade_parity() {
    let root = temp_dir("workflow-recommend");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("openspec")).unwrap();
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/main.ps1"), "'ok'\n").unwrap();

    let mut value = request(&repo, "advisory.workflow-recommend");
    value["implementation"] = json!({
        "id":"advisory.workflow-recommend.compat",
        "version":"1.0.0",
        "toolchainDigests":[
            "03d9cbed70d83c59f7d9540fccc606ce0b2723135efd2c5e32943d367008a199",
            "748c8b087c9d1a68f9aa5711cda200204ac0d05845058a1ee50058b161582de9"
        ]
    });
    value["options"] = json!({"repoPath":repo,"auto":true});
    value["effectPolicy"]["allowedEffects"] = json!([]);
    let out = root.join("out");
    let output = run_with_request_file(
        &value,
        &root.join("request.json"),
        &out,
        "advisory.workflow-recommend",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "successful envelope stderr must be empty"
    );
    let envelope: Value = serde_json::from_slice(&output.stdout)
        .expect("stdout must contain exactly one capability result JSON document");
    assert_eq!(envelope["capability"], "advisory.workflow-recommend");
    assert_eq!(envelope["declaredEffects"], json!([]));
    assert_eq!(envelope["observedEffects"], json!([]));
    assert_eq!(envelope["artifacts"].as_array().unwrap().len(), 1);
    let envelope_proposal: Value =
        serde_json::from_slice(&fs::read(out.join("workflow-recommendation.json")).unwrap())
            .unwrap();
    assert_eq!(envelope_proposal["effects"], json!([]));
    assert_eq!(envelope_proposal["kind"], "proposal");

    let facade = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Invoke-WorkflowRecommendation.ps1");
    let direct = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(facade)
        .arg("-RepoPath")
        .arg(&repo)
        .args(["-Auto", "-Quiet", "-Json"])
        .output()
        .unwrap();
    assert!(
        direct.status.success(),
        "{}",
        String::from_utf8_lossy(&direct.stderr)
    );
    let direct_proposal: Value =
        serde_json::from_slice(&direct.stdout).expect("facade JSON mode must keep stdout pure");
    assert_eq!(envelope_proposal, direct_proposal);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn excessive_json_nesting_is_a_pre_envelope_usage_failure() {
    let root = temp_dir("deep-json");
    fs::create_dir_all(&root).unwrap();
    let request_path = root.join("request.json");
    fs::write(
        &request_path,
        format!("{}0{}", "[".repeat(512), "]".repeat(512)),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(root.join("out"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nesting"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn oversized_request_is_bounded_and_rejected_before_envelope() {
    let root = temp_dir("oversized-json");
    fs::create_dir_all(&root).unwrap();
    let request_path = root.join("request.json");
    fs::write(&request_path, vec![b' '; 8 * 1024 * 1024 + 1]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(root.join("out"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cwd_manifest_shadow_is_ignored() {
    let root = temp_dir("manifest-shadow");
    let repo = root.join("repo");
    let cwd = root.join("untrusted");
    fs::create_dir_all(cwd.join("orchestration")).unwrap();
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("kept.txt"), "kept").unwrap();
    fs::write(
        cwd.join("orchestration").join("integrations.json"),
        r#"{"integrations":[]}"#,
    )
    .unwrap();
    let request_path = root.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request(&repo, "inventory.rg")).unwrap(),
    )
    .unwrap();
    let out = root.join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .current_dir(&cwd)
        .env_remove("CODE_INTEL_HOME")
        .env_remove("CODE_INTEL_INTEGRATIONS_MANIFEST")
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.join("files.txt").is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn declaration_determinism_is_used_for_post_declaration_failures() {
    let root = temp_dir("declaration-determinism");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let mut value = request(&repo, "inventory.rg");
    value["options"]["unsupported"] = json!(true);
    let request_path = root.join("request.json");
    fs::write(&request_path, serde_json::to_vec(&value).unwrap()).unwrap();

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("orchestration")
        .join("integrations.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(source).unwrap()).unwrap();
    let declaration = registry["integrations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == "inventory.rg")
        .unwrap()
        .get_mut("capabilityDeclaration")
        .unwrap();
    declaration["determinism"] = json!("external_nondeterministic");
    let registry_path = root.join("registry.json");
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "inventory.rg", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(root.join("out"))
        .arg("--manifest")
        .arg(&registry_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["determinism"], "external_nondeterministic");
    assert_eq!(result["exitCode"], 64);
    let _ = fs::remove_dir_all(root);
}

fn normalized_lines(text: &str) -> Vec<String> {
    let mut lines = text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.sort();
    lines.dedup();
    lines
}

fn normalized_repo_lines(text: &str, repo: &Path) -> Vec<String> {
    let prefix = format!("{}/", repo.to_string_lossy().replace('\\', "/"));
    let relative = text
        .lines()
        .map(|line| line.trim().replace('\\', "/"))
        .map(|line| line.strip_prefix(&prefix).unwrap_or(&line).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    normalized_lines(&relative)
}

fn find_named_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path)
            } else if path.file_name().and_then(|v| v.to_str()) == Some(name) {
                return Some(path);
            }
        }
    }
    None
}
