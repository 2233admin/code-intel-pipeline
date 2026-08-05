mod common;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "code-intel-b10-{}-{nonce}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("orchestration")
        .join("integrations.json")
}

fn declaration(id: &str) -> Value {
    let registry: Value =
        serde_json::from_slice(&fs::read(manifest_path()).expect("read registry"))
            .expect("parse registry");
    registry["integrations"]
        .as_array()
        .expect("integrations")
        .iter()
        .find_map(|integration| {
            (integration
                .pointer("/capabilityDeclaration/id")
                .and_then(Value::as_str)
                == Some(id))
            .then(|| integration["capabilityDeclaration"].clone())
        })
        .expect("registered declaration")
}

fn snapshot(repo: &Path) -> Value {
    let output = common::cli()
        .args(["snapshot", "identity", "--repo"])
        .arg(repo)
        .args(["--working-tree-policy", "explicit_overlay", "--scope", "."])
        .output()
        .expect("snapshot identity");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).expect("snapshot JSON")["snapshot"].clone()
}

fn request(capability: &str, snapshot: &Value, options: Value, inputs: Value) -> Value {
    let declaration = declaration(capability);
    json!({
        "schema":"code-intel-capability-request.v1",
        "capability":capability,
        "contractVersion":1,
        "implementation":declaration["implementation"],
        "snapshot":snapshot,
        "options":options,
        "inputs":inputs,
        "effectPolicy":{"allowedEffects":declaration["allowedEffects"]}
    })
}

fn exec(
    request: &Value,
    capability: &str,
    request_path: &Path,
    out: &Path,
    artifact_root: Option<&Path>,
    path_prefix: Option<&Path>,
) -> std::process::Output {
    fs::write(request_path, serde_json::to_vec(request).unwrap()).expect("write request");
    let mut command = common::cli();
    command
        .args(["capability", "exec", capability, "--request"])
        .arg(request_path)
        .arg("--out")
        .arg(out)
        .arg("--manifest")
        .arg(manifest_path());
    if let Some(root) = artifact_root {
        command.arg("--artifact-root").arg(root);
    }
    if let Some(prefix) = path_prefix {
        let mut paths = vec![prefix.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        command.env("PATH", std::env::join_paths(paths).expect("fixture PATH"));
    }
    command.output().expect("capability exec")
}

#[test]
fn doctor_cli_emits_one_envelope_and_snapshot_bound_redacted_observation() {
    let root = temp_dir();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    let snapshot = snapshot(&repo);

    let snapshot_out = root.join("snapshot");
    let snapshot_result = exec(
        &request(
            "repo.snapshot",
            &snapshot,
            json!({"repoPath":repo}),
            json!([]),
        ),
        "repo.snapshot",
        &root.join("snapshot-request.json"),
        &snapshot_out,
        None,
        None,
    );
    assert_eq!(snapshot_result.status.code(), Some(0));
    let snapshot_envelope: Value = serde_json::from_slice(&snapshot_result.stdout).unwrap();
    let mut input = snapshot_envelope["artifacts"][0].clone();
    input["path"] = json!("snapshot/snapshot.json");

    let doctor_out = root.join("doctor");
    let doctor_result = exec(
        &request(
            "doctor",
            &snapshot,
            json!({"repoPath":repo,"requireRepowise":false,"requireUnderstand":false}),
            json!([input]),
        ),
        "doctor",
        &root.join("doctor-request.json"),
        &doctor_out,
        Some(&root),
        None,
    );
    assert!(matches!(doctor_result.status.code(), Some(0 | 10)));
    let text = String::from_utf8(doctor_result.stdout).unwrap();
    let mut stream = serde_json::Deserializer::from_str(&text).into_iter::<Value>();
    let envelope = stream.next().unwrap().unwrap();
    assert!(
        stream.next().is_none(),
        "stdout must contain exactly one JSON result"
    );
    assert_eq!(envelope["capability"], "doctor");
    assert_eq!(envelope["status"], "completed");
    assert_eq!(envelope["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(
        envelope["artifacts"][0]["consumedSnapshotIdentity"],
        snapshot["identity"]
    );
    let observation_text = fs::read_to_string(doctor_out.join("doctor-observation.json")).unwrap();
    let observation: Value = serde_json::from_str(&observation_text).unwrap();
    assert_eq!(observation["schema"], "code-intel-doctor-observation.v1");
    assert_eq!(observation["engineeringFacts"], json!([]));
    assert!(!observation_text.to_ascii_lowercase().contains("password="));
    fs::remove_dir_all(root).ok();
}

#[test]
fn doctor_cli_fails_closed_without_verified_snapshot_input() {
    let root = temp_dir();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let snapshot = snapshot(&repo);
    let out = root.join("doctor");
    let output = exec(
        &request("doctor", &snapshot, json!({"repoPath":repo}), json!([])),
        "doctor",
        &root.join("request.json"),
        &out,
        None,
        None,
    );
    assert_eq!(output.status.code(), Some(65));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["status"], "failed");
    assert_eq!(envelope["verdict"], "unknown");
    assert!(envelope["artifacts"].as_array().unwrap().is_empty());
    assert!(!out.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn doctor_cli_reports_nonconforming_provider_and_manifest_drift_as_domain_failure() {
    let root = temp_dir();
    let repo = root.join("repo");
    let fixture_bin = root.join("fixture-bin");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&fixture_bin).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    #[cfg(windows)]
    fs::write(
        fixture_bin.join("sentrux.cmd"),
        "@echo off\r\necho Authorization: Bearer fixture-secret-token\r\nexit /b 0\r\n",
    )
    .unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let sentrux = fixture_bin.join("sentrux");
        fs::write(
            &sentrux,
            "#!/bin/sh\necho 'Authorization: Bearer fixture-secret-token'\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&sentrux, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut drift: Value = serde_json::from_slice(&fs::read(manifest_path()).unwrap()).unwrap();
    let doctor = drift["integrations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry.pointer("/capabilityDeclaration/id") == Some(&json!("doctor")))
        .unwrap();
    doctor["capabilityDeclaration"]["implementation"]["adapter"] = json!("doctor.envelope.drifted");
    let drift_path = root.join("integrations-drift.json");
    fs::write(&drift_path, serde_json::to_vec(&drift).unwrap()).unwrap();

    let snapshot = snapshot(&repo);
    let snapshot_out = root.join("snapshot");
    let snapshot_result = exec(
        &request(
            "repo.snapshot",
            &snapshot,
            json!({"repoPath":repo}),
            json!([]),
        ),
        "repo.snapshot",
        &root.join("snapshot-request.json"),
        &snapshot_out,
        None,
        None,
    );
    assert_eq!(snapshot_result.status.code(), Some(0));
    let snapshot_envelope: Value = serde_json::from_slice(&snapshot_result.stdout).unwrap();
    let mut input = snapshot_envelope["artifacts"][0].clone();
    input["path"] = json!("snapshot/snapshot.json");

    let doctor_out = root.join("doctor");
    let doctor_result = exec(
        &request(
            "doctor",
            &snapshot,
            json!({
                "repoPath":repo,
                "manifestPath":drift_path,
                "requireRepowise":false,
                "requireUnderstand":false
            }),
            json!([input]),
        ),
        "doctor",
        &root.join("doctor-request.json"),
        &doctor_out,
        Some(&root),
        Some(&fixture_bin),
    );

    assert_eq!(doctor_result.status.code(), Some(10));
    assert!(doctor_result.stderr.is_empty());
    let stdout = String::from_utf8(doctor_result.stdout).unwrap();
    let mut stream = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let envelope = stream.next().unwrap().unwrap();
    assert!(stream.next().is_none(), "stdout must be one JSON result");
    assert_eq!(envelope["status"], "completed");
    assert_eq!(envelope["verdict"], "fail");
    assert_eq!(envelope["exitCode"], 10);
    assert_eq!(envelope["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(
        envelope["artifacts"][0]["consumedSnapshotIdentity"],
        snapshot["identity"]
    );
    let diagnostic_text = serde_json::to_string(&envelope["diagnostics"]).unwrap();
    assert!(diagnostic_text.contains("provider conformance failed"));
    assert!(diagnostic_text.contains("manifest reconciliation failed"));
    assert!(!diagnostic_text.contains("fixture-secret-token"));

    let observation_text = fs::read_to_string(doctor_out.join("doctor-observation.json")).unwrap();
    let observation: Value = serde_json::from_str(&observation_text).unwrap();
    let sentrux = observation["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "sentrux")
        .unwrap();
    assert_eq!(sentrux["presence"], "present");
    assert_eq!(sentrux["readiness"], "unavailable");
    assert_eq!(sentrux["conformance"], "nonconforming");
    assert_eq!(sentrux["admissibility"], "not_evaluated");
    assert_eq!(observation["manifest"]["reconciled"], false);
    assert_eq!(observation["engineeringFacts"], json!([]));
    assert!(!observation_text.contains("fixture-secret-token"));
    assert!(!observation_text.contains(root.to_string_lossy().as_ref()));
    fs::remove_dir_all(root).ok();
}

/// A pipeline checkout shaped like the real one: `legacy/` plus the crate
/// files `graphProvider` probes for, so `sourceFound`/`cargoFound` are both
/// true. That is the whole point — those two are the only things
/// `--require-understand` used to check, and inside a checkout they are
/// trivially satisfied.
fn pipeline_scratch(root: &Path) {
    let crate_src = root.join("crates").join("code-intel-cli").join("src");
    fs::create_dir_all(&crate_src).unwrap();
    fs::create_dir_all(root.join("legacy")).unwrap();
    // graph.rs is a directory module (mod.rs + tests.rs, issue #155's
    // god-file split): the probe's sourceFound check looks for
    // src/graph/mod.rs.
    let graph_dir = crate_src.join("graph");
    fs::create_dir_all(&graph_dir).unwrap();
    fs::write(graph_dir.join("mod.rs"), "// graph provider\n").unwrap();
    fs::write(
        root.join("crates")
            .join("code-intel-cli")
            .join("Cargo.toml"),
        "[package]\n",
    )
    .unwrap();
}

fn doctor_bootstrap(pipeline_root: &Path, home: &Path) -> Value {
    let output = common::cli()
        .args(["doctor", "bootstrap", "--pipeline-root"])
        .arg(pipeline_root)
        .args(["--no-require-repowise", "--require-understand", "--json"])
        // The probe derives the Understand Anything candidates from the home
        // directory, so pinning it is what makes this test independent of
        // whatever the developer or CI runner happens to have installed.
        .env("USERPROFILE", home)
        .env("HOME", home)
        .output()
        .expect("doctor bootstrap");
    serde_json::from_slice(&output.stdout).expect("bootstrap JSON")
}

/// Regression: `--require-understand` was a fail-open no-op. It gated only on
/// `/graphProvider/{sourceFound,cargoFound}` — both trivially true in a
/// checkout — and never once read the `understandAnything` block the very same
/// document publishes, so the flag answered `ok: true, missing: []` on a
/// machine reporting `skillFound: false, pluginFound: false`.
#[test]
fn require_understand_reports_missing_understand_anything() {
    let root = temp_dir();
    let pipeline = root.join("pipeline");
    let home = root.join("home");
    pipeline_scratch(&pipeline);
    fs::create_dir_all(&home).unwrap();

    let absent = doctor_bootstrap(&pipeline, &home);
    // Precondition: the checks the flag *used* to rely on are both satisfied,
    // so nothing but the Understand Anything check can fail this run.
    assert_eq!(
        absent["checks"]["graphProvider"]["sourceFound"],
        json!(true)
    );
    assert_eq!(absent["checks"]["graphProvider"]["cargoFound"], json!(true));
    assert_eq!(
        absent["checks"]["understandAnything"]["skillFound"],
        json!(false)
    );
    assert_eq!(
        absent["checks"]["understandAnything"]["pluginFound"],
        json!(false)
    );
    assert_eq!(absent["strict"]["requireUnderstand"], json!(true));

    // An installed skill satisfies the requirement, so the check cannot be a
    // blanket failure that merely looks like enforcement.
    let skill = home.join(".claude").join("skills").join("understand");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "# understand\n").unwrap();
    let present = doctor_bootstrap(&pipeline, &home);
    assert_eq!(
        present["checks"]["understandAnything"]["skillFound"],
        json!(true)
    );

    // A differential rather than a bare membership check: the scratch root is
    // deliberately incomplete in other ways (no pipeline script, no config),
    // so the honest claim is that installing Understand Anything removes
    // exactly one entry and changes nothing else.
    let entries = |observation: &Value| {
        observation["missing"]
            .as_array()
            .expect("missing array")
            .iter()
            .map(|entry| entry.as_str().expect("missing entry").to_string())
            .collect::<Vec<_>>()
    };
    let (before, after) = (entries(&absent), entries(&present));
    let removed = before
        .iter()
        .filter(|entry| !after.contains(entry))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        removed,
        vec!["Understand Anything skill or plugin".to_string()],
        "installing the skill must clear exactly the understand requirement\n\
         before={before:?}\nafter={after:?}"
    );
    assert!(
        after.iter().all(|entry| before.contains(entry)),
        "installing the skill must not introduce new missing entries"
    );

    fs::remove_dir_all(root).ok();
}
