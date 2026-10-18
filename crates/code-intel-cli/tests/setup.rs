//! `code-intel setup` against a fake extracted release and a throwaway home
//! (#401). Every run sets HOME (and USERPROFILE/LOCALAPPDATA) to a temp dir,
//! so nothing here touches the real user environment. Unix profile editing is
//! exercised directly; Windows runs use `--no-modify-path` so CI never writes
//! the runner's registry.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "code-intel-setup-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let binary = if cfg!(windows) {
            "code-intel.exe"
        } else {
            "code-intel"
        };
        fs::create_dir_all(root.join("release/bin")).unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_code-intel"),
            root.join("release/bin").join(binary),
        )
        .unwrap();
        let skill = root.join("release/skills/code-intel-pipeline");
        fs::create_dir_all(skill.join("scripts")).unwrap();
        fs::write(skill.join("SKILL.md"), "# code-intel-pipeline\n").unwrap();
        fs::write(skill.join("scripts/bootstrap.py"), "print('bootstrap')\n").unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        fs::create_dir_all(root.join("empty-path")).unwrap();
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn setup(&self, extra: &[&str]) -> (Output, Value) {
        let mut command = common::cli();
        command
            .arg("setup")
            .arg("--release-root")
            .arg(self.root.join("release"))
            .arg("--json")
            .args(extra)
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env("LOCALAPPDATA", self.home().join("AppData/Local"))
            .env("PATH", self.root.join("empty-path"))
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("ZDOTDIR");
        let output = command.output().unwrap();
        let value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        (output, value)
    }

    fn statuses(value: &Value) -> Vec<(String, String)> {
        value["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|action| {
                (
                    action["name"].as_str().unwrap().to_string(),
                    action["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn status<'a>(statuses: &'a [(String, String)], name: &str) -> &'a str {
    statuses
        .iter()
        .find(|(action, _)| action == name)
        .map(|(_, status)| status.as_str())
        .unwrap_or_else(|| panic!("no {name} action in {statuses:?}"))
}

fn platform_flags() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["--skip-doctor", "--no-modify-path"]
    } else {
        vec!["--skip-doctor"]
    }
}

#[test]
fn installs_binary_and_skill_then_a_second_run_changes_nothing() {
    let fixture = Fixture::new("idempotent");
    let (output, first) = fixture.setup(&platform_flags());
    assert_eq!(output.status.code(), Some(0), "{first:#}");
    assert_eq!(first["schema"], "code-intel-setup.v1");
    assert_eq!(first["ok"], true);
    let statuses = Fixture::statuses(&first);
    for name in ["binary", "skill:source", "skill:claude", "skill:codex"] {
        assert_eq!(status(&statuses, name), "installed", "{first:#}");
    }

    let bin = PathBuf::from(first["paths"]["bin"].as_str().unwrap());
    let installed = bin.join(if cfg!(windows) {
        "code-intel.exe"
    } else {
        "code-intel"
    });
    assert_eq!(
        fs::read(&installed).unwrap(),
        fs::read(env!("CARGO_BIN_EXE_code-intel")).unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&installed).unwrap().permissions().mode() & 0o111,
            0o111
        );
    }
    let canonical = fixture.home().join(".agents/skills/code-intel-pipeline");
    assert!(canonical.join("scripts/bootstrap.py").is_file());
    for agent in [".claude", ".codex"] {
        let served = fixture
            .home()
            .join(agent)
            .join("skills/code-intel-pipeline/SKILL.md");
        assert_eq!(
            fs::read_to_string(served).unwrap(),
            "# code-intel-pipeline\n"
        );
    }
    assert_eq!(
        fs::canonicalize(first["paths"]["codeIntelHome"].as_str().unwrap()).unwrap(),
        fs::canonicalize(fixture.root.join("release")).unwrap()
    );

    let (output, second) = fixture.setup(&platform_flags());
    assert_eq!(output.status.code(), Some(0), "{second:#}");
    for (name, status) in Fixture::statuses(&second) {
        assert!(
            status == "already_present" || status == "skipped",
            "{name} was {status} on the second run: {second:#}"
        );
    }
}

#[cfg(unix)]
#[test]
fn shell_profiles_source_env_once_and_existing_content_survives() {
    let fixture = Fixture::new("profiles");
    fs::write(fixture.home().join(".bashrc"), "alias ll='ls -l'").unwrap();
    let mut bin = String::new();
    for _ in 0..2 {
        let (output, value) = fixture.setup(&["--skip-doctor"]);
        assert_eq!(output.status.code(), Some(0), "{value:#}");
        bin = value["paths"]["bin"].as_str().unwrap().to_string();
    }
    // The data root differs per OS (macOS: Library/Application Support).
    let env_sh = fixture.home().join(".config/code-intel/env.sh");
    let body = fs::read_to_string(&env_sh).unwrap();
    assert!(body.contains("export CODE_INTEL_HOME="), "{body}");
    assert!(body.contains(&bin), "{body}");
    let line = format!(". \"{}\"", env_sh.display());
    let profile = fs::read_to_string(fixture.home().join(".profile")).unwrap();
    assert_eq!(profile.matches(&line).count(), 1, "{profile}");
    let bashrc = fs::read_to_string(fixture.home().join(".bashrc")).unwrap();
    assert!(bashrc.starts_with("alias ll='ls -l'\n"), "{bashrc}");
    assert_eq!(bashrc.matches(&line).count(), 1, "{bashrc}");
    // Only files that exist (or a shell that is installed) are touched.
    assert!(!fixture.home().join(".bash_profile").exists());
    assert!(!fixture.home().join(".zshenv").exists());

    // The generated file really prefixes PATH exactly once when sourced twice.
    let shell = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            ". '{0}'; . '{0}'; printf %s \"$PATH\"",
            env_sh.display()
        ))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let path = String::from_utf8(shell.stdout).unwrap();
    assert_eq!(path.matches(bin.as_str()).count(), 1, "{path}");
}

#[cfg(unix)]
#[test]
fn no_modify_path_writes_env_file_but_leaves_profiles_alone() {
    let fixture = Fixture::new("no-modify");
    let (output, value) = fixture.setup(&["--skip-doctor", "--no-modify-path"]);
    assert_eq!(output.status.code(), Some(0), "{value:#}");
    assert!(fixture.home().join(".config/code-intel/env.sh").is_file());
    assert!(!fixture.home().join(".profile").exists());
    assert_eq!(status(&Fixture::statuses(&value), "env:profile"), "skipped");
    assert!(value["pathRefresh"].as_str().unwrap().starts_with(". \""));
}

#[test]
fn an_occupied_skill_directory_is_moved_aside_not_deleted() {
    let fixture = Fixture::new("occupied");
    let claude = fixture.home().join(".claude/skills/code-intel-pipeline");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("NOTES.md"), "my edits").unwrap();
    let (output, value) = fixture.setup(&platform_flags());
    assert_eq!(output.status.code(), Some(0), "{value:#}");
    let aside = fs::read_dir(fixture.home().join(".claude/skills"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("code-intel-pipeline.replaced-")
        })
        .expect("previous content kept beside the link");
    assert_eq!(
        fs::read_to_string(aside.join("NOTES.md")).unwrap(),
        "my edits"
    );
    assert!(claude.join("SKILL.md").is_file());
}

#[test]
fn missing_tools_are_reported_with_install_commands_without_failing_setup() {
    let fixture = Fixture::new("doctor");
    let mut flags = platform_flags();
    flags.retain(|flag| *flag != "--skip-doctor");
    let (output, value) = fixture.setup(&flags);
    assert_eq!(output.status.code(), Some(0), "{value:#}");
    assert_eq!(value["ok"], true);
    assert_eq!(value["doctor"]["ok"], false, "{value:#}");
    let hints = value["doctor"]["installHints"].as_array().unwrap();
    assert!(
        hints
            .iter()
            .any(|hint| hint["tool"] == "rg" && hint["install"].is_string()),
        "{value:#}"
    );
}

#[test]
fn language_is_written_to_the_repository_config() {
    let fixture = Fixture::new("language");
    let repo = fixture.root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let mut flags = platform_flags();
    flags.extend(["--no-skills", "--language", "zh", "--repo-path"]);
    flags.push(repo.to_str().unwrap());
    let (output, value) = fixture.setup(&flags);
    assert_eq!(output.status.code(), Some(0), "{value:#}");
    let config = fs::read_to_string(repo.join(".code-intel/config.json")).unwrap();
    assert!(config.contains("\"zh\""), "{config}");
}

#[test]
fn usage_errors_exit_65_without_touching_the_home() {
    let fixture = Fixture::new("usage");
    for args in [
        vec!["--bogus"],
        vec!["--language", "zh"],
        vec!["--language", "fr", "--repo-path", "."],
        vec!["--release-root"],
    ] {
        let (output, _) = fixture.setup(&args);
        assert_eq!(output.status.code(), Some(65), "{args:?}");
    }
    let leftovers: Vec<_> = fs::read_dir(fixture.home()).unwrap().collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_canonical_skill_is_replaced_by_a_real_copy() {
    let fixture = Fixture::new("canonical-link");
    let skills = fixture.home().join(".agents/skills");
    fs::create_dir_all(&skills).unwrap();
    let canonical = skills.join("code-intel-pipeline");
    std::os::unix::fs::symlink(
        fixture.root.join("release/skills/code-intel-pipeline"),
        &canonical,
    )
    .unwrap();
    let (output, value) = fixture.setup(&["--skip-doctor", "--no-modify-path"]);
    assert_eq!(output.status.code(), Some(0), "{value:#}");
    assert_eq!(
        status(&Fixture::statuses(&value), "skill:source"),
        "installed"
    );
    let metadata = fs::symlink_metadata(&canonical).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
}
