use std::path::PathBuf;
use std::process::Command;

use super::read_json;

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
            "  if not exist \"%repo%\\.sentrux\\cache\" mkdir \"%repo%\\.sentrux\\cache\"\r\n",
            "  > \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo {\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"tool\": \"sentrux-lite\",\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"quality_signal\": 100,\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"coupling_score\": 1,\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"cycle_count\": 0,\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"god_file_count\": 0,\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"complex_fn_count\": 0,\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"cross_module_edges\": 1,\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo   \"total_import_edges\": 10\r\n",
            "  >> \"%repo%\\.sentrux\\cache\\lite-baseline.json\" echo }\r\n",
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
            "  mkdir -p \"$repo/.sentrux/cache\"\n",
            "  printf '%s\\n' '{' '  \"tool\": \"sentrux-lite\",' '  \"quality_signal\": 100,' '  \"coupling_score\": 1,' '  \"cycle_count\": 0,' '  \"god_file_count\": 0,' '  \"complex_fn_count\": 0,' '  \"cross_module_edges\": 1,' '  \"total_import_edges\": 10' '}' > \"$repo/.sentrux/cache/lite-baseline.json\"\n",
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
    // The agent tool pins the session gate to the repository's lite core and
    // honors SENTRUX_CORE_EXE as the explicit override (issue #182), so the
    // fake CLI is injected through that seam; the PATH prepend stays for the
    // last-resort `sentrux` lookup.
    #[cfg(windows)]
    let fake_cli = tree.fake_bin().join("sentrux.cmd");
    #[cfg(not(windows))]
    let fake_cli = tree.fake_bin().join("sentrux");
    let output = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(legacy_session_script())
        .arg(operation)
        .arg(tree.repo())
        .args(["-SessionId", session_id])
        .env("PATH", path)
        .env("SENTRUX_CORE_EXE", &fake_cli)
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
        read_json(&tree.repo().join(".sentrux/cache/lite-baseline.json"))["quality_signal"],
        100
    );
    assert!(
        !tree.repo().join(".sentrux/baseline.json").exists(),
        "session gate must not create the native engine's .sentrux/baseline.json (issue #182)"
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
    assert!(
        !tree.repo().join(".sentrux/baseline.json").exists(),
        "session gate must not create the native engine's .sentrux/baseline.json (issue #182)"
    );
}
