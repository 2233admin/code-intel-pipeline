mod common;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SOURCE_REVISION: &str = "a56ad2c39a617ebb72447a98c0087a765758c296";

fn head_parity_fixture() -> serde_json::Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli-head-parity.v2.json");
    serde_json::from_slice(
        &std::fs::read(&path)
            .unwrap_or_else(|error| panic!("read HEAD parity fixture {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parse HEAD parity fixture {}: {error}", path.display()))
}

struct FixtureRepository(PathBuf);

impl FixtureRepository {
    fn create() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-cli-head-parity-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(path.join("src")).expect("create route fixture repo");
        std::fs::write(path.join("src/lib.rs"), "pub fn fixture() {}\n")
            .expect("write route fixture source");
        commit_fixture(&path, "route golden fixture");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for FixtureRepository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn execute(argv: &[serde_json::Value], fixture_repo: &Path) -> Output {
    let missing_repo = "__code_intel_parity_missing_repository_v1__";
    assert!(!Path::new(missing_repo).exists());
    let mut command = common::cli();
    for argument in argv {
        let argument = argument.as_str().expect("argv item must be a string");
        match argument {
            "@missing-repo@" => {
                command.arg(missing_repo);
            }
            "@fixture-repo@" => {
                command.arg(fixture_repo);
            }
            argument => {
                command.arg(argument);
            }
        }
    }
    command.output().expect("spawn code-intel parity case")
}

fn assert_exact_process_result(
    label: &str,
    argv: &[serde_json::Value],
    expected: &serde_json::Value,
    fixture_repo: &Path,
) {
    let output = execute(argv, fixture_repo);
    assert_eq!(
        output.status.code(),
        expected["exitCode"].as_i64().map(|code| code as i32),
        "{label} exit"
    );
    if expected["stdoutSemantics"] == "version-minimum" {
        assert_version_at_least(
            label,
            &output.stdout,
            expected["stdoutUtf8"]
                .as_str()
                .expect("stdoutUtf8")
                .as_bytes(),
        );
    } else {
        assert_eq!(
            output.stdout,
            expected["stdoutUtf8"]
                .as_str()
                .expect("stdoutUtf8")
                .as_bytes(),
            "{label} stdout bytes"
        );
    }
    assert_eq!(
        output.stderr,
        expected["stderrUtf8"]
            .as_str()
            .expect("stderrUtf8")
            .as_bytes(),
        "{label} stderr bytes"
    );
}

fn assert_version_at_least(label: &str, actual: &[u8], minimum: &[u8]) {
    let parse = |bytes: &[u8]| {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .unwrap_or_else(|error| panic!("{label} version output is not JSON: {error}"));
        value["version"]
            .as_str()
            .unwrap_or_else(|| panic!("{label} version output omits version"))
            .split(['.', '-'])
            .take(3)
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
            .unwrap_or_else(|| panic!("{label} version is not numeric"))
    };
    let actual = parse(actual);
    let minimum = parse(minimum);
    let at_least = (0..3)
        .map(|index| {
            (
                actual.get(index).copied().unwrap_or_default(),
                minimum.get(index).copied().unwrap_or_default(),
            )
        })
        .find(|(actual, minimum)| actual != minimum)
        .is_none_or(|(actual, minimum)| actual > minimum);
    assert!(at_least, "{label} version is below the fixture floor");
}

#[test]
fn every_ordinary_case_matches_old_head_exactly() {
    let fixture = head_parity_fixture();
    assert_eq!(fixture["schema"], "code-intel-cli-head-parity-fixture.v2");
    assert_eq!(fixture["sourceRevision"], SOURCE_REVISION);
    assert_eq!(fixture["capture"]["encoding"], "utf-8");
    assert_eq!(fixture["capture"]["normalizations"], serde_json::json!([]));
    assert_eq!(
        fixture["capture"]["recipe"]["steps"]
            .as_array()
            .expect("capture recipe steps")
            .len(),
        5
    );
    let cases = fixture["cases"].as_array().expect("exact parity cases");
    assert_eq!(
        cases.len(),
        fixture["exactParityCaseCount"]
            .as_u64()
            .expect("exact parity case count") as usize
    );
    // One case ("repin clean success") moved from here into
    // `intentionalDeltas` when gate G1 added `scanCoverage` to repin's
    // report -- see `every_intentional_delta_is_documented_and_reproduced`.
    assert_eq!(cases.len(), 49);

    let repo = FixtureRepository::create();
    for case in cases {
        let label = case["name"].as_str().expect("case name");
        let argv = case["argv"].as_array().expect("case argv");
        assert_exact_process_result(label, argv, case, repo.path());
    }
}

#[test]
fn every_legacy_command_spelling_honors_trailing_help() {
    let fixture = head_parity_fixture();
    let matrix = &fixture["trailingHelpParity"];
    let expected = &matrix["expected"];
    let repo = FixtureRepository::create();

    for spelling in matrix["commandSpellings"]
        .as_array()
        .expect("legacy command spellings")
    {
        for flag in matrix["helpFlags"].as_array().expect("help flags") {
            let argv = [spelling.clone(), flag.clone()];
            let label = format!(
                "{} {}",
                spelling.as_str().expect("command spelling"),
                flag.as_str().expect("help flag")
            );
            assert_exact_process_result(&label, &argv, expected, repo.path());
        }
    }
}

/// Every intentional CLI-output change since `sourceRevision` must be
/// enumerated here by contract id pair, not just left to accumulate in the
/// fixture: an unexpected third entry (or a missing expected one) fails
/// this list before it can fail silently. Each entry's `new` bytes must
/// also actually reproduce against a live run, the same guarantee
/// `assert_exact_process_result` gives the plain (non-delta) cases.
const EXPECTED_INTENTIONAL_DELTAS: &[(&str, &str)] = &[
    ("text-format:help-full.v1", "text-format:help-full.v3"),
    ("json-format:repin-report.v1", "json-format:repin-report.v2"),
    (
        "text-format:run-namespace-usage.v1",
        "text-format:primary-run-alias-error.v1",
    ),
];

#[test]
fn every_intentional_delta_is_documented_and_reproduced() {
    let fixture = head_parity_fixture();
    let deltas = fixture["intentionalDeltas"]
        .as_array()
        .expect("intentional deltas");
    let seen: Vec<(String, String)> = deltas
        .iter()
        .map(|delta| {
            (
                delta["oldContractId"]
                    .as_str()
                    .expect("oldContractId")
                    .to_string(),
                delta["newContractId"]
                    .as_str()
                    .expect("newContractId")
                    .to_string(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = EXPECTED_INTENTIONAL_DELTAS
        .iter()
        .map(|(old, new)| (old.to_string(), new.to_string()))
        .collect();
    assert_eq!(
        seen, expected,
        "intentionalDeltas must exactly match the documented, reviewed set \
         (add to EXPECTED_INTENTIONAL_DELTAS alongside the fixture when a new one is intentional)"
    );

    let repo = FixtureRepository::create();
    for delta in deltas {
        assert!(
            delta["old"]["stdoutUtf8"] != delta["new"]["stdoutUtf8"]
                || delta["old"]["stderrUtf8"] != delta["new"]["stderrUtf8"],
            "a delta whose output bytes didn't actually change isn't a delta: {delta}"
        );
        assert_eq!(delta["old"]["exitCode"], delta["new"]["exitCode"]);
        assert_exact_process_result(
            delta["name"].as_str().expect("delta name"),
            delta["argv"].as_array().expect("delta argv"),
            &delta["new"],
            repo.path(),
        );
    }
}

fn commit_fixture(repo: &Path, message: &str) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["init", "--quiet"])
        .output()
        .expect("initialize fixture repository");
    assert!(output.status.success(), "git init failed");

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
