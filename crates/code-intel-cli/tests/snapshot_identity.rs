use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("code-intel-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(repo: &Path) {
    git(repo, &["init", "--quiet"]);
    git(repo, &["config", "user.name", "Snapshot Test"]);
    git(repo, &["config", "user.email", "snapshot@example.invalid"]);
    git(repo, &["config", "core.autocrlf", "false"]);
    fs::create_dir_all(repo.join("src/子 目录")).unwrap();
    fs::create_dir_all(repo.join("docs")).unwrap();
    fs::write(repo.join("src/子 目录/main file.txt"), "alpha\n").unwrap();
    fs::write(repo.join("docs/readme.md"), "docs\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", "fixture"]);
}

fn snapshot(repo: &Path, policy: &str, scopes: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    command
        .arg("snapshot")
        .arg("identity")
        .arg("--repo")
        .arg(repo)
        .arg("--working-tree-policy")
        .arg(policy);
    for scope in scopes {
        command.arg("--scope").arg(scope);
    }
    command.output().unwrap()
}

fn ok_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "snapshot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn identity_is_portable_scope_bound_and_dirty_overlay_explicit() {
    let fixture = TempTree::new("snapshot fixture 空格");
    let source = fixture.0.join("源 repo");
    let copy = fixture.0.join("copy repo");
    fs::create_dir_all(&source).unwrap();
    init_repo(&source);
    git(
        &fixture.0,
        &[
            "clone",
            "--quiet",
            "--no-hardlinks",
            "-c",
            "core.autocrlf=false",
            source.to_str().unwrap(),
            copy.to_str().unwrap(),
        ],
    );

    let left = ok_json(snapshot(&source, "explicit_overlay", &["src/子 目录"]));
    let right = ok_json(snapshot(&copy, "explicit_overlay", &["src/子 目录"]));
    assert_eq!(left["snapshot"], right["snapshot"]);
    assert_eq!(left["dirtyOverlay"]["present"], false);
    assert_eq!(
        left["snapshot"]["scope"],
        serde_json::json!(["src/子 目录"])
    );
    let redundant = ok_json(snapshot(
        &source,
        "explicit_overlay",
        &["src/子 目录", "src/子 目录/nested"],
    ));
    assert_eq!(left["snapshot"], redundant["snapshot"]);
    let root_minimal = ok_json(snapshot(&source, "explicit_overlay", &[".", "src"]));
    let root_only = ok_json(snapshot(&source, "explicit_overlay", &["."]));
    assert_eq!(root_minimal["snapshot"], root_only["snapshot"]);

    let clean_head = ok_json(snapshot(&copy, "head_only", &["src/子 目录"]));
    fs::write(copy.join("src/子 目录/main file.txt"), "bravo\n").unwrap();
    let dirty = ok_json(snapshot(&copy, "explicit_overlay", &["src/子 目录"]));
    assert_ne!(left["snapshot"]["identity"], dirty["snapshot"]["identity"]);
    assert_ne!(
        left["snapshot"]["inputDigest"],
        dirty["snapshot"]["inputDigest"]
    );
    assert_eq!(dirty["dirtyOverlay"]["present"], true);
    assert!(dirty["dirtyOverlay"]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "src/子 目录/main file.txt"));

    let head_only = ok_json(snapshot(&copy, "head_only", &["src/子 目录"]));
    assert_eq!(
        clean_head["snapshot"]["identity"],
        head_only["snapshot"]["identity"]
    );
    assert_ne!(
        left["snapshot"]["identity"],
        head_only["snapshot"]["identity"]
    );
    assert_eq!(
        clean_head["snapshot"]["inputDigest"],
        head_only["snapshot"]["inputDigest"]
    );
    assert_eq!(head_only["dirtyOverlay"]["present"], false);

    let docs = ok_json(snapshot(&copy, "explicit_overlay", &["docs"]));
    assert_ne!(dirty["snapshot"]["identity"], docs["snapshot"]["identity"]);
    let docs_before = docs["snapshot"]["identity"].clone();
    fs::write(copy.join("src/子 目录/main file.txt"), "charlie\n").unwrap();
    let docs_after = ok_json(snapshot(&copy, "explicit_overlay", &["docs"]));
    assert_eq!(docs_before, docs_after["snapshot"]["identity"]);
}

#[test]
fn missing_git_is_content_addressed_and_rejects_head_only() {
    let fixture = TempTree::new("snapshot unversioned");
    fs::create_dir_all(fixture.0.join("scope 空格")).unwrap();
    fs::write(fixture.0.join("scope 空格/file.txt"), "one\n").unwrap();

    let first = ok_json(snapshot(&fixture.0, "explicit_overlay", &["scope 空格"]));
    assert_eq!(first["snapshot"]["head"], "unversioned");
    assert!(first["snapshot"]["repoIdentity"]
        .as_str()
        .unwrap()
        .starts_with("content-v1:"));
    assert_eq!(first["dirtyOverlay"]["present"], true);

    fs::write(fixture.0.join("scope 空格/file.txt"), "two\n").unwrap();
    let second = ok_json(snapshot(&fixture.0, "explicit_overlay", &["scope 空格"]));
    assert_ne!(
        first["snapshot"]["identity"],
        second["snapshot"]["identity"]
    );

    let rejected = snapshot(&fixture.0, "head_only", &["."]);
    assert_eq!(rejected.status.code(), Some(69));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("requires Git"));
}

#[test]
fn alternate_vcs_contract_fixture_is_fail_closed_and_rolls_back_to_unversioned() {
    let fixture_contract: Value = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("orchestration/internalization/fixtures/r10-alternate-vcs-port.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(fixture_contract["mismatch"]["exitCode"], 65);
    assert_eq!(fixture_contract["mismatch"]["publishesArtifacts"], false);
    assert_eq!(
        fixture_contract["rollback"]["routes"],
        serde_json::json!(["git", "unversioned-explicit-overlay"])
    );

    let tree = TempTree::new("alternate-vcs-rollback");
    fs::write(tree.0.join("source.txt"), "rollback content\n").unwrap();
    let adapter = tree.0.join("mismatched-adapter.ps1");
    fs::write(
        &adapter,
        "$request = [Console]::In.ReadToEnd() | ConvertFrom-Json\nif ($request.schema -ne 'code-intel-alternate-vcs-snapshot-request.v1') { exit 9 }\n'{\"snapshot\":{\"identity\":\"mismatch\"}}'\n",
    )
    .unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .arg("snapshot")
        .arg("identity")
        .arg("--repo")
        .arg(&tree.0)
        .arg("--working-tree-policy")
        .arg("explicit_overlay")
        .arg("--scope")
        .arg(".")
        .arg("--alternate-vcs-command")
        .arg("pwsh")
        .arg("--alternate-vcs-arg")
        .arg("-NoProfile")
        .arg("--alternate-vcs-arg")
        .arg("-File")
        .arg("--alternate-vcs-arg")
        .arg(&adapter)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(65));
    assert!(
        rejected.stdout.is_empty(),
        "mismatch must publish no artifact"
    );
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("does not match"));

    let rolled_back = ok_json(snapshot(&tree.0, "explicit_overlay", &["."]));
    assert_eq!(rolled_back["snapshot"]["head"], "unversioned");
    assert_eq!(
        rolled_back["snapshot"]["workingTreePolicy"],
        "explicit_overlay"
    );
    assert!(rolled_back["snapshot"]["identity"]
        .as_str()
        .is_some_and(|identity| identity.len() == 64));
}

#[test]
fn nonexistent_nonroot_scope_fails_closed_while_empty_root_scope_is_legal() {
    let fixture = TempTree::new("snapshot scope existence");
    let typo = snapshot(&fixture.0, "explicit_overlay", &["typo/missing"]);
    assert_eq!(typo.status.code(), Some(64));
    assert!(typo.stdout.is_empty());
    assert!(String::from_utf8_lossy(&typo.stderr).contains("does not exist"));

    let root = ok_json(snapshot(&fixture.0, "explicit_overlay", &["."]));
    assert_eq!(root["snapshot"]["scope"], serde_json::json!(["."]));
    assert_eq!(root["snapshot"]["head"], "unversioned");
}

#[test]
fn head_identity_ignores_checkout_state_time_and_attachment_but_binds_scope_and_mode() {
    let fixture = TempTree::new("snapshot head rules");
    let repo = fixture.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let attached = ok_json(snapshot(&repo, "head_only", &["src", "docs/."]));
    let head = attached["snapshot"]["head"].as_str().unwrap().to_string();
    git(&repo, &["checkout", "--quiet", "--detach", &head]);
    let detached = ok_json(snapshot(&repo, "head_only", &["docs", "src/."]));
    assert_eq!(attached["snapshot"], detached["snapshot"]);

    fs::write(repo.join("src/子 目录/main file.txt"), b"binary\0\r\n").unwrap();
    let after_bytes = ok_json(snapshot(&repo, "head_only", &["src", "docs"]));
    assert_eq!(attached["snapshot"], after_bytes["snapshot"]);
    let metadata = fs::metadata(repo.join("docs/readme.md")).unwrap();
    let file = fs::OpenOptions::new()
        .write(true)
        .open(repo.join("docs/readme.md"))
        .unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    let after_time = ok_json(snapshot(&repo, "head_only", &["src", "docs"]));
    assert_eq!(attached["snapshot"], after_time["snapshot"]);

    let sub_scope = ok_json(snapshot(&repo, "head_only", &["src"]));
    assert_ne!(
        attached["snapshot"]["identity"],
        sub_scope["snapshot"]["identity"]
    );
    let escaped = snapshot(&repo, "head_only", &["../src"]);
    assert_eq!(escaped.status.code(), Some(64));

    git(&repo, &["update-index", "--chmod=+x", "docs/readme.md"]);
    git(&repo, &["commit", "--quiet", "-m", "mode"]);
    let mode_changed = ok_json(snapshot(&repo, "head_only", &["src", "docs"]));
    assert_ne!(
        attached["snapshot"]["identity"],
        mode_changed["snapshot"]["identity"]
    );
}

#[test]
fn overlay_classifies_delete_untracked_and_ignored_without_following_ignored_input() {
    let fixture = TempTree::new("snapshot overlay rules");
    let repo = fixture.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    fs::write(repo.join(".gitignore"), "ignored.bin\nscratch/\n").unwrap();
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "--quiet", "-m", "ignore"]);
    let clean = ok_json(snapshot(&repo, "explicit_overlay", &["."]));

    fs::remove_file(repo.join("docs/readme.md")).unwrap();
    fs::write(repo.join("new-data.txt"), [0, 1, 2, 255]).unwrap();
    fs::write(repo.join("ignored.bin"), "ignored-one").unwrap();
    let dirty = ok_json(snapshot(&repo, "explicit_overlay", &["."]));
    assert_ne!(clean["snapshot"]["identity"], dirty["snapshot"]["identity"]);
    assert!(dirty["dirtyOverlay"]["members"]["trackedDeleted"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "docs/readme.md"));
    assert!(
        dirty["dirtyOverlay"]["members"]["untracked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "new-data.txt"),
        "{}",
        dirty
    );
    let before_ignored = dirty["snapshot"]["identity"].clone();
    fs::write(repo.join("ignored.bin"), "ignored-two").unwrap();
    let ignored_changed = ok_json(snapshot(&repo, "explicit_overlay", &["."]));
    assert_eq!(before_ignored, ignored_changed["snapshot"]["identity"]);
    fs::create_dir_all(repo.join("scratch/nested")).unwrap();
    fs::write(repo.join("scratch/nested/.gitignore"), "*.bin\n").unwrap();
    fs::write(
        repo.join("scratch/nested/generated.bin"),
        vec![7_u8; 1024 * 1024],
    )
    .unwrap();
    let ignored_tree_changed = ok_json(snapshot(&repo, "explicit_overlay", &["."]));
    assert_eq!(
        ignored_changed["snapshot"]["identity"], ignored_tree_changed["snapshot"]["identity"],
        "an ignored scratch subtree, including nested controls, must not become snapshot input"
    );
    assert_eq!(
        ignored_tree_changed["dirtyOverlay"]["ignoredPolicy"],
        "excluded_by_git_ignore"
    );
}

#[test]
fn intent_to_add_is_an_explicit_overlay_member() {
    let fixture = TempTree::new("snapshot intent to add");
    let repo = fixture.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    fs::write(repo.join("intent.txt"), "intent").unwrap();
    git(&repo, &["add", "-N", "intent.txt"]);
    let snapshot = ok_json(snapshot(&repo, "explicit_overlay", &["."]));
    assert_eq!(snapshot["dirtyOverlay"]["present"], true);
    assert!(snapshot["dirtyOverlay"]["members"]["trackedModified"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "intent.txt"));
}

#[cfg(windows)]
#[test]
fn windows_scope_case_collision_fails_closed() {
    let fixture = TempTree::new("snapshot scope collision");
    let repo = fixture.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let output = snapshot(&repo, "head_only", &["src", "SRC"]);
    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("case collision"));
    let overlapping = snapshot(&repo, "head_only", &["src", "SRC/nested"]);
    assert_eq!(overlapping.status.code(), Some(64));
}

#[test]
fn unborn_git_is_explicit_content_snapshot_and_head_only_is_unavailable() {
    let fixture = TempTree::new("snapshot unborn");
    git(&fixture.0, &["init", "--quiet"]);
    fs::write(fixture.0.join("file.txt"), "unborn").unwrap();
    let explicit = ok_json(snapshot(&fixture.0, "explicit_overlay", &["."]));
    assert_eq!(explicit["snapshot"]["head"], "unborn");
    assert_eq!(explicit["repository"]["kind"], "git_unborn");
    assert!(explicit["snapshot"]["repoIdentity"]
        .as_str()
        .unwrap()
        .starts_with("content-v1:"));
    let rejected = snapshot(&fixture.0, "head_only", &["."]);
    assert_eq!(rejected.status.code(), Some(69));
}

#[test]
fn head_snapshot_binds_gitlink_lfs_pointer_and_case_sensitive_scope() {
    let fixture = TempTree::new("snapshot special entries");
    let repo = fixture.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let commit = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    git(
        &repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},vendor/sub", commit.trim()),
        ],
    );
    fs::write(
        repo.join("asset.lfs"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize 1\n",
    )
    .unwrap();
    git(&repo, &["add", "asset.lfs"]);
    git(&repo, &["commit", "--quiet", "-m", "special entries"]);
    let first = ok_json(snapshot(&repo, "head_only", &["."]));

    fs::write(
        repo.join("asset.lfs"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\nsize 1\n",
    )
    .unwrap();
    git(&repo, &["add", "asset.lfs"]);
    git(&repo, &["commit", "--quiet", "-m", "new pointer"]);
    let second = ok_json(snapshot(&repo, "head_only", &["."]));
    assert_ne!(
        first["snapshot"]["identity"],
        second["snapshot"]["identity"]
    );

    let lower = ok_json(snapshot(&repo, "head_only", &["src"]));
    let upper = ok_json(snapshot(&repo, "head_only", &["SRC"]));
    assert_ne!(lower["snapshot"]["identity"], upper["snapshot"]["identity"]);
}

#[test]
fn shallow_lineage_is_rejected_and_symlink_target_is_hashed_without_following() {
    let fixture = TempTree::new("snapshot shallow symlink");
    let repo = fixture.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    fs::write(repo.join("docs/readme.md"), "second").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "second"]);
    let shallow = fixture.0.join("shallow");
    let source_url = format!("file:///{}", repo.to_string_lossy().replace('\\', "/"));
    git(
        &fixture.0,
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &source_url,
            shallow.to_str().unwrap(),
        ],
    );
    let rejected = snapshot(&shallow, "head_only", &["."]);
    assert_eq!(rejected.status.code(), Some(69));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("shallow"));

    let outside = fixture.0.join("outside.txt");
    fs::write(&outside, "outside-one").unwrap();
    let link = repo.join("outside-link");
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, &link).is_ok();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
    #[cfg(not(any(windows, unix)))]
    let linked = false;
    if linked {
        let before = ok_json(snapshot(&repo, "explicit_overlay", &["."]));
        fs::write(&outside, "outside-two").unwrap();
        let after_external = ok_json(snapshot(&repo, "explicit_overlay", &["."]));
        assert_eq!(
            before["snapshot"]["identity"], after_external["snapshot"]["identity"],
            "symlink target bytes outside the repository must not be followed"
        );
    }
}

#[test]
fn missing_git_executable_is_unavailable_not_unversioned() {
    let fixture = TempTree::new("snapshot missing git");
    fs::write(fixture.0.join("file.txt"), "content").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "snapshot",
            "identity",
            "--repo",
            fixture.0.to_str().unwrap(),
            "--working-tree-policy",
            "explicit_overlay",
            "--scope",
            ".",
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(69));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot launch Git"));
}
