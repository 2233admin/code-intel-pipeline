use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{json, Value};

use super::{doctor_tool_fixture, temp_dir};
use crate::common;

#[path = "../support/sha256.rs"]
mod sha256;

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    artifacts: PathBuf,
}

impl Fixture {
    fn new(
        payloads: Vec<(&str, &str, Value)>,
        report: impl FnOnce(&[Value]) -> Option<Value>,
    ) -> Self {
        let root = temp_dir();
        let repo = root.join("fixture-repo");
        let source = root.join("source");
        let artifacts = root.join("artifacts");
        let authority = artifacts.join("fixture-repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(&authority).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        let tools = doctor_tool_fixture(&root, true);
        let output = common::cli()
            .args(["run", "dag-coordinate", "--repo"])
            .arg(&repo)
            .arg("--out")
            .arg(&source)
            .arg("--doctor-tool-path-prefix")
            .arg(tools)
            .output()
            .unwrap();
        let original = successful_json(output);
        let snapshot = original["snapshotIdentity"].as_str().unwrap();
        let mut refs = Vec::new();
        for (schema, kind, value) in payloads {
            refs.push(stage(&source, schema, kind, snapshot, &value));
        }
        let report_value = report(&refs);
        let mut nodes = json!({"repo.snapshot":original["nodes"]["repo.snapshot"],
            "consumer-fixture":{"status":"succeeded","verdict":"pass","artifacts":refs}});
        if let Some(report) = report_value {
            let reference = stage(
                &source,
                "code-intel-anchor-verification.v1",
                "verification.anchors",
                snapshot,
                &report,
            );
            nodes["verification.anchors"] =
                json!({"status":"succeeded","verdict":"pass","artifacts":[reference]});
        }
        let provenance = stage(
            &source,
            "code-intel-repository-iteration-provenance.v1",
            "repository.iteration",
            snapshot,
            &json!({
                "schema":"code-intel-repository-iteration-provenance.v1",
                "purpose":"repository_intelligence_iteration","runIdentity":original["runIdentity"],
                "snapshotIdentity":snapshot,"repositoryKey":"fixture-repo","publicationName":"run-001",
                "producer":{"component":"code-intel.authoritative-run","contract":"repository-iteration-producer","version":"1"},
            }),
        );
        nodes["repository.iteration"] =
            json!({"status":"succeeded","verdict":"pass","artifacts":[provenance]});
        let manifest = json!({"schema":"code-intel-run-manifest.v1","runIdentity":original["runIdentity"],
            "snapshotIdentity":snapshot,"outcome":"completed","nodes":nodes});
        let manifest_ref = stage(
            &source,
            "code-intel-run-manifest.v1",
            "run.manifest",
            snapshot,
            &manifest,
        );
        let manifest_ref_path = source.join("consumer-manifest-ref.json");
        fs::write(
            &manifest_ref_path,
            serde_json::to_vec(&manifest_ref).unwrap(),
        )
        .unwrap();
        successful_json(
            common::cli()
                .args(["run", "commit", "--source-root"])
                .arg(&source)
                .arg("--authority-root")
                .arg(&authority)
                .arg("--manifest-ref")
                .arg(&manifest_ref_path)
                .args(["--final-name", "run-001"])
                .output()
                .unwrap(),
        );
        Self {
            root,
            repo,
            artifacts,
        }
    }

    fn query_output(&self, kind: &str) -> Output {
        common::cli()
            .args(["artifact", "query", "--artifact-root"])
            .arg(&self.artifacts)
            .args(["--repo", "fixture-repo", "--type", kind])
            .output()
            .unwrap()
    }

    fn query(&self, kind: &str) -> Value {
        successful_json(self.query_output(kind))
    }

    fn navigation(&self, kind: &str) -> Value {
        let query = self.query(kind);
        assert_eq!(query["schema"], "code-intel-evidence-query.v2");
        assert_eq!(query["matches"].as_array().unwrap().len(), 1, "{query}");
        query["matches"][0]["anchorEvidence"].clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn successful_json(output: Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn stage(root: &Path, schema: &str, kind: &str, snapshot: &str, value: &Value) -> Value {
    let bytes = serde_json::to_vec(value).unwrap();
    let digest = sha256::sha256_hex(&bytes);
    let path = format!("objects/sha256/{digest}");
    fs::create_dir_all(root.join("objects/sha256")).unwrap();
    fs::write(root.join(&path), bytes).unwrap();
    json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":schema,"type":kind,
        "path":path,"sha256":digest,"consumedSnapshotIdentity":snapshot})
}

fn ranking(paths: &[&str]) -> (&'static str, &'static str, Value) {
    (
        "agent-code-slice-ranking.v1",
        "code_evidence.agent_slice",
        json!({
            "schema":"agent-code-slice-ranking.v1","strategy":"native-evidence-default",
            "files":paths.iter().map(|path| json!({"path":path})).collect::<Vec<_>>(),
        }),
    )
}

fn symbols(claims: Value) -> (&'static str, &'static str, Value) {
    (
        "code-evidence-symbols.v1",
        "code_evidence.symbols",
        json!({"schema":"code-evidence-symbols.v1","symbols":claims}),
    )
}

fn source(reference: &Value, anchor_kind: &str, anchors: Value) -> Value {
    json!({"artifactType":reference["type"],"artifactPath":reference["path"],"anchorKind":anchor_kind,"anchors":anchors})
}

fn report(sources: Vec<Value>) -> Value {
    let mut counts = json!({"verified":0,"approximate":0,"dropped":0});
    for source in &sources {
        for anchor in source["anchors"].as_array().unwrap() {
            let state = anchor["state"].as_str().unwrap();
            counts[state] = json!(counts[state].as_u64().unwrap() + 1);
        }
    }
    json!({"schema":"code-intel-anchor-verification.v1","counts":counts,"sources":sources})
}

fn assert_unknown_items(navigation: &Value) {
    assert_eq!(navigation["basis"], "publication_time");
    assert_eq!(navigation["currentValidity"], "not_assessed");
    assert!(!navigation["items"].as_array().unwrap().is_empty());
    for item in navigation["items"].as_array().unwrap() {
        assert_eq!(item["state"], "not_assessed", "{navigation}");
        assert!(item["navigationTarget"].is_null(), "{navigation}");
        assert!(item["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()));
    }
}

#[test]
fn navigation_preserves_claims_moved_lines_and_dropped_reasons_in_source_order() {
    let fixture = Fixture::new(
        vec![
            ranking(&["src/lib.rs", "src/deleted.rs"]),
            symbols(json!([
                {"file":"src/lib.rs","name":"moved","startLine":2},
                {"file":"src/lib.rs","name":"fixture","startLine":1},
                {"file":"src/lib.rs","name":"removed","startLine":9},
            ])),
        ],
        |refs| {
            Some(report(vec![
                source(
                    &refs[0],
                    "file",
                    json!([
                        {"path":"src/deleted.rs","state":"dropped","reason":"file not found in repository: src/deleted.rs"},
                        {"path":"src/lib.rs","state":"verified"},
                    ]),
                ),
                source(
                    &refs[1],
                    "symbol",
                    json!([
                        {"file":"src/lib.rs","name":"fixture","claimedLine":1,"state":"verified"},
                        {"file":"src/lib.rs","name":"removed","claimedLine":9,"state":"dropped","reason":"symbol removed from src/lib.rs"},
                        {"file":"src/lib.rs","name":"moved","claimedLine":2,"state":"approximate","resolvedLine":8},
                    ]),
                ),
            ]))
        },
    );
    let report_ref = fixture.query("verification.anchors")["matches"][0]["artifactRef"].clone();
    let files = fixture.navigation("code_evidence.agent_slice");
    assert_eq!(files["status"], "available");
    assert_eq!(files["reportRef"], report_ref);
    assert_eq!(
        files["items"],
        json!([
            {"path":"src/lib.rs","name":null,"claimedLine":null,"state":"verified","resolvedLine":null,
                "navigationTarget":{"path":"src/lib.rs","line":null},"reason":null},
            {"path":"src/deleted.rs","name":null,"claimedLine":null,"state":"dropped","resolvedLine":null,
                "navigationTarget":null,"reason":"file not found in repository: src/deleted.rs"},
        ])
    );
    let navigation = fixture.navigation("code_evidence.symbols");
    assert_eq!(navigation["status"], "available");
    assert_eq!(navigation["reportRef"], report_ref);
    assert_eq!(
        navigation["items"],
        json!([
            {"path":"src/lib.rs","name":"moved","claimedLine":2,"state":"approximate","resolvedLine":8,
                "navigationTarget":{"path":"src/lib.rs","line":8},"reason":null},
            {"path":"src/lib.rs","name":"fixture","claimedLine":1,"state":"verified","resolvedLine":null,
                "navigationTarget":{"path":"src/lib.rs","line":1},"reason":null},
            {"path":"src/lib.rs","name":"removed","claimedLine":9,"state":"dropped","resolvedLine":null,
                "navigationTarget":null,"reason":"symbol removed from src/lib.rs"},
        ])
    );
    assert_eq!(navigation["currentValidity"], "not_assessed");
    assert_eq!(navigation["itemsTruncated"], false);
    // Publication evidence must not be silently refreshed against the checkout.
    fs::write(fixture.repo.join("src/lib.rs"), "// all symbols removed\n").unwrap();
    assert_eq!(fixture.navigation("code_evidence.symbols"), navigation);
}

#[test]
fn historical_run_without_companion_keeps_query_success_and_unknown_navigation() {
    let fixture = Fixture::new(vec![ranking(&["src/lib.rs"])], |_| None);
    let navigation = fixture.navigation("code_evidence.agent_slice");
    assert_eq!(navigation["status"], "unavailable");
    assert!(navigation["reportRef"].is_null());
    assert!(navigation["reason"]
        .as_str()
        .unwrap()
        .contains("no publication-time"));
    assert_unknown_items(&navigation);
}

#[test]
fn source_association_requires_exact_type_path_and_anchor_kind() {
    for mismatch in ["type", "path", "kind"] {
        let fixture = Fixture::new(vec![ranking(&["src/lib.rs"])], |refs| {
            let mut entry = source(
                &refs[0],
                "file",
                json!([{"path":"src/lib.rs","state":"verified"}]),
            );
            match mismatch {
                "type" => entry["artifactType"] = json!("diagnosis.surgery-plan"),
                "path" => {
                    entry["artifactPath"] =
                        json!(format!("{}-different", refs[0]["path"].as_str().unwrap()))
                }
                _ => {
                    entry["anchorKind"] = json!("symbol");
                    entry["anchors"] = json!([{"file":"src/lib.rs","name":"fixture","claimedLine":1,"state":"verified"}]);
                }
            }
            Some(report(vec![entry]))
        });
        let navigation = fixture.navigation("code_evidence.agent_slice");
        assert_eq!(navigation["status"], "unavailable", "{mismatch}");
        assert!(!navigation["reportRef"].is_null());
        assert_unknown_items(&navigation);
    }
}

#[test]
fn duplicate_sources_and_conflicting_claim_records_never_promote_navigation() {
    let fixture = Fixture::new(vec![ranking(&["src/lib.rs"])], |refs| {
        Some(report(vec![
            source(
                &refs[0],
                "file",
                json!([{"path":"src/lib.rs","state":"verified"}]),
            ),
            source(
                &refs[0],
                "file",
                json!([{"path":"src/lib.rs","state":"dropped","reason":"conflicting assessment"}]),
            ),
        ]))
    });
    let navigation = fixture.navigation("code_evidence.agent_slice");
    assert_eq!(navigation["status"], "unavailable");
    assert_unknown_items(&navigation);
    let fixture = Fixture::new(vec![ranking(&["src/lib.rs"])], |refs| {
        Some(report(vec![source(
            &refs[0],
            "file",
            json!([
                {"path":"src/lib.rs","state":"verified"},
                {"path":"src/lib.rs","state":"dropped","reason":"conflicting assessment"},
            ]),
        )]))
    });
    let navigation = fixture.navigation("code_evidence.agent_slice");
    assert_eq!(navigation["status"], "available");
    assert_unknown_items(&navigation);
}

#[test]
fn symbol_identity_never_borrows_assessment_from_another_file_name_or_line() {
    let fixture = Fixture::new(
        vec![symbols(json!([
            {"file":"src/lib.rs","name":"fixture","startLine":1},
            {"file":"src/lib.rs","name":"missing-name","startLine":1},
            {"file":"src/lib.rs","name":"fixture","startLine":7},
        ]))],
        |refs| {
            Some(report(vec![source(
                &refs[0],
                "symbol",
                json!([
                    {"file":"src/other.rs","name":"fixture","claimedLine":1,"state":"verified"},
                    {"file":"src/lib.rs","name":"fixture","claimedLine":3,"state":"verified"},
                    {"file":"src/lib.rs","name":"different-name","claimedLine":1,"state":"verified"},
                ]),
            )]))
        },
    );
    let navigation = fixture.navigation("code_evidence.symbols");
    assert_eq!(navigation["status"], "available");
    assert_unknown_items(&navigation);
    assert_eq!(navigation["items"][0]["path"], "src/lib.rs");
    assert_eq!(navigation["items"][0]["claimedLine"], 1);
}

#[test]
fn anchor_item_bound_is_independent_from_artifact_limit_and_exactly_twenty_is_complete() {
    for count in [20, 21] {
        let paths = (0..count)
            .map(|index| format!("src/file-{index:02}.rs"))
            .collect::<Vec<_>>();
        let borrowed = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let fixture = Fixture::new(vec![ranking(&borrowed)], |refs| {
            Some(report(vec![source(
                &refs[0],
                "file",
                Value::Array(
                    paths
                        .iter()
                        .rev()
                        .map(|path| json!({"path":path,"state":"verified"}))
                        .collect(),
                ),
            )]))
        });
        let query = fixture.query("code_evidence.agent_slice");
        assert_eq!(query["searchCoverage"]["status"], "complete");
        let navigation = &query["matches"][0]["anchorEvidence"];
        assert_eq!(navigation["items"].as_array().unwrap().len(), 20);
        assert_eq!(navigation["items"][0]["path"], "src/file-00.rs");
        assert_eq!(navigation["items"][19]["path"], "src/file-19.rs");
        assert_eq!(navigation["itemsTruncated"], count > 20);
    }
}

#[test]
fn unsupported_artifact_contract_is_explicitly_not_applicable() {
    let fixture = Fixture::new(
        vec![(
            "code-evidence-files.v1",
            "code_evidence.files",
            json!({
                "schema":"code-evidence-files.v1","files":[{"path":"src/lib.rs"}],
            }),
        )],
        |refs| {
            Some(report(vec![source(
                &refs[0],
                "file",
                json!([{"path":"src/lib.rs","state":"verified"}]),
            )]))
        },
    );
    let navigation = fixture.navigation("code_evidence.files");
    assert_eq!(navigation["status"], "not_applicable");
    assert_eq!(navigation["items"], json!([]));
    assert_eq!(navigation["itemsTruncated"], false);
    assert!(navigation["reason"]
        .as_str()
        .unwrap()
        .contains("no supported"));
}

#[test]
fn surgery_plan_primary_target_is_projected_but_null_target_is_not_a_claim() {
    for target in [json!("src/lib.rs"), Value::Null] {
        let fixture = Fixture::new(
            vec![(
                "code-intel-surgery-plan.v1",
                "diagnosis.surgery-plan",
                json!({
                    "schema":"code-intel-surgery-plan.v1","status":"planned","admission":{},
                    "primary_target":{"file":target},"operating_plan":[],"verification":[],"discharge_criteria":[],
                }),
            )],
            |refs| {
                Some(report(vec![source(
                    &refs[0],
                    "file",
                    json!([{"path":"src/lib.rs","state":"verified"}]),
                )]))
            },
        );
        let navigation = fixture.navigation("diagnosis.surgery-plan");
        if target.is_null() {
            assert_eq!(navigation["status"], "not_applicable");
            assert_eq!(navigation["items"], json!([]));
        } else {
            assert_eq!(navigation["status"], "available");
            assert_eq!(
                navigation["items"][0]["navigationTarget"],
                json!({"path":"src/lib.rs","line":null})
            );
        }
    }
}

#[test]
fn tampered_report_is_rejected_at_admission_even_when_query_requests_only_source() {
    let fixture = Fixture::new(vec![ranking(&["src/lib.rs"])], |refs| {
        Some(report(vec![source(
            &refs[0],
            "file",
            json!([{"path":"src/lib.rs","state":"verified"}]),
        )]))
    });
    let query = fixture.query("verification.anchors");
    let report_path = query["matches"][0]["artifactRef"]["path"].as_str().unwrap();
    fs::write(
        fixture
            .artifacts
            .join("fixture-repo/run-001")
            .join(report_path),
        b"tampered report\n",
    )
    .unwrap();
    let output = fixture.query_output("code_evidence.agent_slice");
    assert_eq!(output.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no committed authoritative run"));
}

#[test]
fn real_publication_delivers_verified_navigation_without_a_second_report_query() {
    let root = temp_dir();
    let repo = root.join("fixture-repo");
    let artifacts = root.join("artifacts");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(artifacts.join("fixture-repo")).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    let tools = doctor_tool_fixture(&root, true);
    successful_json(
        common::cli()
            .args(["run", "execute", "--repo"])
            .arg(&repo)
            .arg("--out")
            .arg(root.join("source"))
            .arg("--authority-root")
            .arg(artifacts.join("fixture-repo"))
            .args(["--final-name", "run-001"])
            .arg("--doctor-tool-path-prefix")
            .arg(tools)
            .output()
            .unwrap(),
    );
    let fixture = Fixture {
        root,
        repo,
        artifacts,
    };
    let navigation = fixture.navigation("code_evidence.symbols");
    assert_eq!(navigation["status"], "available");
    assert_eq!(navigation["currentValidity"], "not_assessed");
    assert_eq!(navigation["items"][0]["path"], "src/lib.rs");
    assert_eq!(navigation["items"][0]["name"], "fixture");
    assert_eq!(navigation["items"][0]["state"], "verified");
    assert_eq!(
        navigation["items"][0]["navigationTarget"],
        json!({"path":"src/lib.rs","line":1})
    );
    assert_eq!(navigation["reportRef"]["type"], "verification.anchors");
}

#[test]
fn incomplete_claim_identity_and_zero_resolved_line_remain_unknown() {
    let fixture = Fixture::new(
        vec![symbols(json!([
            {"file":"src/lib.rs","name":"incomplete"},
            {"file":"src/lib.rs","name":"moved","startLine":2},
        ]))],
        |refs| {
            Some(report(vec![source(
                &refs[0],
                "symbol",
                json!([
                    {"file":"src/lib.rs","name":"incomplete","claimedLine":0,"state":"verified"},
                    {"file":"src/lib.rs","name":"moved","claimedLine":2,"state":"approximate","resolvedLine":0},
                ]),
            )]))
        },
    );
    let navigation = fixture.navigation("code_evidence.symbols");
    assert_eq!(navigation["status"], "available");
    assert_unknown_items(&navigation);
    assert!(navigation["items"][0]["claimedLine"].is_null());
    assert!(navigation["items"][1]["resolvedLine"].is_null());
}
