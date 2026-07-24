use std::collections::BTreeSet;
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
            "code-intel-pon-{label}-{}-{nonce}",
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

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
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

fn run(repo: &Path, out: &Path) {
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
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pon_frontend_pattern_maps_supported_languages_into_one_code_evidence_contract() {
    let fixture = read_json(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("orchestration/internalization/fixtures/pon-multilanguage-code-evidence.json"),
    );
    let temp = Temp::new("multilanguage-mapping");
    let repo = temp.0.join("repo");
    let out = temp.0.join("run");
    fs::create_dir_all(&repo).unwrap();

    for sample in fixture["samples"].as_array().unwrap() {
        fs::write(
            repo.join(sample["path"].as_str().unwrap()),
            sample["content"].as_str().unwrap(),
        )
        .unwrap();
    }

    run(&repo, &out);
    let root = out.join("evidence.native-code/code-evidence");
    let files = read_json(root.join("merged/full/files.json"));
    let symbols = read_json(root.join("merged/full/symbols.json"));
    let imports = read_json(root.join("merged/full/imports.json"));
    let coverage = read_json(root.join("coverage.json"));

    assert_eq!(files["schema"], "code-evidence-files.v1");
    assert_eq!(symbols["schema"], "code-evidence-symbols.v1");
    assert_eq!(imports["schema"], "code-evidence-imports.v1");
    assert_eq!(coverage["parserKind"], "line-heuristic");
    assert_eq!(coverage["relationshipPrecision"], "unknown");
    assert_eq!(coverage["callGraph"], "unknown");

    let unsupported = coverage["unsupportedFiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();

    for sample in fixture["samples"].as_array().unwrap() {
        let path = sample["path"].as_str().unwrap();
        let language = sample["language"].as_str().unwrap();
        let supported = sample["supported"].as_bool().unwrap();
        let file = files["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == path)
            .unwrap_or_else(|| panic!("missing file fact for {path}"));

        assert_eq!(file["language"], language, "language mismatch for {path}");
        assert_eq!(file["source"], "native-minimal");
        assert_eq!(unsupported.contains(path), !supported);

        let actual_symbols = symbols["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|symbol| symbol["file"] == path)
            .map(|symbol| {
                assert_eq!(symbol["language"], language);
                assert_eq!(symbol["confidence"], 0.55);
                assert_eq!(symbol["source"], "native-minimal");
                (
                    symbol["kind"].as_str().unwrap().to_string(),
                    symbol["name"].as_str().unwrap().to_string(),
                )
            })
            .collect::<BTreeSet<_>>();
        let expected_symbols = sample["expectedSymbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|symbol| {
                (
                    symbol["kind"].as_str().unwrap().to_string(),
                    symbol["name"].as_str().unwrap().to_string(),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_symbols, expected_symbols, "symbol drift for {path}");

        let actual_imports = imports["imports"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|import| import["file"] == path)
            .map(|import| {
                assert_eq!(import["language"], language);
                assert_eq!(import["confidence"], 0.6);
                assert_eq!(import["source"], "native-minimal");
                import["target"].as_str().unwrap().to_string()
            })
            .collect::<BTreeSet<_>>();
        let expected_imports = sample["expectedImports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|target| target.as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_imports, expected_imports, "import drift for {path}");
    }
}
