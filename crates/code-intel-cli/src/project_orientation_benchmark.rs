use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{json, Value};

use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::capability_inventory::{AdapterArtifact, AdapterError, AdapterOutput};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TARGET_MS: u64 = 60_000;

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if request["options"]
        .as_object()
        .map_or(true, |options| !options.is_empty())
    {
        return Err(AdapterError::InvalidOptions(
            "project.orientation-benchmark accepts no options".into(),
        ));
    }
    if verified_inputs.len() != 1
        || verified_inputs[0].artifact_schema()
            != "code-intel-project-orientation-benchmark-observations.v1"
        || verified_inputs[0].artifact_type() != "benchmark.orientation-observations"
    {
        return Err(AdapterError::Contract(
            "project.orientation-benchmark requires one A03-verified observation corpus".into(),
        ));
    }
    let observations: Value =
        serde_json::from_slice(verified_inputs[0].bytes()).map_err(|error| {
            AdapterError::Contract(format!("parse benchmark observations: {error}"))
        })?;
    if observations["snapshotIdentity"] != request["snapshot"]["identity"] {
        return Err(AdapterError::Contract(
            "benchmark observations do not match the request snapshot".into(),
        ));
    }
    let report = evaluate(&observations)?;
    let bytes = serde_json::to_vec(&report)
        .map_err(|error| AdapterError::Internal(format!("serialize benchmark report: {error}")))?;
    let markdown = render(&report).into_bytes();
    publish(out, "report.json", &bytes)?;
    publish(out, "report.md", &markdown)?;
    Ok(AdapterOutput {
        artifacts: vec![
            AdapterArtifact {
                artifact_schema: "code-intel-project-orientation-benchmark.v1".into(),
                artifact_type: "benchmark.orientation-report".into(),
                relative_path: "report.json".into(),
                bytes,
            },
            AdapterArtifact {
                artifact_schema: "code-intel-project-orientation-benchmark-markdown.v1".into(),
                artifact_type: "benchmark.orientation-report-view".into(),
                relative_path: "report.md".into(),
                bytes: markdown,
            },
        ],
        observed_effects: vec!["local_write".into()],
        domain_verdict: crate::capability_inventory::AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match run(raw) {
        Ok(report) => {
            println!("{}", serde_json::to_string(&report).unwrap());
            0
        }
        Err((code, message)) => {
            eprintln!("{message}");
            code
        }
    }
}

fn run(raw: &[String]) -> Result<Value, (i32, String)> {
    if raw.first().map(String::as_str) != Some("orientation") {
        return Err((64, "usage: benchmark orientation --out <directory> [--repetitions <2..10>] [--manifest <integrations.json>]".into()));
    }
    let mut out = None;
    let mut manifest = None;
    let mut repetitions = 3usize;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(flag, "--out" | "--repetitions" | "--manifest") {
            return Err((
                64,
                format!("unknown orientation benchmark argument: {flag}"),
            ));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| (64, format!("{flag} requires one value")))?;
        match flag {
            "--out" if out.replace(PathBuf::from(value)).is_some() => {
                return Err((64, "duplicate --out".into()))
            }
            "--manifest" if manifest.replace(PathBuf::from(value)).is_some() => {
                return Err((64, "duplicate --manifest".into()))
            }
            "--repetitions" => {
                repetitions = value
                    .parse()
                    .map_err(|_| (64, "--repetitions must be an integer".into()))?;
            }
            _ => {}
        }
        index += 2;
    }
    if !(2..=10).contains(&repetitions) {
        return Err((64, "--repetitions must be between 2 and 10".into()));
    }
    let out = out.ok_or_else(|| (64, "--out is required".into()))?;
    fs::create_dir(&out)
        .map_err(|error| (74, format!("exclusive benchmark output create: {error}")))?;
    let manifest = manifest.unwrap_or_else(default_registry);
    let binary = std::env::current_exe()
        .map_err(|error| (74, format!("locate benchmark executable: {error}")))?;
    let fixtures = build_fixtures();
    let mut observed = Vec::new();
    for fixture in fixtures {
        let warm_root = out.join("work").join("warm").join(&fixture.id);
        let materialize_start = Instant::now();
        let request_path = materialize(&warm_root, &fixture, &manifest)?;
        let warm_materialization = elapsed_ms(materialize_start);
        let mut warm = Vec::new();
        for repetition in 0..repetitions {
            warm.push(run_sample(
                &binary,
                &manifest,
                &warm_root,
                &request_path,
                &warm_root.join(format!("out-{repetition}")),
                0,
            )?);
        }
        let mut cold = Vec::new();
        for repetition in 0..repetitions {
            let root = out
                .join("work")
                .join("cold")
                .join(format!("{}-{repetition}", fixture.id));
            let start = Instant::now();
            let request_path = materialize(&root, &fixture, &manifest)?;
            let materialization = elapsed_ms(start);
            cold.push(run_sample(
                &binary,
                &manifest,
                &root,
                &request_path,
                &root.join("out"),
                materialization,
            )?);
        }
        observed.push(json!({
            "id":fixture.id,
            "size":fixture.size,
            "condition":fixture.condition,
            "typical":fixture.typical,
            "expected":{
                "activeChange":fixture.active_change,
                "fileCount":fixture.file_count,
                "providerStatus":if fixture.condition == "provider_missing" {"unavailable"} else {"available"},
                "unknownFields":["call_graph","purpose","structural_relationships"],
                "unsupportedFiles":["Cargo.toml","README.md"]
            },
            "warmPreparationMs":warm_materialization,
            "samples":{"cold":cold,"warm":warm}
        }));
    }
    let observations = json!({
        "schema":"code-intel-project-orientation-benchmark-observations.v1",
        "snapshotIdentity":SNAPSHOT,
        "method":{
            "clock":"std::time::Instant",
            "execution":"sequential_child_process",
            "concurrency":1,
            "repetitionsPerTemperature":repetitions,
            "coldDefinition":"fresh materialized immutable Artifact Ref corpus and fresh output directory",
            "warmDefinition":"reused immutable Artifact Ref corpus with a fresh output directory",
            "percentile":"nearest_rank",
            "llm":"disabled"
        },
        "environment":{
            "os":std::env::consts::OS,
            "arch":std::env::consts::ARCH,
            "processor":std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".into()),
            "cleanMachine":false
        },
        "fixtures":observed
    });
    fs::write(
        out.join("observations.json"),
        serde_json::to_vec(&observations).unwrap(),
    )
    .map_err(|error| (74, format!("write benchmark observations: {error}")))?;
    let report = evaluate(&observations).map_err(|error| (65, adapter_message(error)))?;
    fs::write(
        out.join("report.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .map_err(|error| (74, format!("write benchmark report: {error}")))?;
    fs::write(out.join("report.md"), render(&report))
        .map_err(|error| (74, format!("write benchmark Markdown: {error}")))?;
    Ok(report)
}

struct Fixture {
    id: String,
    size: &'static str,
    condition: &'static str,
    typical: bool,
    file_count: usize,
    active_change: &'static str,
}

fn build_fixtures() -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    for (size, file_count, typical) in [
        ("small", 5, true),
        ("medium", 50, true),
        ("large", 500, false),
    ] {
        for condition in ["clean", "dirty", "provider_missing"] {
            fixtures.push(Fixture {
                id: format!("{size}-{condition}"),
                size,
                condition,
                typical,
                file_count,
                active_change: if condition == "dirty" {
                    "dirty"
                } else {
                    "clean"
                },
            });
        }
    }
    fixtures
}

fn materialize(root: &Path, fixture: &Fixture, manifest: &Path) -> Result<PathBuf, (i32, String)> {
    fs::create_dir_all(root)
        .map_err(|error| (74, format!("create benchmark fixture root: {error}")))?;
    let mut paths = vec![
        "Cargo.toml".to_string(),
        "README.md".into(),
        "run.ps1".into(),
        "src/main.rs".into(),
        "tests/orientation.rs".into(),
    ];
    for index in paths.len()..fixture.file_count {
        paths.push(format!("src/module-{index:04}.rs"));
    }
    paths.sort();
    let native = paths
        .iter()
        .map(|path| json!({
            "path":path,
            "language":if path.ends_with(".rs") {"rust"} else if path.ends_with(".ps1") {"powershell"} else {"text"},
            "bytes":16,"lines":1,"textHash":"3".repeat(64),"source":"benchmark-fixture"
        }))
        .collect::<Vec<_>>();
    let dirty = fixture.condition == "dirty";
    let snapshot = json!({
        "schema":"code-intel-repository-snapshot.v1",
        "snapshot":{"identity":SNAPSHOT,"repoIdentity":format!("content-v1:{}", "c".repeat(64)),"head":"benchmark-corpus-v1","workingTreePolicy":"explicit_overlay","scope":["."],"inputDigest":"d".repeat(64)},
        "dirtyOverlay":{"present":dirty,"digest":if dirty {json!("e".repeat(64))} else {Value::Null},"paths":if dirty {json!(["src/main.rs"])} else {json!([])},"members":{"trackedModified":if dirty {json!(["src/main.rs"])} else {json!([])},"trackedDeleted":[],"untracked":[],"renamed":[],"typeChanged":[],"staged":[]},"ignoredPolicy":"excluded_by_git_ignore"},
        "repository":{"kind":"unversioned"}
    });
    let inventory = format!("{}\n", paths.join("\n"));
    let snapshot_ref = write_ref(
        root,
        "snapshot.json",
        "code-intel-repository-snapshot.v1",
        "repository.snapshot",
        serde_json::to_vec(&snapshot).unwrap(),
    )?;
    let inventory_ref = write_ref(
        root,
        "files.txt",
        "code-intel-file-inventory.v1",
        "inventory.files",
        inventory.into_bytes(),
    )?;
    let survival = json!({
        "schema":"code-intel-repository-survival-scan-result.v1","status":"completed","snapshotIdentity":SNAPSHOT,
        "repository":{"kind":"unversioned","identity":format!("content-v1:{}", "c".repeat(64)),"revision":"benchmark-corpus-v1","dirty":dirty,"sourceSha256":snapshot_ref["sha256"]},
        "inventory":{"fileCount":fixture.file_count,"extensions":{"rs":fixture.file_count.saturating_sub(3),"md":1,"toml":1,"ps1":1},"sourceSha256":inventory_ref["sha256"]},
        "providerDiagnosis":{"providerId":"codenexus.full","status":"provider_unavailable","domainVerdict":"unknown"},
        "completeness":"reduced","structuralVerdict":"unknown",
        "limitations":["only repository identity and basic file inventory are available","deeper structural perception requires an admitted provider result"],
        "engineeringFacts":[
            {"kind":"repository_identity","value":format!("content-v1:{}", "c".repeat(64)),"sourceSha256":snapshot_ref["sha256"]},
            {"kind":"repository_revision","value":"benchmark-corpus-v1","sourceSha256":snapshot_ref["sha256"]},
            {"kind":"inventory_file_count","value":fixture.file_count,"sourceSha256":inventory_ref["sha256"]}
        ]
    });
    let inputs = vec![
        snapshot_ref,
        inventory_ref,
        write_json_ref(
            root,
            "survival.json",
            "code-intel-repository-survival-scan-result.v1",
            "repository.survival-scan",
            &survival,
        )?,
        write_json_ref(
            root,
            "native-files.json",
            "code-evidence-files.v1",
            "code_evidence.files",
            &json!({"schema":"code-evidence-files.v1","files":native}),
        )?,
        write_json_ref(
            root,
            "native-coverage.json",
            "code-evidence-coverage.v1",
            "code_evidence.coverage",
            &json!({"schema":"code-evidence-coverage.v1","producer":if fixture.condition == "provider_missing" {"benchmark-provider-missing"} else {"benchmark-provider-ready"},"parserKind":"line-heuristic","supportedHeuristics":["rust","powershell"],"unsupportedFiles":["Cargo.toml","README.md"],"symbolPrecision":"heuristic","importPrecision":"heuristic","relationshipPrecision":"unknown","callGraph":"unknown","effects":["repo_read","local_write"]}),
        )?,
        write_json_ref(
            root,
            "native-ranking.json",
            "agent-code-slice-ranking.v1",
            "code_evidence.agent_slice",
            &json!({"schema":"agent-code-slice-ranking.v1","strategy":"benchmark-fixture","files":[{"path":"src/main.rs","language":"rust","score":40,"reasons":["entrypoint"],"symbols":null,"imports":null}]}),
        )?,
    ];
    let registry: Value = serde_json::from_slice(
        &fs::read(manifest).map_err(|error| (74, format!("read registry: {error}")))?,
    )
    .map_err(|error| (65, format!("parse registry: {error}")))?;
    let declaration = registry["integrations"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == "project.orientation")
        })
        .ok_or_else(|| (65, "project.orientation declaration is missing".into()))?;
    let request = json!({
        "schema":"code-intel-capability-request.v1","capability":"project.orientation","contractVersion":1,
        "implementation":declaration["capabilityDeclaration"]["implementation"],"snapshot":snapshot["snapshot"],"options":{},"inputs":inputs,
        "effectPolicy":{"allowedEffects":["local_write"]}
    });
    let path = root.join("request.json");
    fs::write(&path, serde_json::to_vec(&request).unwrap())
        .map_err(|error| (74, format!("write fixture request: {error}")))?;
    Ok(path)
}

fn write_json_ref(
    root: &Path,
    path: &str,
    schema: &str,
    kind: &str,
    value: &Value,
) -> Result<Value, (i32, String)> {
    write_ref(root, path, schema, kind, serde_json::to_vec(value).unwrap())
}

fn write_ref(
    root: &Path,
    path: &str,
    schema: &str,
    kind: &str,
    bytes: Vec<u8>,
) -> Result<Value, (i32, String)> {
    fs::write(root.join(path), &bytes)
        .map_err(|error| (74, format!("write fixture artifact: {error}")))?;
    Ok(
        json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":schema,"type":kind,"path":path,"sha256":sha256_hex(&bytes),"consumedSnapshotIdentity":SNAPSHOT}),
    )
}

fn run_sample(
    binary: &Path,
    manifest: &Path,
    root: &Path,
    request: &Path,
    out: &Path,
    materialization_ms: u64,
) -> Result<Value, (i32, String)> {
    let start = Instant::now();
    let output = Command::new(binary)
        .args(["capability", "exec", "project.orientation", "--request"])
        .arg(request)
        .arg("--out")
        .arg(out)
        .arg("--artifact-root")
        .arg(root)
        .arg("--manifest")
        .arg(manifest)
        .output()
        .map_err(|error| (69, format!("launch project.orientation: {error}")))?;
    let orientation_ms = elapsed_ms(start);
    if !output.status.success() {
        return Err((
            output.status.code().unwrap_or(70),
            format!(
                "project.orientation benchmark sample failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    let orientation_bytes = fs::read(out.join("project-orientation.json"))
        .map_err(|error| (74, format!("read orientation sample: {error}")))?;
    let orientation: Value = serde_json::from_slice(&orientation_bytes)
        .map_err(|error| (65, format!("parse orientation sample: {error}")))?;
    let coverage: Value = serde_json::from_slice(
        &fs::read(root.join("native-coverage.json"))
            .map_err(|error| (74, format!("read native coverage sample: {error}")))?,
    )
    .map_err(|error| (65, format!("parse native coverage sample: {error}")))?;
    Ok(json!({
        "wallTimeMs":materialization_ms.saturating_add(orientation_ms),
        "cost":{"materializationMs":materialization_ms,"orientationProcessMs":orientation_ms},
        "artifact":{"bytes":orientation_bytes.len(),"sha256":sha256_hex(&orientation_bytes)},
        "coverage":{"unsupportedFiles":coverage["unsupportedFiles"]},
        "orientation":orientation
    }))
}

fn evaluate(observations: &Value) -> Result<Value, AdapterError> {
    validate_observation_header(observations)?;
    let fixtures = observations["fixtures"]
        .as_array()
        .ok_or_else(|| contract("benchmark fixtures must be an array"))?;
    let expected_pairs = ["small", "medium", "large"]
        .into_iter()
        .flat_map(|size| {
            ["clean", "dirty", "provider_missing"]
                .into_iter()
                .map(move |condition| format!("{size}:{condition}"))
        })
        .collect::<BTreeSet<_>>();
    let actual_pairs = fixtures
        .iter()
        .filter_map(|fixture| {
            Some(format!(
                "{}:{}",
                fixture["size"].as_str()?,
                fixture["condition"].as_str()?
            ))
        })
        .collect::<BTreeSet<_>>();
    if fixtures.len() != 9 || actual_pairs != expected_pairs {
        return Err(contract("benchmark corpus must contain the exact small/medium/large x clean/dirty/provider_missing matrix"));
    }
    let repetitions = observations["method"]["repetitionsPerTemperature"]
        .as_u64()
        .ok_or_else(|| contract("benchmark repetitions are invalid"))?
        as usize;
    let mut typical = Vec::new();
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    let mut materialization = Vec::new();
    let mut process = Vec::new();
    let mut artifact_sizes = Vec::new();
    let mut typical_artifact_sizes = Vec::new();
    let mut field_total = 0u64;
    let mut field_ok = 0u64;
    let mut unknown_total = 0u64;
    let mut unknown_ok = 0u64;
    let mut unsupported_total = 0u64;
    let mut unsupported_ok = 0u64;
    let mut determinism_total = 0u64;
    let mut determinism_ok = 0u64;
    let mut provenance_total = 0u64;
    let mut provenance_ok = 0u64;
    for fixture in fixtures {
        let mut replay_digests = BTreeSet::new();
        for temperature in ["cold", "warm"] {
            let samples = fixture["samples"][temperature]
                .as_array()
                .ok_or_else(|| contract("benchmark samples are invalid"))?;
            if samples.len() != repetitions {
                return Err(contract(
                    "each fixture temperature must have the declared repetitions",
                ));
            }
            for sample in samples {
                let wall = sample["wallTimeMs"]
                    .as_u64()
                    .ok_or_else(|| contract("sample wall time is invalid"))?;
                if fixture["typical"] == true {
                    typical.push(wall);
                }
                if temperature == "cold" {
                    cold.push(wall);
                } else {
                    warm.push(wall);
                }
                materialization.push(
                    sample["cost"]["materializationMs"]
                        .as_u64()
                        .ok_or_else(|| contract("materialization cost is invalid"))?,
                );
                process.push(
                    sample["cost"]["orientationProcessMs"]
                        .as_u64()
                        .ok_or_else(|| contract("orientation cost is invalid"))?,
                );
                let orientation = &sample["orientation"];
                let canonical_orientation = serde_json::to_vec(orientation)
                    .map_err(|_| contract("orientation sample cannot be serialized"))?;
                let artifact_bytes = sample["artifact"]["bytes"]
                    .as_u64()
                    .ok_or_else(|| contract("sample artifact byte size is invalid"))?;
                let artifact_sha = sample["artifact"]["sha256"]
                    .as_str()
                    .ok_or_else(|| contract("sample artifact digest is invalid"))?;
                if artifact_bytes != canonical_orientation.len() as u64
                    || artifact_sha != sha256_hex(&canonical_orientation)
                {
                    return Err(contract(
                        "sample artifact size or digest does not bind the orientation bytes",
                    ));
                }
                artifact_sizes.push(artifact_bytes);
                if fixture["typical"] == true {
                    typical_artifact_sizes.push(artifact_bytes);
                }
                replay_digests.insert(artifact_sha.to_string());
                let observed_file_count = orientation["languages"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item["fileCount"].as_u64())
                            .sum::<u64>()
                    })
                    .unwrap_or(0);
                let provider_status = orientation["evidenceAvailability"]
                    .as_array()
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|item| item["evidence"] == "benchmark_provider")
                    })
                    .and_then(|item| item["status"].as_str());
                let checks = [
                    orientation["schema"] == "code-intel-project-orientation.v1",
                    orientation["snapshotIdentity"] == observations["snapshotIdentity"],
                    orientation["purpose"]["status"] == "unknown",
                    orientation["purpose"]["evidence"] == json!([]),
                    orientation["activeChange"]["status"] == fixture["expected"]["activeChange"],
                    observed_file_count.saturating_add(2)
                        == fixture["expected"]["fileCount"]
                            .as_u64()
                            .unwrap_or(u64::MAX),
                    provider_status == fixture["expected"]["providerStatus"].as_str(),
                    orientation["languages"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty()),
                    orientation["risks"].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["code"] == "structural_evidence_unavailable")
                            == (fixture["condition"] == "provider_missing")
                    }),
                ];
                field_total += checks.len() as u64;
                field_ok += checks.iter().filter(|value| **value).count() as u64;
                let expected_unknowns = fixture["expected"]["unknownFields"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>();
                let actual_unknowns = orientation["unknowns"]
                    .as_array()
                    .ok_or_else(|| contract("orientation unknowns are invalid"))?
                    .iter()
                    .filter_map(|item| item["field"].as_str())
                    .collect::<BTreeSet<_>>();
                unknown_total += actual_unknowns.len().max(expected_unknowns.len()) as u64;
                unknown_ok += actual_unknowns.intersection(&expected_unknowns).count() as u64;
                let expected_unsupported = fixture["expected"]["unsupportedFiles"]
                    .as_array()
                    .ok_or_else(|| contract("expected unsupported files are invalid"))?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>();
                let actual_unsupported = sample["coverage"]["unsupportedFiles"]
                    .as_array()
                    .ok_or_else(|| contract("sample unsupported files are invalid"))?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>();
                unsupported_total +=
                    actual_unsupported.len().max(expected_unsupported.len()) as u64;
                unsupported_ok += actual_unsupported
                    .intersection(&expected_unsupported)
                    .count() as u64;
                for claim in claim_nodes(orientation)? {
                    provenance_total += 1;
                    if claim["provenance"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty())
                    {
                        provenance_ok += 1;
                    }
                }
            }
        }
        determinism_total += 1;
        if replay_digests.len() == 1 {
            determinism_ok += 1;
        }
    }
    if provenance_ok != provenance_total {
        return Err(contract(
            "fast orientation result without complete claim provenance is rejected",
        ));
    }
    let field_correctness = ratio(field_ok, field_total);
    let unresolved_coverage = ratio(unknown_ok, unknown_total);
    let unsupported_coverage = ratio(unsupported_ok, unsupported_total);
    let deterministic_replay_rate = ratio(determinism_ok, determinism_total);
    let provenance_completeness = ratio(provenance_ok, provenance_total);
    let typical_p95 = percentile(&mut typical, 95);
    let verdict = if typical_p95 <= TARGET_MS
        && field_correctness == 1.0
        && unresolved_coverage == 1.0
        && unsupported_coverage == 1.0
        && deterministic_replay_rate == 1.0
        && provenance_completeness == 1.0
    {
        "pass"
    } else {
        "fail"
    };
    Ok(json!({
        "schema":"code-intel-project-orientation-benchmark.v1","verdict":verdict,"target":{"typicalP95WallTimeMs":TARGET_MS,"llm":"disabled"},
        "corpus":{"fixtureCount":fixtures.len(),"sizes":["small","medium","large"],"conditions":["clean","dirty","provider_missing"],"typicalDefinition":"small_and_medium_all_conditions","stressDefinition":"large_all_conditions"},
        "method":observations["method"],"environment":observations["environment"],
        "latency":{"typical":{"p50WallTimeMs":percentile(&mut typical,50),"p95WallTimeMs":typical_p95},"cold":{"p50WallTimeMs":percentile(&mut cold,50),"p95WallTimeMs":percentile(&mut cold,95)},"warm":{"p50WallTimeMs":percentile(&mut warm,50),"p95WallTimeMs":percentile(&mut warm,95)}},
        "artifactSize":{"typical":{"p50Bytes":percentile(&mut typical_artifact_sizes,50),"p95Bytes":percentile(&mut typical_artifact_sizes,95)},"all":{"p50Bytes":percentile(&mut artifact_sizes,50),"p95Bytes":percentile(&mut artifact_sizes,95),"maxBytes":artifact_sizes.iter().copied().max().unwrap_or(0)}},
        "quality":{"fieldCorrectness":field_correctness,"unknownPrecision":unresolved_coverage,"unresolvedCoverage":unresolved_coverage,"unsupportedCoverage":unsupported_coverage,"deterministicReplayRate":deterministic_replay_rate,"provenanceCompleteness":provenance_completeness},
        "costCenters":[{"name":"fixture_materialization","p50Ms":percentile(&mut materialization,50),"p95Ms":percentile(&mut materialization,95)},{"name":"a01_process_and_orientation","p50Ms":percentile(&mut process,50),"p95Ms":percentile(&mut process,95)}],
        "limitations":["this run was not performed on a clean machine","cold means fresh materialization, not an operating-system page-cache flush","all measurements are sequential and local; hosted services and LLMs are excluded","provider conditions are deterministic committed benchmark evidence, not live hosted-provider probes"]
    }))
}

fn validate_observation_header(value: &Value) -> Result<(), AdapterError> {
    if value["schema"] != "code-intel-project-orientation-benchmark-observations.v1"
        || value["snapshotIdentity"] != SNAPSHOT
        || value["method"]["clock"] != "std::time::Instant"
        || value["method"]["execution"] != "sequential_child_process"
        || value["method"]["concurrency"] != 1
        || value["method"]["llm"] != "disabled"
        || value["method"]["percentile"] != "nearest_rank"
        || value["environment"]["cleanMachine"] != false
    {
        return Err(contract(
            "benchmark observation method is not reproducible or no-LLM",
        ));
    }
    Ok(())
}

fn claim_nodes<'a>(orientation: &'a Value) -> Result<Vec<&'a Value>, AdapterError> {
    let mut claims = vec![
        &orientation["identity"],
        &orientation["purpose"],
        &orientation["activeChange"],
        &orientation["confidence"],
    ];
    for field in [
        "languages",
        "boundaries",
        "entryPoints",
        "commands",
        "evidenceAvailability",
        "risks",
        "unknowns",
    ] {
        claims.extend(
            orientation[field]
                .as_array()
                .ok_or_else(|| contract(format!("orientation {field} is invalid")))?,
        );
    }
    Ok(claims)
}

fn percentile(values: &mut [u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let rank = (percentile * values.len()).div_ceil(100).max(1);
    values[rank - 1]
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn publish(out: &Path, relative: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    fs::create_dir_all(out)
        .map_err(|error| AdapterError::Io(format!("create benchmark output: {error}")))?;
    let path = out.join(relative);
    if path.exists() {
        return Err(AdapterError::Io(format!(
            "refusing to overwrite benchmark artifact: {relative}"
        )));
    }
    fs::write(path, bytes).map_err(|error| AdapterError::Io(format!("write {relative}: {error}")))
}

fn render(report: &Value) -> String {
    format!("# Project Orientation Benchmark\n\n- Verdict: {}\n- Typical p50: {} ms\n- Typical p95: {} ms\n- Typical artifact p95: {} bytes\n- Field correctness: {}\n- Unresolved coverage: {}\n- Unsupported coverage: {}\n- Deterministic replay rate: {}\n- Provenance completeness: {}\n- Clean machine: false\n",
        report["verdict"].as_str().unwrap_or("fail"), report["latency"]["typical"]["p50WallTimeMs"], report["latency"]["typical"]["p95WallTimeMs"], report["artifactSize"]["typical"]["p95Bytes"], report["quality"]["fieldCorrectness"], report["quality"]["unresolvedCoverage"], report["quality"]["unsupportedCoverage"], report["quality"]["deterministicReplayRate"], report["quality"]["provenanceCompleteness"])
}

fn default_registry() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("orchestration/integrations.json")
}

fn contract(message: impl Into<String>) -> AdapterError {
    AdapterError::Contract(message.into())
}

fn adapter_message(error: AdapterError) -> String {
    match error {
        AdapterError::InvalidOptions(message)
        | AdapterError::Contract(message)
        | AdapterError::Unavailable(message)
        | AdapterError::Internal(message)
        | AdapterError::Io(message) => message,
    }
}
