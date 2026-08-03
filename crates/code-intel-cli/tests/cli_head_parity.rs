use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SOURCE_REVISION: &str = "a56ad2c39a617ebb72447a98c0087a765758c296";

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
}

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
    let mut command = Command::new(binary());
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
    assert_eq!(
        output.stdout,
        expected["stdoutUtf8"]
            .as_str()
            .expect("stdoutUtf8")
            .as_bytes(),
        "{label} stdout bytes"
    );
    assert_eq!(
        output.stderr,
        expected["stderrUtf8"]
            .as_str()
            .expect("stderrUtf8")
            .as_bytes(),
        "{label} stderr bytes"
    );
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
    assert_eq!(cases.len(), 51);

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

#[test]
fn full_help_v2_is_the_only_intentional_delta() {
    let fixture = head_parity_fixture();
    let deltas = fixture["intentionalDeltas"]
        .as_array()
        .expect("intentional deltas");
    assert_eq!(deltas.len(), 1);
    let delta = &deltas[0];
    assert_eq!(delta["oldContractId"], "text-format:help-full.v1");
    assert_eq!(delta["newContractId"], "text-format:help-full.v2");
    assert_ne!(delta["old"]["stdoutUtf8"], delta["new"]["stdoutUtf8"]);
    assert_eq!(delta["old"]["exitCode"], delta["new"]["exitCode"]);
    assert_eq!(delta["old"]["stderrUtf8"], delta["new"]["stderrUtf8"]);

    let repo = FixtureRepository::create();
    assert_exact_process_result(
        delta["name"].as_str().expect("delta name"),
        delta["argv"].as_array().expect("delta argv"),
        &delta["new"],
        repo.path(),
    );
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
