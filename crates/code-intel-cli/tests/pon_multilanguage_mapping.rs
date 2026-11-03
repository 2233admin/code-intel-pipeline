mod common;

use common::TempDir;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn tool_fixture(root: &Path) -> std::ffi::OsString {
    let bin = root.join("dag-tools");
    fs::create_dir_all(&bin).unwrap();
    #[cfg(windows)]
    {
        for name in ["python", "repowise"] {
            fs::write(
                bin.join(format!("{name}.cmd")),
                "@echo off\r\nexit /b 0\r\n",
            )
            .unwrap();
        }
    }
    #[cfg(not(windows))]
    {
        for name in ["python", "repowise"] {
            common::write_executable(&bin, name, "#!/bin/sh\nexit 0\n");
        }
    }
    // Stub only doctor Python/Repowise; keep rg/git and Sentrux real.
    let binary_dir = PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut paths = vec![bin, binary_dir];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).expect("fixture PATH")
}

fn run(repo: &Path, out: &Path) {
    let baseline = common::cli()
        .args(["sentrux", "--operation", "save_baseline", "--repo"])
        .arg(repo)
        .output()
        .unwrap();
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    let tools = tool_fixture(out.parent().unwrap());
    let output = common::cli()
        .args(["run", "dag-coordinate", "--repo"])
        .arg(repo)
        .arg("--out")
        .arg(out)
        .env("PATH", tools)
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
    let temp = TempDir::new("multilanguage-mapping");
    let repo = temp.path().join("repo");
    let out = temp.path().join("run");
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
