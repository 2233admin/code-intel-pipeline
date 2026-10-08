mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Project {
    root: PathBuf,
    repo: PathBuf,
    artifacts: PathBuf,
    absent_provider: PathBuf,
}

impl Project {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "code-intel-codegraph space-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("source repo");
        let artifacts = root.join("evidence artifacts");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("left")).unwrap();
        fs::create_dir_all(repo.join("right")).unwrap();
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"codegraph_contract\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "pub fn rust_leaf() -> u32 { 17 }\npub fn rust_caller() -> u32 { rust_leaf() }\n",
        )
        .unwrap();
        fs::write(
            repo.join("leaf.ts"),
            "export function graph_leaf(): number { return 17; }\n",
        )
        .unwrap();
        fs::write(
            repo.join("middle.ts"),
            "import { graph_leaf } from './leaf';\nexport function graph_caller(): number { return graph_leaf(); }\n",
        )
        .unwrap();
        fs::write(
            repo.join("middle.test.ts"),
            "import { graph_caller } from './middle';\nexport function graph_test(): number { return graph_caller(); }\n",
        )
        .unwrap();
        fs::write(
            repo.join("retired.ts"),
            "export function retired_symbol(): number { return 3; }\n",
        )
        .unwrap();
        for (directory, caller, number) in
            [("left", "left_caller", 1), ("right", "right_caller", 2)]
        {
            fs::write(
                repo.join(directory).join("duplicate.ts"),
                format!(
                    "export function duplicate_symbol(): number {{ return {number}; }}\nexport function {caller}(): number {{ return duplicate_symbol(); }}\n"
                ),
            )
            .unwrap();
        }
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["config", "user.name", "CodeGraph Contract"]);
        git(
            &repo,
            &["config", "user.email", "codegraph@example.invalid"],
        );
        git(&repo, &["config", "core.autocrlf", "false"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "contract project"]);
        let absent_provider = root.join("never-installed-codegraph.exe");
        assert!(absent_provider.is_absolute());
        assert!(!absent_provider.exists());
        Self {
            root,
            repo,
            artifacts,
            absent_provider,
        }
    }

    fn command(&self, operation: &str) -> Command {
        let mut command = common::cli_in(&self.root);
        command
            .env_remove("CODE_INTEL_CODEGRAPH_BIN")
            .args(["provider", "codegraph", operation, "--repo"])
            .arg(&self.repo)
            .arg("--artifact-root")
            .arg(&self.artifacts);
        command
    }

    fn absent(&self, operation: &str) -> Command {
        let mut command = self.command(operation);
        command.env("CODE_INTEL_CODEGRAPH_BIN", &self.absent_provider);
        command
    }

    fn run(&self, operation: &str, args: &[&str], expected: i32) -> Value {
        let output = self.command(operation).args(args).output().unwrap();
        envelope(output, expected)
    }

    fn query(&self, name: &str) -> Value {
        let value = self.run("query", &["--query", name, "--limit", "100"], 0);
        assert_observation(&value);
        value
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn envelope(output: Output, expected: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(value["schema"], "code-intel-codegraph-result.v1");
    assert_eq!(value["provider"]["id"], "codegraph.cli");
    assert_eq!(value["authority"], "advisory");
    assert_eq!(value["domainVerdict"], "unknown");
    assert_eq!(value["engineeringFacts"], json!([]));
    if expected != 0 {
        assert!(value["result"].is_null(), "{value:#}");
        assert!(value["admission"].is_null(), "{value:#}");
    }
    value
}

fn assert_observation(value: &Value) {
    assert_eq!(value["status"], "ok", "{value:#}");
    assert_eq!(value["completeness"], "partial");
    assert_eq!(value["scope"], json!(["."]));
    assert_eq!(value["cache"]["usable"], true);
    assert_eq!(value["cache"]["binding"], "current");
    let implementation = &value["provider"]["implementation"];
    assert!(!implementation["id"].as_str().unwrap().is_empty());
    assert!(!implementation["version"].as_str().unwrap().is_empty());
    assert_digest(&implementation["digest"]);
    assert!(Path::new(value["provider"]["executable"].as_str().unwrap()).is_absolute());
    let admission = &value["admission"];
    assert_eq!(admission["status"], "admitted", "{value:#}");
    assert_eq!(admission["domainVerdict"], "unknown");
    assert_eq!(admission["engineeringFacts"], json!([]));
    let evidence = &admission["evidence"];
    assert_eq!(evidence["completeness"], "partial");
    assert_eq!(evidence["claimedComplete"], false);
    assert_eq!(evidence["provider"]["id"], "codegraph.cli");
    assert_eq!(evidence["provider"]["implementation"], *implementation);
    assert_digest(&value["snapshot"]["identity"]);
    assert_eq!(
        evidence["consumedSnapshotIdentity"],
        value["snapshot"]["identity"]
    );
    assert_eq!(
        evidence["payload"]["artifactSchema"],
        "code-intel-evidence-payload.v1"
    );
    assert_digest(&evidence["payload"]["sha256"]);
    assert!(!evidence["payload"]["path"].as_str().unwrap().is_empty());
    let stdout = value["rawOutput"]["stdout"].as_str().unwrap();
    if value["rawOutput"]["format"] == "json" {
        let raw: Value = serde_json::from_str(stdout).unwrap();
        assert_eq!(value["result"], raw);
    } else {
        assert_eq!(value["result"], stdout);
    }
}

fn assert_digest(value: &Value) {
    let digest = value.as_str().unwrap();
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

fn query_has(value: &Value, name: &str, file: &str) -> bool {
    value["result"].as_array().unwrap().iter().any(|entry| {
        entry["node"]["name"] == name
            && entry["node"]["filePath"]
                .as_str()
                .unwrap()
                .replace('\\', "/")
                == file
    })
}

fn nodes_have(value: &Value, field: &str, name: &str) -> bool {
    value["result"][field]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["name"] == name)
}

fn snapshot(project: &Project) -> Value {
    let output = common::cli()
        .args(["snapshot", "identity", "--repo"])
        .arg(&project.repo)
        .args(["--working-tree-policy", "explicit_overlay", "--scope", "."])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["snapshot"].clone()
}

#[test]
fn invalid_public_arguments_are_rejected_before_provider_launch() {
    let project = Project::new();
    for (operation, args) in [
        ("not-an-operation", vec![]),
        ("query", vec![]),
        ("query", vec!["--query"]),
        ("query", vec!["--query", "graph_leaf", "--unknown"]),
        ("query", vec!["--query", "graph_leaf", "--limit", "0"]),
        ("query", vec!["--query", "graph_leaf", "--limit", "NaN"]),
        ("impact", vec!["--query", "graph_leaf", "--depth", "0"]),
        ("impact", vec!["--query", "graph_leaf", "--depth", "-1"]),
        ("affected", vec![]),
        ("index", vec!["--changed", "leaf.ts"]),
    ] {
        let value = envelope(project.absent(operation).args(&args).output().unwrap(), 64);
        assert_eq!(value["status"], "rejected", "{operation} {args:?}");
    }
    let output = common::cli()
        .env("CODE_INTEL_CODEGRAPH_BIN", &project.absent_provider)
        .args(["provider", "codegraph", "index"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(!project.repo.join(".codegraph").exists());
}

#[test]
fn absolute_and_traversing_source_paths_fail_closed_without_a_provider() {
    let project = Project::new();
    let absolute = project.root.join("outside.ts");
    for path in [
        "../outside.ts",
        "src/../../outside.ts",
        "..\\outside.ts",
        "src\\..\\..\\outside.ts",
        absolute.to_str().unwrap(),
    ] {
        for (operation, flag, query) in [
            ("affected", "--changed", None),
            ("node", "--file", None),
            ("callers", "--file", Some("graph_leaf")),
        ] {
            let mut command = project.absent(operation);
            command.arg(flag).arg(path);
            if let Some(query) = query {
                command.args(["--query", query]);
            }
            let value = envelope(command.output().unwrap(), 64);
            assert_eq!(value["status"], "rejected");
        }
    }
    assert!(!absolute.exists());
    assert!(!project.repo.join(".codegraph").exists());
}

#[test]
fn missing_provider_is_unavailable_not_an_empty_or_complete_graph() {
    let project = Project::new();
    for (operation, args) in [
        ("index", vec![]),
        ("sync", vec![]),
        ("status", vec![]),
        ("query", vec!["--query", "graph_leaf"]),
        ("explore", vec!["--query", "graph_leaf"]),
        ("node", vec!["--file", "leaf.ts"]),
        ("callers", vec!["--query", "graph_leaf"]),
        ("callees", vec!["--query", "graph_caller"]),
        ("impact", vec!["--query", "graph_leaf"]),
        ("affected", vec!["--changed", "leaf.ts"]),
        ("files", vec![]),
    ] {
        let value = envelope(project.absent(operation).args(&args).output().unwrap(), 69);
        assert_eq!(value["status"], "unavailable", "{operation}: {value:#}");
        assert_ne!(value["completeness"], "complete");
        assert_ne!(value["cache"]["usable"], true);
    }
    assert!(!project.repo.join(".codegraph").exists());
}

#[test]
fn artifact_root_cannot_self_index_even_through_parent_components() {
    let project = Project::new();
    for root in [
        project.repo.clone(),
        project.repo.join("evidence"),
        project
            .root
            .join("evidence artifacts/../source repo/evidence"),
    ] {
        let output = common::cli()
            .env("CODE_INTEL_CODEGRAPH_BIN", &project.absent_provider)
            .args(["provider", "codegraph", "index", "--repo"])
            .arg(&project.repo)
            .arg("--artifact-root")
            .arg(&root)
            .output()
            .unwrap();
        let value = envelope(output, 65);
        assert_eq!(value["status"], "rejected");
    }
    assert!(!project.repo.join("evidence").exists());
    assert!(!project.repo.join(".codegraph").exists());
}

#[test]
#[ignore = "requires official CodeGraph >=1.6.2 on PATH; CI installs and runs --ignored"]
fn real_upstream_edges_source_and_add_change_delete_sync_are_snapshot_bound() {
    let project = Project::new();
    let index = project.run("index", &[], 0);
    assert_eq!(index["status"], "ok");
    let before_index = snapshot(&project);
    assert_eq!(index["snapshot"], before_index);
    for file in [".gitignore", "AGENTS.md", "CLAUDE.md"] {
        assert!(!project.repo.join(file).exists(), "index created {file}");
    }
    let leaf = project.query("graph_leaf");
    assert!(query_has(&leaf, "graph_leaf", "leaf.ts"), "{leaf:#}");
    let rust = project.query("rust_leaf");
    assert!(query_has(&rust, "rust_leaf", "src/lib.rs"), "{rust:#}");
    let retired = project.query("retired_symbol");
    assert!(query_has(&retired, "retired_symbol", "retired.ts"));
    let callers = project.run("callers", &["--query", "graph_leaf"], 0);
    assert_observation(&callers);
    assert!(
        nodes_have(&callers, "callers", "graph_caller"),
        "{callers:#}"
    );
    let edges = callers["result"]["definitions"][0]["edges"]
        .as_array()
        .unwrap();
    assert!(
        edges.iter().any(|edge| {
            edge["kind"] == "calls"
                && edge["metadata"]["confidence"].is_number()
                && edge["metadata"]["resolvedBy"].is_string()
        }),
        "{callers:#}"
    );
    let callees = project.run("callees", &["--query", "graph_caller"], 0);
    assert_observation(&callees);
    assert!(nodes_have(&callees, "callees", "graph_leaf"));
    let impact = project.run("impact", &["--query", "graph_leaf", "--depth", "3"], 0);
    assert_observation(&impact);
    assert!(
        nodes_have(&impact, "affected", "graph_caller"),
        "{impact:#}"
    );
    let affected = project.run("affected", &["--changed", "leaf.ts"], 0);
    assert_observation(&affected);
    assert_eq!(affected["result"]["changedFiles"], json!(["leaf.ts"]));
    assert!(
        affected["result"]["affectedTests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "middle.test.ts"),
        "{affected:#}"
    );
    for (operation, args) in [
        ("node", vec!["--file", "leaf.ts"]),
        ("explore", vec!["--query", "graph_leaf"]),
    ] {
        let value = project.run(operation, &args, 0);
        assert_observation(&value);
        assert!(value["result"].as_str().unwrap().contains("return 17"));
    }

    fs::write(
        project.repo.join("leaf.ts"),
        "export function updated_leaf(): number { return 41; }\n",
    )
    .unwrap();
    for (operation, args) in [
        ("query", vec!["--query", "graph_leaf"]),
        ("callers", vec!["--query", "graph_leaf"]),
        ("callees", vec!["--query", "graph_caller"]),
        ("impact", vec!["--query", "graph_leaf"]),
        ("affected", vec!["--changed", "leaf.ts"]),
        ("node", vec!["--file", "leaf.ts"]),
        ("explore", vec!["--query", "graph_leaf"]),
        ("files", vec![]),
    ] {
        let rejected = project.run(operation, &args, 65);
        assert_eq!(rejected["status"], "rejected");
        assert_eq!(rejected["cache"]["binding"], "stale");
        assert_eq!(rejected["cache"]["usable"], false);
    }
    let stale_status = project.run("status", &[], 0);
    assert_eq!(stale_status["cache"]["usable"], false);
    assert_eq!(stale_status["cache"]["binding"], "stale");

    fs::write(
        project.repo.join("middle.ts"),
        "import { updated_leaf } from './leaf';\nexport function updated_caller(): number { return updated_leaf(); }\n",
    )
    .unwrap();
    fs::write(
        project.repo.join("middle.test.ts"),
        "import { updated_caller } from './middle';\nexport function updated_test(): number { return updated_caller(); }\n",
    )
    .unwrap();
    fs::write(
        project.repo.join("added.ts"),
        "import { updated_leaf } from './leaf';\nexport function added_caller(): number { return updated_leaf(); }\n",
    )
    .unwrap();
    fs::write(
        project.repo.join("src/lib.rs"),
        "pub fn updated_rust_leaf() -> u32 { 41 }\npub fn updated_rust_caller() -> u32 { updated_rust_leaf() }\n",
    )
    .unwrap();
    fs::remove_file(project.repo.join("retired.ts")).unwrap();
    let before_sync = snapshot(&project);
    assert_ne!(before_sync["identity"], before_index["identity"]);
    let sync = project.run("sync", &[], 0);
    assert_eq!(sync["status"], "ok");
    assert_eq!(sync["snapshot"], before_sync);
    assert_eq!(snapshot(&project), before_sync);
    assert!(query_has(
        &project.query("updated_leaf"),
        "updated_leaf",
        "leaf.ts"
    ));
    assert!(query_has(
        &project.query("added_caller"),
        "added_caller",
        "added.ts"
    ));
    assert!(query_has(
        &project.query("updated_rust_leaf"),
        "updated_rust_leaf",
        "src/lib.rs"
    ));
    for removed in ["graph_leaf", "graph_caller", "retired_symbol", "rust_leaf"] {
        let query = project.query(removed);
        assert!(
            !query["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["node"]["name"] == removed),
            "{removed} survived sync: {query:#}"
        );
    }
    let callers = project.run("callers", &["--query", "updated_leaf"], 0);
    assert_observation(&callers);
    assert!(nodes_have(&callers, "callers", "updated_caller"));
    assert!(nodes_have(&callers, "callers", "added_caller"));
    assert!(!nodes_have(&callers, "callers", "graph_caller"));
    let impact = project.run("impact", &["--query", "updated_leaf", "--depth", "3"], 0);
    assert_observation(&impact);
    assert!(nodes_have(&impact, "affected", "updated_caller"));
    assert!(nodes_have(&impact, "affected", "added_caller"));
    let affected = project.run(
        "affected",
        &["--changed", "leaf.ts", "--changed", "added.ts"],
        0,
    );
    assert_observation(&affected);
    assert!(affected["result"]["affectedTests"]
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file == "middle.test.ts"));
    for file in ["leaf.ts", "src/lib.rs"] {
        let current = project.run("node", &["--file", file], 0);
        assert_observation(&current);
        let source = current["result"].as_str().unwrap();
        assert!(source.contains("41"), "{current:#}");
        assert!(!source.contains("return 17"));
    }
    let current = project.run("explore", &["--query", "updated_leaf"], 0);
    assert_observation(&current);
    assert!(current["result"].as_str().unwrap().contains("return 41"));
    let files = project.run("files", &[], 0);
    assert_observation(&files);
    let files = files["result"].as_array().unwrap();
    assert!(files.iter().any(|file| file["path"] == "added.ts"));
    assert!(!files.iter().any(|file| file["path"] == "retired.ts"));
}

#[test]
#[ignore = "requires official CodeGraph >=1.6.2 on PATH; CI installs and runs --ignored"]
fn real_upstream_duplicate_definitions_and_truncation_remain_explicit() {
    let project = Project::new();
    project.run("index", &[], 0);
    let query = project.query("duplicate_symbol");
    assert!(query_has(&query, "duplicate_symbol", "left/duplicate.ts"));
    assert!(query_has(&query, "duplicate_symbol", "right/duplicate.ts"));
    let callers = project.run("callers", &["--query", "duplicate_symbol"], 0);
    assert_observation(&callers);
    assert_eq!(callers["result"]["ambiguous"], true);
    assert_eq!(callers["result"]["aggregation"], "union");
    assert!(callers["result"]["definitions"].as_array().unwrap().len() >= 2);
    assert!(nodes_have(&callers, "callers", "left_caller"));
    assert!(nodes_have(&callers, "callers", "right_caller"));
    let narrowed = project.run(
        "callers",
        &["--query", "duplicate_symbol", "--file", "left/duplicate.ts"],
        0,
    );
    assert_observation(&narrowed);
    assert_eq!(narrowed["result"]["ambiguous"], false);
    assert_eq!(narrowed["result"]["aggregation"], "definition");
    assert!(nodes_have(&narrowed, "callers", "left_caller"));
    assert!(!nodes_have(&narrowed, "callers", "right_caller"));
    let limited = project.run(
        "callers",
        &["--query", "duplicate_symbol", "--limit", "1"],
        0,
    );
    assert_observation(&limited);
    assert_eq!(limited["result"]["limit"], 1);
    assert_eq!(limited["result"]["truncated"], true);
    assert!(limited["result"]["total"].as_u64().unwrap() >= 2);
    assert_eq!(limited["result"]["callers"].as_array().unwrap().len(), 1);
    let impact = project.run(
        "impact",
        &["--query", "duplicate_symbol", "--depth", "2"],
        0,
    );
    assert_observation(&impact);
    assert_eq!(impact["result"]["ambiguous"], true);
    assert!(nodes_have(&impact, "affected", "left_caller"));
    assert!(nodes_have(&impact, "affected", "right_caller"));
}

#[test]
#[ignore = "requires official CodeGraph >=1.6.2 on PATH; CI installs and runs --ignored"]
fn real_upstream_queries_do_not_index_unbound_sources_or_inject_flags() {
    let project = Project::new();
    let unbound = project.run("query", &["--query", "graph_leaf"], 65);
    assert_eq!(unbound["cache"]["usable"], false);
    assert!(!project.repo.join(".codegraph").exists());
    project.run("index", &[], 0);
    let before = snapshot(&project);
    for query in ["--help", "--version", "--path", "--limit 1"] {
        let value = project.query(query);
        assert!(
            value["result"].is_array(),
            "query became an option: {query}"
        );
    }
    let mut command = project.command("explore");
    command.args(["--query", "--help"]);
    let value = envelope(command.output().unwrap(), 0);
    assert_observation(&value);
    assert_eq!(snapshot(&project), before);
    assert!(query_has(
        &project.query("graph_leaf"),
        "graph_leaf",
        "leaf.ts"
    ));

    // A database is not a Pipeline snapshot binding, even if CodeGraph can read it.
    fs::remove_file(project.repo.join(".codegraph/code-intel-snapshot.json")).unwrap();
    let unbound = project.run("callers", &["--query", "graph_leaf"], 65);
    assert_eq!(unbound["cache"]["binding"], "unbound");
    assert_eq!(unbound["cache"]["usable"], false);
    let unbound_sync = project.run("sync", &[], 65);
    assert_eq!(unbound_sync["cache"]["usable"], false);
    project.run("index", &[], 0);
    let rebound = project.run("callers", &["--query", "graph_leaf"], 0);
    assert_observation(&rebound);
    assert!(nodes_have(&rebound, "callers", "graph_caller"));
    fs::write(
        project.repo.join(".codegraph/code-intel-snapshot.json"),
        b"{not-valid-json",
    )
    .unwrap();
    let malformed = project.run("query", &["--query", "graph_leaf"], 65);
    assert_eq!(malformed["cache"]["binding"], "invalid");
    assert_eq!(malformed["cache"]["usable"], false);
}

#[test]
#[ignore = "requires official CodeGraph >=1.6.2 on PATH; CI installs and runs --ignored"]
fn real_upstream_forced_ignored_sources_cannot_be_bound_or_read_as_snapshot_evidence() {
    let project = Project::new();
    let ignore = b"ignored.ts\n";
    let source = b"export function outside_snapshot(): number { return 999; }\n";
    // Upstream's `include` overrides gitignore for regular first-party source.
    // `includeIgnored` instead controls embedded git repositories.
    let forced_config = b"{\"include\":[\"ignored.ts\"]}\n";
    fs::write(project.repo.join(".gitignore"), ignore).unwrap();
    fs::write(project.repo.join("ignored.ts"), source).unwrap();
    fs::write(project.repo.join("codegraph.json"), forced_config).unwrap();

    let rejected = project.run("index", &[], 65);
    assert_eq!(rejected["status"], "rejected");
    assert_ne!(rejected["cache"]["usable"], true);
    assert_eq!(rejected["rawOutput"]["exitCode"], 0);
    // The concrete offending path is reported by the post-index inventory
    // audit, not by scanning the configuration's text for suspicious options.
    assert!(
        rejected["diagnostics"].to_string().contains("ignored.ts"),
        "{rejected:#}"
    );
    assert!(!project
        .repo
        .join(".codegraph/code-intel-snapshot.json")
        .exists());
    assert_eq!(fs::read(project.repo.join(".gitignore")).unwrap(), ignore);
    assert_eq!(
        fs::read(project.repo.join("codegraph.json")).unwrap(),
        forced_config
    );
    assert_eq!(fs::read(project.repo.join("ignored.ts")).unwrap(), source);

    let allowed_config = b"{\"include\":[]}\n";
    fs::write(project.repo.join("codegraph.json"), allowed_config).unwrap();
    let index = project.run("index", &[], 0);
    assert_eq!(index["status"], "ok");
    let query = project.query("graph_leaf");
    assert!(query_has(&query, "graph_leaf", "leaf.ts"));
    let rejected = project.run("node", &["--file", "ignored.ts"], 65);
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(fs::read(project.repo.join(".gitignore")).unwrap(), ignore);
    assert_eq!(
        fs::read(project.repo.join("codegraph.json")).unwrap(),
        allowed_config
    );
    assert_eq!(fs::read(project.repo.join("ignored.ts")).unwrap(), source);
}
