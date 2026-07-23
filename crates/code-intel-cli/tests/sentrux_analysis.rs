use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../src/sentrux_analysis.rs"]
mod sentrux_analysis;

struct Fixture {
    root: PathBuf,
}

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "code-intel-sentrux-analysis-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, content).expect("write fixture");
    }

    fn write_bytes(&self, relative: &str, content: &[u8]) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, content).expect("write fixture bytes");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn file<'a>(snapshot: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    snapshot["file_details"]
        .as_array()
        .expect("file_details array")
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("missing {path}"))
}

fn module<'a>(snapshot: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    snapshot["modules"]
        .as_array()
        .expect("modules array")
        .iter()
        .find(|module| module["name"] == name)
        .unwrap_or_else(|| panic!("missing module {name}"))
}

#[test]
fn dsm_snapshot_preserves_contract_and_excludes_non_governed_source() {
    let fixture = Fixture::new();
    fixture.write(
        "src/lib.rs",
        "// choice\npub async fn choose(value: i32) -> i32 {\n    if value > 0 && value < 10 { value } else { 0 }\n}\n",
    );
    fixture.write(
        "tests/test_lib.rs",
        "fn choose_contract() { assert_eq!(1, 1); }\n",
    );
    fixture.write(
        "tools/ignored.ps1",
        "function Invoke-Ignored { if ($true) { 1 } }\n",
    );
    fixture.write("target/ignored.rs", "fn ignored() {}\n");
    fixture.write(".gitignore", "work/\n");
    fixture.write("work/generated.rs", "fn generated() {}\n");
    fixture.write("README.md", "not source\n");

    let snapshot = sentrux_analysis::analyze(&fixture.root).expect("native DSM analysis");

    assert_eq!(snapshot["tool"], "dsm");
    assert_eq!(snapshot["default_color_mode"], "Risk");
    assert_eq!(snapshot["color_modes"].as_array().unwrap().len(), 9);
    assert_eq!(snapshot["scope"]["included_files"], 2);
    assert_eq!(snapshot["scope"]["excluded_files"], 3);
    assert!(snapshot["scope"]["excluded_by_reason"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["reason"] == "repository_ignored" && entry["files"] == 1));
    assert_eq!(snapshot["file_details"].as_array().unwrap().len(), 2);
    assert!(snapshot.get("modules").is_some());
    assert!(snapshot.get("edges").is_some());
    assert!(snapshot.get("note").is_some());

    let rust = file(&snapshot, "src/lib.rs");
    assert_eq!(rust["id"], "1ec5a09a3e51f785");
    assert_eq!(rust["language"], "rust");
    assert_eq!(rust["function_count"], 1);
    assert_eq!(rust["functions"][0]["name"], "choose");
    assert_eq!(rust["functions"][0]["async"], true);
    assert_eq!(rust["functions"][0]["public"], true);
    assert_eq!(rust["functions"][0]["params"], 1);
    assert_eq!(rust["functions"][0]["complexity"], 3);
    assert_eq!(rust["source_anchor"]["label"], "src/lib.rs:1");
}

#[test]
fn dsm_snapshot_builds_cross_module_edges_and_risk_colors() {
    let fixture = Fixture::new();
    fixture.write(
        "foo/a.py",
        "from bar import b\n\ndef alpha(x):\n    if x:\n        return b(x)\n    return 0\n",
    );
    fixture.write("bar/b.py", "def b(x):\n    return x\n");

    let snapshot =
        sentrux_analysis::analyze(Path::new(&fixture.root)).expect("native DSM analysis");
    let edges = snapshot["edges"].as_array().expect("edge array");
    assert!(edges
        .iter()
        .any(|edge| edge["from"] == "foo" && edge["to"] == "bar"));

    let modules = snapshot["modules"].as_array().expect("module array");
    let foo = modules
        .iter()
        .find(|module| module["name"] == "foo")
        .expect("foo module");
    assert_eq!(foo["metrics"]["outbound_edges"], 1);
    assert_eq!(foo["metrics"]["coupling"], 1);
    assert!(foo["colors"]["Risk"]["score"].is_number());
    assert!(foo["colors"]["Risk"]["color"]
        .as_str()
        .expect("risk color")
        .starts_with('#'));
}

#[test]
fn dsm_dependency_graph_accumulates_imports_and_resolves_workspace_crates() {
    let fixture = Fixture::new();
    fixture.write("a.py", "import b\nimport b\n");
    fixture.write("b.py", "VALUE = 1\n");
    fixture.write("crates/foo/src/lib.rs", "use bar::Thing;\n");
    fixture.write("crates/bar/src/lib.rs", "pub struct Thing;\n");

    let snapshot = sentrux_analysis::analyze(&fixture.root).expect("native DSM analysis");
    let edges = snapshot["edges"].as_array().expect("edge array");
    let python = edges
        .iter()
        .find(|edge| edge["from"] == "a.py" && edge["to"] == "b.py")
        .expect("flat Python import edge");
    assert_eq!(python["count"], 2);
    assert!(edges
        .iter()
        .any(|edge| edge["from"] == "crates/foo" && edge["to"] == "crates/bar"));
}

#[test]
fn dsm_execution_depth_collapses_dependency_cycles() {
    let fixture = Fixture::new();
    fixture.write("a.py", "import b\n");
    fixture.write("b.py", "import a\n");
    fixture.write("c.py", "import a\n");

    let snapshot = sentrux_analysis::analyze(&fixture.root).expect("native DSM analysis");
    assert_eq!(module(&snapshot, "a.py")["metrics"]["exec_depth"], 0);
    assert_eq!(module(&snapshot, "b.py")["metrics"]["exec_depth"], 0);
    assert_eq!(module(&snapshot, "c.py")["metrics"]["exec_depth"], 1);
}

#[test]
fn dsm_function_boundaries_ignore_sibling_decorators_and_structural_braces() {
    let fixture = Fixture::new();
    fixture.write(
        "decorated.py",
        "def first():\n    return 1\n@decorate\ndef second():\n    return 2\n",
    );
    fixture.write(
        "src/lib.rs",
        "trait Contract {\n    fn declaration(&self);\n}\npub fn real() -> &'static str {\n    let marker = \"}\"; // }\n    \"ok\"\n}\n",
    );
    fixture.write("script.ps1", "function   Invoke-Contract { 1 }\n");
    fixture.write(
        "frontend/main.js",
        "export const project = (items) => items.map((item) => item + 1);\n",
    );

    let snapshot = sentrux_analysis::analyze(&fixture.root).expect("native DSM analysis");
    let python = file(&snapshot, "decorated.py");
    assert_eq!(python["functions"][0]["end_line"], 2);
    let rust = file(&snapshot, "src/lib.rs");
    assert_eq!(rust["functions"][0]["end_line"], 2);
    assert_eq!(rust["functions"][1]["end_line"], 7);
    assert_eq!(file(&snapshot, "script.ps1")["function_count"], 1);
    assert_eq!(
        file(&snapshot, "frontend/main.js")["functions"][0]["complexity"],
        1
    );
}

#[test]
fn dsm_analysis_reports_unreadable_source_content() {
    let fixture = Fixture::new();
    fixture.write_bytes("src/bad.rs", &[0xff, 0xfe]);

    let error = sentrux_analysis::analyze(&fixture.root).expect_err("invalid UTF-8 must fail");
    assert!(error.contains("read source"));
    assert!(error.contains("bad.rs"));
}

#[cfg(unix)]
#[test]
fn dsm_inventory_does_not_traverse_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let external = Fixture::new();
    external.write("outside.py", "raise RuntimeError\n");
    let fixture = Fixture::new();
    fixture.write("inside.py", "VALUE = 1\n");
    symlink(&external.root, fixture.root.join("linked")).expect("create fixture symlink");

    let snapshot = sentrux_analysis::analyze(&fixture.root).expect("native DSM analysis");
    assert_eq!(snapshot["scope"]["included_files"], 1);
    assert_eq!(file(&snapshot, "inside.py")["path"], "inside.py");
    assert!(snapshot["file_details"]
        .as_array()
        .unwrap()
        .iter()
        .all(|detail| detail["path"] != "linked/outside.py"));
}

#[test]
fn production_pipeline_prefers_rust_dsm_with_explicit_powershell_rollback() {
    let pipeline = include_str!("../../../run-code-intel.ps1");
    assert!(pipeline.contains("CODE_INTEL_SENTRUX_DSM_PROVIDER"));
    assert!(pipeline.contains("CODE_INTEL_RUST_CLI"));
    assert!(pipeline.contains("& $sentruxDsmRustCli sentrux dsm $sentruxTargetPath"));
    assert!(pipeline.contains("& $sentruxAgentTool dsm $sentruxTargetPath"));
    assert!(pipeline.contains("RuntimeInformation]::IsOSPlatform"));
    assert!(pipeline.contains("$dsmLaunchError = $_.Exception.Message"));
    assert!(pipeline.contains("Sentrux DSM provider: $sentruxDsmProvider"));
}
