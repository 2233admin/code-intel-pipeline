use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
}

#[test]
fn root_help_leads_with_the_compiled_primary_entry() {
    let output = Command::new(binary())
        .arg("--help")
        .output()
        .expect("run code-intel --help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("code-intel ."));
    assert!(stdout.contains("code-intel <path> --mode lite|normal|full"));
    assert!(!stdout.contains("legacy/invoke-code-intel.ps1"));
}

#[test]
fn root_entry_rejects_a_missing_repository_with_usage_exit_code() {
    let missing =
        std::env::temp_dir().join(format!("code-intel-missing-repo-{}", std::process::id()));
    let output = Command::new(binary())
        .arg(&missing)
        .arg("--mode")
        .arg("lite")
        .output()
        .expect("run code-intel with a missing repository");

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(stderr.contains("repository path is not a directory:"));
    assert!(!stderr.contains("unknown command"));
    // A usage error is the user's mistake, not ours, so it carries no source
    // location.
    assert!(!stderr.contains("main.rs:"), "{stderr}");
}

#[test]
fn root_entry_keeps_json_machine_readable_on_usage_errors() {
    let missing = std::env::temp_dir().join(format!(
        "code-intel-json-missing-repo-{}",
        std::process::id()
    ));
    let output = Command::new(binary())
        .arg(&missing)
        .args(["--mode", "lite", "--json"])
        .output()
        .expect("run code-intel JSON error path");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stderr.is_empty());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("error output is JSON");
    assert_eq!(result["schema"], "code-intel-primary-result.v1");
    assert_eq!(result["outcome"], "error");
    assert_eq!(result["exitCode"], 64);
    assert!(result["diagnostic"]
        .as_str()
        .is_some_and(|message| message.contains("repository path is not a directory:")));
}

#[test]
fn named_commands_are_not_misclassified_as_repository_paths() {
    let output = Command::new(binary())
        .args(["orchestrate", "--action", "List", "--json"])
        .output()
        .expect("run an existing named command");

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(!stderr.contains("unknown primary entry argument"));
}

/// End-to-end cover for the stable wrapper: the default route with no
/// subcommand, the summary it prints, and what the authoritative index does
/// with a run that failed.
///
/// This was `legacy/scripts/tests/test-stable-wrapper-e2e.ps1`. It never tested
/// PowerShell — every assertion was already about the compiled binary — but it
/// was the only cover for the summary lines `main.rs` prints.
#[test]
fn stable_wrapper_publishes_a_completed_run_then_keeps_a_failed_one_out_of_the_index() {
    let root = std::env::temp_dir().join(format!(
        "code-intel-wrapper-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_nanos()
    ));
    let repo = root.join("fixture-repo");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(repo.join("assets")).expect("fixture assets");
    std::fs::create_dir_all(repo.join("src")).expect("fixture src");
    std::fs::create_dir_all(repo.join(".sentrux")).expect("fixture sentrux");
    std::fs::write(repo.join("README.md"), "stable wrapper fixture").expect("fixture readme");
    std::fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}").expect("fixture source");
    std::fs::write(
        repo.join(".sentrux/rules.toml"),
        "[constraints]\nmax_cycles = 0\nmax_coupling = \"F\"\nmax_cc = 100\nno_god_files = false\n",
    )
    .expect("fixture rules");
    // An unsupported binary file must not be grounds for rejecting the run.
    std::fs::write(repo.join("assets/logo.png"), [0x89, 0x50, 0x4e, 0x47, 0xff])
        .expect("fixture binary asset");

    // Provision the baseline with the built-in engine the authoritative run
    // gates with; a PATH-resolved external Sentrux writes a foreign baseline
    // identity and trips the engine-mismatch check.
    for operation in ["save_baseline", "check"] {
        let output = Command::new(binary())
            .args(["sentrux", "--operation", operation, "--repo"])
            .arg(&repo)
            .output()
            .expect("run sentrux");
        assert!(
            output.status.success(),
            "sentrux {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    commit_fixture(&repo, "baseline");

    let (code, output) = run_wrapper(&repo, &artifacts);
    // The doctor reports three distinct causes (`doctor_adapter.rs::diagnosis`).
    // Two of them — bootstrap readiness and provider conformance — describe the
    // machine's tools, and this route passes the doctor no flags to fix that up,
    // so on a host that is missing or mismatching a pinned tool they say nothing
    // about the wrapper. The third, manifest reconciliation, is about this
    // repository's own orchestration manifest and is never tolerated here, nor
    // is any second domain failure. CI installs the pinned tools, so everything
    // below still runs where it counts.
    let host_toolchain_gap = code != Some(0)
        && output.matches("Domain failure:").count() == 1
        && output.contains("Domain failure: doctor")
        && !output.contains("manifest reconciliation failed")
        && ["bootstrap readiness failed", "provider conformance failed"]
            .iter()
            .any(|cause| output.contains(cause));
    if host_toolchain_gap {
        eprintln!(
            "host toolchain is incomplete, so the authoritative route is not asserted:\n{output}"
        );
        let _ = std::fs::remove_dir_all(root);
        return;
    }
    assert_eq!(
        code,
        Some(0),
        "wrapper rejected a clean repository: {output}"
    );
    assert!(
        !output.contains("legacy compatibility pipeline"),
        "the default route still executed the legacy pipeline: {output}"
    );
    for marker in ["[PASS]", "Run evidence:", "Outcome:", "completed"] {
        assert!(output.contains(marker), "summary lacks {marker}: {output}");
    }

    let authority = artifacts.join("fixture-repo");
    let completed_run = latest_core_run(&authority);
    let completed = manifest_of(&completed_run);
    assert_eq!(completed["outcome"], "completed", "manifest={completed}");
    for node in ["evidence.graph", "evidence.sentrux", "diagnosis.hospital"] {
        assert_eq!(
            completed["nodes"][node]["status"], "succeeded",
            "default spine did not complete {node}: {completed}"
        );
        assert_eq!(
            completed["nodes"][node]["verdict"], "pass",
            "default spine did not pass {node}: {completed}"
        );
    }

    let doctor = completed["nodes"]["doctor"]["artifacts"]
        .as_array()
        .expect("doctor artifacts")
        .iter()
        .find(|artifact| artifact["type"] == "doctor.observation")
        .unwrap_or_else(|| panic!("no authoritative doctor observation: {completed}"))
        .clone();
    let observation = read_json(&completed_run.join(doctor["path"].as_str().expect("path")));
    assert_eq!(
        observation["environmentPolicy"]["policy"]["requireRepowise"], false,
        "the default route repowise skip did not reach the authoritative doctor policy"
    );

    let name = completed_run
        .file_name()
        .and_then(|value| value.to_str())
        .expect("run name")
        .to_string();
    assert_single_index_entry(&artifacts, &name);

    let query = Command::new(binary())
        .args(["artifact", "query", "--artifact-root"])
        .arg(&artifacts)
        .args(["--repo", "fixture-repo", "--repo-path"])
        .arg(&repo)
        .args(["--type", "observed.evidence.payload"])
        .output()
        .expect("run artifact query");
    assert!(
        query.status.success(),
        "provider payload query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: serde_json::Value = serde_json::from_slice(&query.stdout).expect("query is JSON");
    assert_eq!(query["freshness"]["status"], "current", "query={query}");
    assert!(
        query["matches"].as_array().expect("matches").len() >= 2,
        "query={query}"
    );

    // Invalid UTF-8 in a source file fails the native-code node, and the run
    // has to stay visible for audit without becoming authoritative.
    std::fs::write(repo.join("broken.rs"), [0xff, 0xfe, 0xfd]).expect("invalid source");
    commit_fixture(&repo, "invalid-utf8-source");

    let (code, output) = run_wrapper(&repo, &artifacts);
    assert_ne!(
        code,
        Some(0),
        "wrapper hid an authoritative failure: {output}"
    );
    for marker in [
        "[FAIL]",
        "process_failed",
        "evidence.native-code",
        "Run evidence:",
    ] {
        assert!(
            output.contains(marker),
            "failure summary lacks {marker}: {output}"
        );
    }

    let failed_run = latest_core_run(&authority);
    assert_ne!(
        failed_run, completed_run,
        "the failed run was not retained for audit"
    );
    let failed = manifest_of(&failed_run);
    assert_eq!(failed["outcome"], "process_failed", "manifest={failed}");
    assert_eq!(
        failed["nodes"]["evidence.native-code"]["status"], "process_failed",
        "manifest={failed}"
    );

    // The last completed run keeps the authority, and the failed one is
    // classified outside the index rather than dropped.
    let index = assert_single_index_entry(&artifacts, &name);
    let failed_name = failed_run.file_name().and_then(|value| value.to_str());
    assert!(
        index["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|item| {
                item["repo"] == "fixture-repo"
                    && item["run"].as_str() == failed_name
                    && item["classification"] == "non_completed"
                    && item["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("process_failed"))
            }),
        "the failed run was not classified outside the authoritative index: {index}"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// `git init` is allowed to fail on the second call; everything else is not.
fn commit_fixture(repo: &std::path::Path, message: &str) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["init", "--quiet"])
        .output();
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "user.name=CodeIntelTest",
            "-c",
            "user.email=code-intel-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(&args)
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The wrapper takes the repository from the working directory, which is the
/// route a user actually types.
fn run_wrapper(repo: &std::path::Path, artifacts: &std::path::Path) -> (Option<i32>, String) {
    let output = Command::new(binary())
        .arg("--artifact-root")
        .arg(artifacts)
        .current_dir(repo)
        .output()
        .expect("run the stable wrapper");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code(), text)
}

/// Authoritative runs are published as `<name>-core`; staging directories and
/// non-committed runs must never be picked up here.
fn latest_core_run(authority: &std::path::Path) -> PathBuf {
    let mut runs: Vec<PathBuf> = std::fs::read_dir(authority)
        .unwrap_or_else(|error| panic!("no authority at {}: {error}", authority.display()))
        .filter_map(|entry| {
            let path = entry.expect("read authority entry").path();
            let name = path.file_name()?.to_str()?.to_string();
            (path.is_dir() && name.ends_with("-core")).then_some(path)
        })
        .collect();
    runs.sort();
    runs.pop()
        .unwrap_or_else(|| panic!("no authoritative run under {}", authority.display()))
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("unreadable {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("unparsable {}: {error}", path.display()))
}

/// The manifest is reached through the marker rather than by name, so this
/// fails if the marker ever stops binding it.
fn manifest_of(run: &std::path::Path) -> serde_json::Value {
    let marker = read_json(&run.join("run-complete.json"));
    read_json(&run.join(marker["manifest"]["path"].as_str().expect("manifest path")))
}

fn assert_single_index_entry(artifacts: &std::path::Path, run: &str) -> serde_json::Value {
    let index = read_json(&artifacts.join("index.json"));
    let entries: Vec<&serde_json::Value> = index["entries"]
        .as_array()
        .expect("index entries")
        .iter()
        .filter(|entry| entry["repo"] == "fixture-repo")
        .collect();
    assert_eq!(entries.len(), 1, "index={index}");
    assert_eq!(entries[0]["run"].as_str(), Some(run), "index={index}");
    assert_eq!(entries[0]["outcome"], "completed", "index={index}");
    index
}

struct LegacySessionTemp(PathBuf);

impl LegacySessionTemp {
    fn new(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-session-gate-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(path.join("fake-bin")).expect("create hermetic tree");
        std::fs::create_dir_all(path.join("repo/src")).expect("create repository tree");
        std::fs::write(path.join("repo/src/lib.rs"), "pub fn baseline() {}\n")
            .expect("write baseline source");
        write_fake_sentrux(&path.join("fake-bin"));
        Self(path)
    }

    fn repo(&self) -> PathBuf {
        self.0.join("repo")
    }

    fn fake_bin(&self) -> PathBuf {
        self.0.join("fake-bin")
    }
}

impl Drop for LegacySessionTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_fake_sentrux(fake_bin: &std::path::Path) {
    #[cfg(windows)]
    let path = fake_bin.join("sentrux.cmd");
    #[cfg(not(windows))]
    let path = fake_bin.join("sentrux");

    #[cfg(windows)]
    std::fs::write(
        &path,
        concat!(
            "@echo off\r\n",
            "setlocal EnableExtensions\r\n",
            "set \"save=0\"\r\n",
            "set \"repo=\"\r\n",
            ":args\r\n",
            "if \"%~1\"==\"\" goto args_done\r\n",
            "if /I \"%~1\"==\"--save\" set \"save=1\"\r\n",
            "set \"repo=%~1\"\r\n",
            "shift\r\n",
            "goto args\r\n",
            ":args_done\r\n",
            "if \"%save%\"==\"1\" (\r\n",
            "  if not exist \"%repo%\\.sentrux\" mkdir \"%repo%\\.sentrux\"\r\n",
            "  > \"%repo%\\.sentrux\\baseline.json\" echo {\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"quality_signal\": 100,\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"coupling_score\": 1,\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"cycle_count\": 0,\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"god_file_count\": 0,\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"complex_fn_count\": 0,\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"cross_module_edges\": 1,\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo   \"total_import_edges\": 10\r\n",
            "  >> \"%repo%\\.sentrux\\baseline.json\" echo }\r\n",
            ")\r\n",
            "set \"quality=100\"\r\n",
            "if exist \"%repo%\\src\\regression.marker\" set \"quality=99\"\r\n",
            "echo [resolve] 10 resolved, 0 unresolved\r\n",
            "echo [build_graphs] 5 files ^| 10 import, 3 call, 0 inherit edges\r\n",
            "echo Quality: 100 -^> %quality%\r\n",
            "echo Coupling: 1 -^> 1\r\n",
            "echo Cycles: 0 -^> 0\r\n",
            "echo God files: 0 -^> 0\r\n",
            "echo Distance from Main Sequence: 0.01\r\n",
            "exit /b 0\r\n",
        ),
    )
    .expect("write fake sentrux");

    #[cfg(not(windows))]
    std::fs::write(
        &path,
        concat!(
            "#!/bin/sh\n",
            "save=0\n",
            "repo=\n",
            "for arg in \"$@\"; do\n",
            "  [ \"$arg\" = \"--save\" ] && save=1\n",
            "  repo=$arg\n",
            "done\n",
            "if [ \"$save\" = 1 ]; then\n",
            "  mkdir -p \"$repo/.sentrux\"\n",
            "  printf '%s\\n' '{' '  \"quality_signal\": 100,' '  \"coupling_score\": 1,' '  \"cycle_count\": 0,' '  \"god_file_count\": 0,' '  \"complex_fn_count\": 0,' '  \"cross_module_edges\": 1,' '  \"total_import_edges\": 10' '}' > \"$repo/.sentrux/baseline.json\"\n",
            "fi\n",
            "quality=100\n",
            "[ -f \"$repo/src/regression.marker\" ] && quality=99\n",
            "printf '%s\\n' '[resolve] 10 resolved, 0 unresolved' '[build_graphs] 5 files | 10 import, 3 call, 0 inherit edges' \"Quality: 100 -> $quality\" 'Coupling: 1 -> 1' 'Cycles: 0 -> 0' 'God files: 0 -> 0' 'Distance from Main Sequence: 0.01'\n",
        ),
    )
    .expect("write fake sentrux");

    #[cfg(unix)]
    {
        let status = Command::new("chmod")
            .arg("755")
            .arg(&path)
            .status()
            .expect("run chmod for fake sentrux");
        assert!(status.success(), "make fake sentrux executable");
    }
}

fn legacy_session_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("legacy/Invoke-SentruxAgentTool.ps1")
}

fn invoke_legacy_session(
    tree: &LegacySessionTemp,
    operation: &str,
    session_id: &str,
) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let path = std::env::join_paths(
        std::iter::once(tree.fake_bin()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .expect("compose hermetic PATH");
    let output = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(legacy_session_script())
        .arg(operation)
        .arg(tree.repo())
        .args(["-SessionId", session_id])
        .env("PATH", path)
        .output()
        .expect("invoke real legacy session gate");
    (output.status.code(), output.stdout, output.stderr)
}

fn parse_legacy_session_json(output: &(Option<i32>, Vec<u8>, Vec<u8>)) -> serde_json::Value {
    serde_json::from_slice(&output.1).unwrap_or_else(|error| {
        panic!(
            "session gate must emit JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.1),
            String::from_utf8_lossy(&output.2)
        )
    })
}

fn assert_exact_keys(value: &serde_json::Value, expected: &str, label: &str) {
    let mut actual = value
        .as_object()
        .unwrap_or_else(|| panic!("{label} must be an object: {value}"))
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut expected = expected.split(',').collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{label} keys");
}

fn assert_session_document_shape(value: &serde_json::Value, phase: &str) {
    let top_level = match phase {
        "session_start" => {
            "tool,session_id,path,status,quality_signal,bottleneck,started_at,gate"
        }
        "session_end" => {
            "tool,session_id,path,pass,signal_before,signal_after,delta,summary,metrics_observed_count,backfilled_metrics,ended_at,gate,rules"
        }
        _ => panic!("unsupported session phase: {phase}"),
    };
    assert_exact_keys(value, top_level, phase);
    assert_exact_keys(
        &value["gate"],
        "pass,status,exit_code,duration_ms,metrics,baseline,bottleneck,raw_output,metrics_observed_count,backfilled_metrics",
        &format!("{phase}.gate"),
    );
    assert_exact_keys(
        &value["gate"]["metrics"],
        "quality_before,quality_signal,coupling_before,coupling,cycles_before,cycles,god_files_before,god_files,distance_from_main_sequence,no_degradation,violations,scan",
        &format!("{phase}.gate.metrics"),
    );
    assert_exact_keys(
        &value["gate"]["metrics"]["scan"],
        "resolvedImports,unresolvedImports,files,importEdges,callEdges,inheritEdges",
        &format!("{phase}.gate.metrics.scan"),
    );
    assert_exact_keys(
        &value["gate"]["baseline"],
        "path,quality_signal,coupling,cycles,god_files,complex_functions,total_import_edges,cross_module_edges",
        &format!("{phase}.gate.baseline"),
    );
}

#[test]
fn real_session_start_change_end_pass_contract_is_stable() {
    let tree = LegacySessionTemp::new("pass");
    let session_id = "pass-contract";

    let start = invoke_legacy_session(&tree, "session_start", session_id);
    assert_eq!(start.0, Some(0));
    let start_json = parse_legacy_session_json(&start);
    assert_session_document_shape(&start_json, "session_start");
    assert_eq!(start_json["tool"], "session_start");
    assert_eq!(start_json["session_id"], session_id);
    assert_eq!(start_json["status"], "Baseline saved");
    assert_eq!(start_json["gate"]["pass"], true);
    assert_eq!(start_json["gate"]["exit_code"], 0);
    assert_eq!(start_json["gate"]["baseline"]["quality_signal"], 100);
    let records = tree.repo().join(".sentrux/agent-sessions");
    let persisted_start = read_json(&records.join(format!("{session_id}.start.json")));
    assert_eq!(
        persisted_start, start_json,
        "persisted start differs from stdout"
    );
    assert_eq!(persisted_start["session_id"], session_id);
    assert_eq!(persisted_start["quality_signal"], 100);
    assert_eq!(
        read_json(&tree.repo().join(".sentrux/baseline.json"))["quality_signal"],
        100
    );

    std::fs::write(
        tree.repo().join("src/lib.rs"),
        "pub fn baseline() {}\npub fn changed() {}\n",
    )
    .expect("make a real repository change");

    let end = invoke_legacy_session(&tree, "session_end", session_id);
    assert_eq!(end.0, Some(0));
    let end_json = parse_legacy_session_json(&end);
    assert_session_document_shape(&end_json, "session_end");
    assert_eq!(end_json["tool"], "session_end");
    assert_eq!(end_json["session_id"], session_id);
    assert_eq!(end_json["pass"], true);
    assert_eq!(end_json["delta"], 0);
    assert_eq!(
        end_json["summary"],
        "No structural degradation during this session"
    );
    assert_eq!(end_json["gate"]["exit_code"], 0);

    assert_eq!(
        read_json(&records.join(format!("{session_id}.end.json"))),
        end_json,
        "persisted end differs from stdout"
    );
}

#[test]
fn real_session_start_change_end_failure_is_json_with_zero_process_exit() {
    let tree = LegacySessionTemp::new("fail");
    let session_id = "fail-contract";

    let start = invoke_legacy_session(&tree, "session_start", session_id);
    assert_eq!(start.0, Some(0));
    let start_json = parse_legacy_session_json(&start);
    assert_session_document_shape(&start_json, "session_start");
    assert_eq!(start_json["gate"]["pass"], true);

    std::fs::write(tree.repo().join("src/regression.marker"), "regressed\n")
        .expect("make a real repository change");

    let end = invoke_legacy_session(&tree, "session_end", session_id);
    assert_eq!(
        end.0,
        Some(0),
        "legacy gate currently reports domain failure in JSON, not process status"
    );
    let end_json = parse_legacy_session_json(&end);
    assert_session_document_shape(&end_json, "session_end");
    assert_eq!(end_json["tool"], "session_end");
    assert_eq!(end_json["pass"], false);
    assert_eq!(end_json["signal_before"], 100);
    assert_eq!(end_json["signal_after"], 99);
    assert_eq!(end_json["delta"], -1);
    assert_eq!(end_json["summary"], "Quality degraded during this session");
    assert_eq!(end_json["gate"]["pass"], true);
    assert_eq!(end_json["gate"]["exit_code"], 0);
    let records = tree.repo().join(".sentrux/agent-sessions");
    assert_eq!(
        read_json(&records.join(format!("{session_id}.start.json"))),
        start_json,
        "persisted failure-case start differs from stdout"
    );
    assert_eq!(
        read_json(&records.join(format!("{session_id}.end.json"))),
        end_json,
        "persisted failure-case end differs from stdout"
    );
}
