use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-b08-{label}-{}-{nonce}",
            std::process::id()
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

fn tool_fixture(root: &Path) -> PathBuf {
    let bin = root.join("dag-tools");
    fs::create_dir_all(&bin).unwrap();
    #[cfg(windows)]
    {
        for name in ["rg", "git", "python", "repowise"] {
            fs::write(
                bin.join(format!("{name}.cmd")),
                "@echo off\r\nexit /b 0\r\n",
            )
            .unwrap();
        }
        fs::write(
            bin.join("sentrux.cmd"),
            "@echo off\r\necho Enforce architectural rules\r\necho Tier: pro\r\nexit /b 0\r\n",
        )
        .unwrap();
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["rg", "git", "python", "repowise", "sentrux"] {
            let path = bin.join(name);
            let content = if name == "sentrux" {
                "#!/bin/sh\necho 'Enforce architectural rules'\necho 'Tier: pro'\nexit 0\n"
            } else {
                "#!/bin/sh\nexit 0\n"
            };
            fs::write(&path, content).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    bin
}

fn run_with_expected_code(repo: &Path, out: &Path, expected_code: i32) -> Value {
    let tools = tool_fixture(out.parent().unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["run", "dag-coordinate", "--repo"])
        .arg(repo)
        .arg("--out")
        .arg(out)
        .arg("--doctor-tool-path-prefix")
        .arg(tools)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn run(repo: &Path, out: &Path) -> Value {
    run_with_expected_code(repo, out, 0)
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn run_legacy(repo: &Path, root: &Path) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = root.join("pipeline.config.json");
    fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({
            "artifactRoot":"",
            "repowiseWorkspaceRoot":"",
            "codeEvidence":{
                "enabled":true,
                "nativeMinimal":true,
                "adapters":{"cocoindex-code":{"enabled":false,"required":false,"command":"ccc"}}
            },
            "inventoryExclude":[],
            "repos":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let artifacts = root.join("legacy-artifacts");
    let output = Command::new("pwsh")
        .arg("-NoProfile")
        .arg("-File")
        .arg(project_root.join("run-code-intel.ps1"))
        .arg("-RepoPath")
        .arg(repo)
        .arg("-Config")
        .arg(config)
        .arg("-Mode")
        .arg("lite")
        .arg("-ArtifactRoot")
        .arg(&artifacts)
        .args([
            "-SkipRepowise",
            "-SkipSentrux",
            "-SkipGitHubResearch",
            "-SkipRepomix",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "legacy stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let repo_runs = artifacts.join(repo.file_name().unwrap());
    fs::read_dir(repo_runs)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("code-evidence/merged/full/files.json").is_file())
        .expect("legacy producer must publish code evidence")
}

fn canonicalize_artifact(relative: &str, mut value: Value) -> Value {
    let (field, key): (&str, fn(&Value) -> String) = match relative {
        "code-evidence/merged/full/files.json" => ("files", |item| {
            item["path"].as_str().unwrap_or_default().to_string()
        }),
        "code-evidence/merged/full/symbols.json" => ("symbols", |item| {
            format!(
                "{}\0{:020}\0{}\0{}",
                item["file"].as_str().unwrap_or_default(),
                item["startLine"].as_u64().unwrap_or_default(),
                item["kind"].as_str().unwrap_or_default(),
                item["name"].as_str().unwrap_or_default()
            )
        }),
        "code-evidence/merged/full/chunks.json" => ("chunks", |item| {
            format!(
                "{}\0{:020}\0{:020}\0{}",
                item["file"].as_str().unwrap_or_default(),
                item["startLine"].as_u64().unwrap_or_default(),
                item["endLine"].as_u64().unwrap_or_default(),
                item["id"].as_str().unwrap_or_default()
            )
        }),
        "code-evidence/merged/full/symbol-chunks.json" => ("mappings", |item| {
            format!(
                "{}\0{}",
                item["symbolId"].as_str().unwrap_or_default(),
                item["chunkId"].as_str().unwrap_or_default()
            )
        }),
        "code-evidence/merged/full/imports.json" => ("imports", |item| {
            format!(
                "{}\0{:020}\0{}",
                item["file"].as_str().unwrap_or_default(),
                item["line"].as_u64().unwrap_or_default(),
                item["target"].as_str().unwrap_or_default()
            )
        }),
        "code-evidence/merged/agent/ranking.json" => ("files", |item| {
            item["path"].as_str().unwrap_or_default().to_string()
        }),
        _ => return value,
    };
    if let Some(items) = value.get_mut(field).and_then(Value::as_array_mut) {
        items.sort_by_key(key);
    }
    value
}

fn reverse_semantic_arrays(relative: &str, mut value: Value) -> Value {
    let field = match relative {
        "code-evidence/merged/full/files.json" | "code-evidence/merged/agent/ranking.json" => {
            "files"
        }
        "code-evidence/merged/full/symbols.json" => "symbols",
        "code-evidence/merged/full/chunks.json" => "chunks",
        "code-evidence/merged/full/symbol-chunks.json" => "mappings",
        "code-evidence/merged/full/imports.json" => "imports",
        _ => return value,
    };
    if let Some(items) = value.get_mut(field).and_then(Value::as_array_mut) {
        items.reverse();
    }
    value
}

#[test]
fn native_atom_preserves_representative_v1_artifacts_through_a01_a03_a09() {
    let temp = Temp::new("parity");
    let repo = temp.0.join("repo");
    let out = temp.0.join("run");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("index.js"),
        "function greet(name) {\n  return `hello ${name}`;\n}\n\nmodule.exports = { greet };\n",
    )
    .unwrap();
    fs::write(
        repo.join("index.test.js"),
        "const { greet } = require(\"./index\");\n\ntest(\"greet\", () => greet(\"Ada\"));\n",
    )
    .unwrap();

    let manifest = run(&repo, &out);
    assert_eq!(
        manifest["nodes"]["evidence.native-code"]["status"], "succeeded",
        "{}",
        manifest["nodes"]["evidence.native-code"]
    );

    let result = read_json(out.join("evidence.native-code.result.json"));
    assert_eq!(result["schema"], "code-intel-capability-result.v1");
    assert_eq!(result["capability"], "evidence.native-code");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["verdict"], "pass");
    assert_eq!(
        result["observedEffects"],
        serde_json::json!(["repo_read", "local_write"])
    );
    assert_eq!(result["artifacts"].as_array().unwrap().len(), 8);
    for artifact in result["artifacts"].as_array().unwrap() {
        assert_eq!(artifact["schema"], "code-intel-artifact-ref.v1");
        assert_eq!(
            artifact["consumedSnapshotIdentity"],
            result["snapshotIdentity"]
        );
        assert!(artifact["sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64));
    }

    let root = out.join("evidence.native-code/code-evidence");
    let files = read_json(root.join("merged/full/files.json"));
    let symbols = read_json(root.join("merged/full/symbols.json"));
    let imports = read_json(root.join("merged/full/imports.json"));
    let scorecard = read_json(root.join("merged/scorecard.json"));
    let ranking = read_json(root.join("merged/agent/ranking.json"));
    assert_eq!(files["schema"], "code-evidence-files.v1");
    assert_eq!(symbols["schema"], "code-evidence-symbols.v1");
    assert!(symbols["symbols"].as_array().unwrap().iter().any(|symbol| {
        symbol["name"] == "greet"
            && symbol["kind"] == "function"
            && symbol["source"] == "native-minimal"
    }));
    assert!(imports["imports"]
        .as_array()
        .unwrap()
        .iter()
        .any(|import| import["target"] == "./index"));
    assert_eq!(scorecard["schema"], "code-evidence-scorecard.v1");
    assert_eq!(scorecard["metrics"]["files"], 2);
    assert_eq!(scorecard["metrics"]["chunks"], 2);
    assert_eq!(ranking["schema"], "agent-code-slice-ranking.v1");
    assert!(ranking["files"].as_array().unwrap().iter().any(|file| {
        file["path"] == "index.js"
            && file["reasons"]
                .as_array()
                .unwrap()
                .contains(&Value::from("entrypoint"))
    }));
    let agent_index = fs::read_to_string(root.join("merged/agent/index.md")).unwrap();
    assert!(agent_index.contains("Call graph precision: unknown"));
    assert!(root
        .join("merged/agent/slices/native-retrieval.md")
        .is_file());
}

#[test]
fn unsupported_language_is_explicit_unknown_without_relationship_fabrication() {
    let temp = Temp::new("unsupported");
    let repo = temp.0.join("repo");
    let out = temp.0.join("run");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("mystery.zig"), "pub fn hidden() void {}\n").unwrap();

    run(&repo, &out);
    let root = out.join("evidence.native-code/code-evidence");
    let coverage = read_json(root.join("coverage.json"));
    assert_eq!(coverage["schema"], "code-evidence-coverage.v1");
    assert_eq!(coverage["parserKind"], "line-heuristic");
    assert_eq!(coverage["relationshipPrecision"], "unknown");
    assert_eq!(coverage["callGraph"], "unknown");
    assert!(coverage["unsupportedFiles"]
        .as_array()
        .unwrap()
        .contains(&Value::from("mystery.zig")));
    assert!(!root.join("full/call-graph.json").exists());
    let symbols = read_json(root.join("merged/full/symbols.json"));
    assert!(symbols["symbols"].as_array().unwrap().is_empty());
}

#[test]
fn binary_non_source_is_preserved_as_explicit_unsupported_without_failing_the_run() {
    let temp = Temp::new("binary-unsupported");
    let repo = temp.0.join("repo");
    let out = temp.0.join("run");
    fs::create_dir_all(repo.join("assets")).unwrap();
    fs::write(repo.join("src.rs"), "pub fn visible() {}\n").unwrap();
    fs::write(repo.join("assets/logo.png"), [0x89, 0x50, 0x4e, 0x47, 0xff]).unwrap();

    let manifest = run(&repo, &out);
    assert_eq!(manifest["outcome"], "completed");
    assert_eq!(
        manifest["nodes"]["evidence.native-code"]["status"],
        "succeeded"
    );
    let evidence = out.join("evidence.native-code/code-evidence");
    let coverage = read_json(evidence.join("coverage.json"));
    assert!(coverage["unsupportedFiles"]
        .as_array()
        .unwrap()
        .contains(&Value::from("assets/logo.png")));
    let files = read_json(evidence.join("merged/full/files.json"));
    let binary = files["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "assets/logo.png")
        .expect("binary inventory entry");
    assert_eq!(binary["language"], "text");
    assert_eq!(binary["lines"], 0);
}

#[test]
fn non_utf8_supported_source_remains_a_visible_contract_failure() {
    let temp = Temp::new("binary-supported");
    let repo = temp.0.join("repo");
    let out = temp.0.join("run");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("broken.rs"), [0xff, 0xfe, 0xfd]).unwrap();

    let manifest = run_with_expected_code(&repo, &out, 70);
    assert_eq!(manifest["outcome"], "process_failed");
    assert_eq!(
        manifest["nodes"]["evidence.native-code"]["status"],
        "process_failed"
    );
    assert!(manifest["nodes"]["evidence.native-code"]["diagnostic"]
        .as_str()
        .unwrap()
        .contains("supported source file is not UTF-8 text: broken.rs"));
}

#[test]
fn labeled_multilingual_corpus_quantifies_native_symbol_precision_recall_and_coverage() {
    let started = std::time::Instant::now();
    let corpus: Value = read_json(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("orchestration/internalization/fixtures/r06-native-labeled-corpus.json"),
    );
    let temp = Temp::new("labeled-corpus");
    let repo = temp.0.join("repo");
    let out = temp.0.join("run");
    fs::create_dir_all(&repo).unwrap();
    for sample in corpus["samples"].as_array().unwrap() {
        fs::write(
            repo.join(sample["path"].as_str().unwrap()),
            sample["content"].as_str().unwrap(),
        )
        .unwrap();
    }

    run(&repo, &out);
    let evidence = out.join("evidence.native-code/code-evidence");
    let symbols = read_json(evidence.join("merged/full/symbols.json"));
    let predicted_files: HashSet<&str> = symbols["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|symbol| symbol["file"].as_str())
        .collect();
    let coverage = read_json(evidence.join("coverage.json"));
    let unsupported: HashSet<&str> = coverage["unsupportedFiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();

    let mut tp = 0;
    let mut fp = 0;
    let mut fn_ = 0;
    let mut tn = 0;
    for sample in corpus["samples"].as_array().unwrap() {
        let path = sample["path"].as_str().unwrap();
        let expected = sample["symbolPresent"].as_bool().unwrap();
        let predicted = predicted_files.contains(path);
        match (expected, predicted) {
            (true, true) => tp += 1,
            (false, true) => fp += 1,
            (true, false) => fn_ += 1,
            (false, false) => tn += 1,
        }
    }
    assert_eq!((tp, fp, fn_, tn), (6, 2, 2, 2));
    assert_eq!(unsupported.len(), 2);
    assert_eq!(corpus["samples"].as_array().unwrap().len(), 12);
    assert_eq!(tp as f64 / (tp + fp) as f64, 0.75);
    assert_eq!(tp as f64 / (tp + fn_) as f64, 0.75);
    assert!(started.elapsed().as_millis() < 120_000);
}

#[test]
fn a01_a09_artifacts_match_the_real_legacy_producer_on_the_same_fixture() {
    let temp = Temp::new("legacy-parity");
    let repo = temp.0.join("repo");
    let out = temp.0.join("native-run");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("main.js"),
        "import first from \"./first.js\";\nimport second from \"./second.js\";\nfunction main() {}\nconst helper = () => {};\n",
    )
    .unwrap();
    fs::write(
        repo.join("single.js"),
        "const dependency = require(\"./dependency.js\");\nfunction single() {}\n",
    )
    .unwrap();
    fs::write(repo.join("plain.txt"), "plain text only\n").unwrap();

    run(&repo, &out);
    let native = out.join("evidence.native-code");
    let comparable = [
        "code-evidence/merged/full/files.json",
        "code-evidence/merged/full/symbols.json",
        "code-evidence/merged/full/chunks.json",
        "code-evidence/merged/full/symbol-chunks.json",
        "code-evidence/merged/full/imports.json",
        "code-evidence/merged/scorecard.json",
        "code-evidence/merged/agent/ranking.json",
        "code-evidence/adapters/cocoindex-code/outcome.json",
    ];
    let mut raw_file_orders = HashSet::new();
    let mut first_raw = Vec::new();
    for iteration in 0..10 {
        let legacy_root = temp.0.join(format!("legacy-{iteration}"));
        let legacy = run_legacy(&repo, &legacy_root);
        for relative in comparable {
            let legacy_raw = read_json(legacy.join(relative));
            let native_value = read_json(native.join(relative));
            assert_eq!(
                canonicalize_artifact(relative, native_value.clone()),
                native_value,
                "native artifact is not in canonical order: {relative}"
            );
            assert_eq!(
                canonicalize_artifact(relative, legacy_raw.clone()),
                native_value,
                "legacy/A01-A09 canonical artifact drift on iteration {iteration}: {relative}"
            );
            if iteration == 0 {
                first_raw.push((relative, legacy_raw));
            }
        }
        let raw_files = read_json(legacy.join("code-evidence/merged/full/files.json"));
        raw_file_orders.insert(
            raw_files["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| file["path"].as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
        );
    }
    assert!(!raw_file_orders.is_empty());
    for (relative, raw) in first_raw {
        let permuted = reverse_semantic_arrays(relative, raw.clone());
        assert_eq!(
            canonicalize_artifact(relative, raw),
            canonicalize_artifact(relative, permuted),
            "canonical comparison must ignore non-semantic traversal order: {relative}"
        );
    }

    let native_ranking = read_json(native.join("code-evidence/merged/agent/ranking.json"));
    let ranked_files = native_ranking["files"].as_array().unwrap();
    assert_eq!(
        ranked_files
            .iter()
            .map(|file| file["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["main.js", "plain.txt", "single.js"]
    );
    assert_eq!(ranked_files[0]["score"], 60);
    assert_eq!(
        ranked_files[0]["reasons"],
        serde_json::json!(["entrypoint", "symbols", "imports"])
    );
    assert_eq!(ranked_files[0]["symbols"].as_array().unwrap().len(), 2);
    assert_eq!(ranked_files[0]["imports"].as_array().unwrap().len(), 2);
    assert!(ranked_files[1]["symbols"].is_null());
    assert!(ranked_files[1]["imports"].is_null());
    assert_eq!(ranked_files[1]["score"], 1);
    assert_eq!(ranked_files[1]["reasons"], serde_json::json!(["inventory"]));
    assert!(ranked_files[2]["symbols"].is_string());
    assert!(ranked_files[2]["imports"].is_string());
    assert_eq!(ranked_files[2]["score"], 10);
    assert_eq!(
        ranked_files[2]["reasons"],
        serde_json::json!(["symbols", "imports"])
    );
    let coverage = read_json(native.join("code-evidence/coverage.json"));
    assert_eq!(coverage["relationshipPrecision"], "unknown");
    assert_eq!(coverage["callGraph"], "unknown");
}
