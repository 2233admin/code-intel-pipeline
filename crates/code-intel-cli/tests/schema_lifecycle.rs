use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate must live under <repo>/crates")
        .to_path_buf()
}

fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn strings<'a>(value: &'a Value, label: &str) -> Vec<&'a str> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{label} must be an array"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{label} items must be strings"))
        })
        .collect()
}

fn exact_keys(value: &Value, expected: &[&str], label: &str) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{label} must be an object"));
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{label} keys differ");
}

fn versioned_schema_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".schema.json") else {
        return false;
    };
    let Some((base, version)) = stem.rsplit_once(".v") else {
        return false;
    };
    !base.is_empty()
        && !version.is_empty()
        && !version.starts_with('0')
        && version.bytes().all(|byte| byte.is_ascii_digit())
}

#[test]
fn all_schema_files_have_stable_unique_identity_and_registry_refs_resolve() {
    let root = repo_root();
    let schema_root = root.join("orchestration/schemas");
    let mut ids = BTreeMap::new();
    let mut count = 0;
    for entry in fs::read_dir(&schema_root).expect("read schema directory") {
        let entry = entry.expect("read schema entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".schema.json") {
            continue;
        }
        count += 1;
        assert!(
            versioned_schema_name(&name),
            "schema filename is not versioned: {name}"
        );
        let schema = read_json(&entry.path());
        assert!(
            matches!(
                schema["$schema"].as_str(),
                Some("https://json-schema.org/draft/2020-12/schema")
                    | Some("http://json-schema.org/draft-07/schema#")
            ),
            "unsupported JSON Schema draft in {name}"
        );
        let id = schema["$id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| panic!("schema $id is required in {name}"));
        assert!(
            ids.insert(id.to_string(), name.clone()).is_none(),
            "duplicate schema $id: {id}"
        );
    }
    assert!(count >= 12, "schema inventory unexpectedly small");

    let registry = read_json(&root.join("orchestration/integrations.json"));
    for integration in registry["integrations"]
        .as_array()
        .expect("integrations must be an array")
    {
        let id = integration["id"].as_str().expect("integration id");
        let Some(contracts) = integration.get("artifactContract") else {
            continue;
        };
        for contract in contracts.as_array().into_iter().flatten() {
            let Some(path) = contract.as_str() else {
                continue;
            };
            if path.ends_with(".schema.json") {
                assert!(
                    root.join(path).is_file(),
                    "integration {id} references missing schema {path}"
                );
            }
        }
    }
}

#[test]
fn lifecycle_catalog_is_coherent() {
    let root = repo_root();
    let lifecycle = read_json(&root.join("orchestration/schema-lifecycle.v1.json"));
    exact_keys(&lifecycle, &["schema", "policy", "contracts"], "lifecycle");
    assert_eq!(lifecycle["schema"], "code-intel-schema-lifecycle.v1");
    exact_keys(
        &lifecycle["policy"],
        &[
            "breakingChange",
            "compatibleChange",
            "retirement",
            "coreRuntimeRule",
        ],
        "lifecycle policy",
    );
    assert_eq!(lifecycle["policy"]["breakingChange"], "publish-new-version");
    assert_eq!(lifecycle["policy"]["compatibleChange"], "additive-only");
    assert_eq!(
        lifecycle["policy"]["retirement"],
        "evidence-backed-compatibility-window"
    );
    assert_eq!(
        lifecycle["policy"]["coreRuntimeRule"],
        "implementation-and-tests-required"
    );

    let required = [
        "code-intel-schema-lifecycle.v1",
        "code-intel-repository-snapshot.v1",
        "code-intel-run-dag.v1",
        "code-intel-run-state.v1",
        "code-intel-run-manifest.v1",
        "code-intel-staged-artifact-set.v1",
        "code-intel-run-commit.v1",
        "code-intel-artifact-index.v1",
        "code-intel-evidence-query.v1",
        "code-intel-change-impact.v1",
        "code-evidence-files.v1",
        "code-intel-session-evidence.v1",
    ];
    let mut paths = BTreeSet::new();
    let mut logical = BTreeSet::new();
    for contract in lifecycle["contracts"]
        .as_array()
        .expect("contracts must be an array")
    {
        exact_keys(
            contract,
            &[
                "schemaPath",
                "schemaId",
                "logicalSchemas",
                "shape",
                "status",
                "compatibilityPolicy",
                "implementation",
            ],
            "lifecycle contract",
        );
        assert_eq!(contract["status"], "active");
        assert_eq!(contract["compatibilityPolicy"], "additive-only");
        let relative = contract["schemaPath"].as_str().expect("schemaPath");
        assert!(
            paths.insert(relative),
            "duplicate lifecycle schemaPath: {relative}"
        );
        let schema_path = root.join(relative);
        let schema = read_json(&schema_path);
        assert_eq!(
            schema["$id"], contract["schemaId"],
            "$id mismatch for {relative}"
        );

        let names = strings(&contract["logicalSchemas"], "logicalSchemas");
        for name in &names {
            assert!(logical.insert(*name), "duplicate logical schema: {name}");
        }
        match contract["shape"].as_str() {
            Some("closed-object") => {
                assert_eq!(schema["type"], "object", "{relative} must be an object");
                assert_eq!(
                    schema["additionalProperties"], false,
                    "{relative} must be closed"
                );
                assert_eq!(
                    schema["properties"]["schema"]["const"], names[0],
                    "logical schema mismatch for {relative}"
                );
                assert_eq!(names.len(), 1, "closed object must have one logical schema");
            }
            Some("closed-composite") => {
                assert!(schema["oneOf"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty()));
                let encoded = serde_json::to_string(&schema).expect("encode composite schema");
                for name in names {
                    assert!(
                        encoded.contains(name),
                        "{relative} omits logical schema {name}"
                    );
                }
            }
            other => panic!("unsupported lifecycle shape: {other:?}"),
        }

        exact_keys(
            &contract["implementation"],
            &["source", "symbols", "tests"],
            "contract implementation",
        );
        let source_path = root.join(
            contract["implementation"]["source"]
                .as_str()
                .expect("implementation source"),
        );
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
        for symbol in strings(&contract["implementation"]["symbols"], "symbols") {
            assert!(
                source.contains(symbol),
                "implementation {} omits symbol {symbol}",
                source_path.display()
            );
        }
        for test in strings(&contract["implementation"]["tests"], "tests") {
            assert!(
                root.join(test).is_file(),
                "missing lifecycle test binding: {test}"
            );
        }
    }
    for schema in required {
        assert!(logical.contains(schema), "core lifecycle omits {schema}");
    }
}
