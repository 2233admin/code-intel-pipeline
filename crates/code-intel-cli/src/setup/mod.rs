//! `code-intel setup`: finish an installation from an extracted release.
//!
//! The Skill's `bootstrap.py` downloads, verifies and extracts a release, then
//! runs the release's own binary with `setup` (#401, DR-0018). Setup installs
//! the binary into the bin directory, persists `PATH` and `CODE_INTEL_HOME`,
//! installs the Skill for agents, optionally records the repository language,
//! and ends with the doctor probe. Missing external tools are reported with an
//! install command, never installed: the install itself still succeeds.

mod env;
mod skills;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::doctor_bootstrap::{self, paths};

const SCHEMA: &str = "code-intel-setup.v1";

#[derive(Debug)]
pub(crate) struct Action {
    name: String,
    status: &'static str,
    detail: String,
    fix: Option<String>,
}

impl Action {
    fn new(name: &str, status: &'static str, detail: &str, fix: Option<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
            fix,
        }
    }
    fn installed(name: &str, detail: &str) -> Self {
        Self::new(name, "installed", detail, None)
    }
    fn already_present(name: &str, detail: &str) -> Self {
        Self::new(name, "already_present", detail, None)
    }
    fn skipped(name: &str, detail: &str, fix: Option<String>) -> Self {
        Self::new(name, "skipped", detail, fix)
    }
    fn failed(name: &str, detail: &str, fix: Option<String>) -> Self {
        Self::new(name, "failed", detail, fix)
    }
    fn is_failed(&self) -> bool {
        self.status == "failed"
    }
    fn to_json(&self) -> Value {
        json!({"name": self.name, "status": self.status, "detail": self.detail, "fix": self.fix})
    }
}

struct Options {
    release_root: Option<PathBuf>,
    repo_path: Option<PathBuf>,
    language: Option<String>,
    platform: String,
    modify_path: bool,
    skills: bool,
    doctor: bool,
    json: bool,
}

fn parse(raw: &[String]) -> Result<Options, String> {
    let mut options = Options {
        release_root: None,
        repo_path: None,
        language: None,
        platform: "auto".into(),
        modify_path: true,
        skills: true,
        doctor: true,
        json: false,
    };
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        let value = || {
            raw.get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        index += match flag {
            "--release-root" => {
                options.release_root = Some(PathBuf::from(value()?));
                2
            }
            "--repo-path" => {
                options.repo_path = Some(PathBuf::from(value()?));
                2
            }
            "--language" => {
                let language = value()?;
                if !matches!(language.as_str(), "zh" | "en") {
                    return Err(format!("--language must be zh|en, got {language}"));
                }
                options.language = Some(language);
                2
            }
            "--platform" => {
                options.platform = value()?;
                2
            }
            "--no-modify-path" => {
                options.modify_path = false;
                1
            }
            "--no-skills" => {
                options.skills = false;
                1
            }
            "--skip-doctor" => {
                options.doctor = false;
                1
            }
            "--json" => {
                options.json = true;
                1
            }
            other => return Err(format!("unknown setup argument: {other}")),
        };
    }
    if options.language.is_some() && options.repo_path.is_none() {
        return Err("--language needs --repo-path: the preference is stored per repository".into());
    }
    Ok(options)
}

/// The release this binary came from: `<release>/bin/code-intel[.exe]`.
fn default_release_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    let release = bin.parent()?;
    (bin.file_name()? == "bin" && release.join("skills").is_dir()).then(|| release.to_path_buf())
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let options = match parse(raw) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return 65;
        }
    };
    let platform = match paths::resolve_platform(&options.platform) {
        Ok(platform) => platform,
        Err(message) => {
            eprintln!("{message}");
            return 65;
        }
    };
    let Some(release_root) = options
        .release_root
        .clone()
        .or_else(default_release_root)
        .map(|root| paths::resolve_code_intel_path(&root))
    else {
        eprintln!(
            "cannot locate the release: run the binary from <release>/bin or pass --release-root"
        );
        return 65;
    };
    let result = setup(&options, &platform, &release_root);
    let ok = result["ok"].as_bool() == Some(true);
    if options.json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        print!("{}", render_human(&result));
    }
    if ok {
        0
    } else {
        1
    }
}

fn setup(options: &Options, platform: &str, release_root: &Path) -> Value {
    let home = paths::home_directory();
    let data_root = paths::data_root(platform, &home);
    let bin = paths::bin_dir(&data_root);
    let artifact_root = crate::artifacts::resolve_artifact_root(None)
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    let mut actions = vec![install_binary(platform, release_root, &bin)];
    let persisted = env::persist(&home, &bin, release_root, options.modify_path);
    actions.extend(persisted.actions);
    if options.skills {
        actions.extend(skills::install(&home, release_root));
    }
    if let (Some(language), Some(repo)) = (&options.language, &options.repo_path) {
        actions.push(
            match crate::language_pref::write_project_config(repo, language) {
                Ok(path) => Action::installed("language", &path.display().to_string()),
                Err(error) => Action::failed(
                    "language",
                    &format!("write language preference: {error}"),
                    None,
                ),
            },
        );
    }

    // This process now behaves like a fresh shell would after setup.
    std::env::set_var("CODE_INTEL_HOME", release_root);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![bin.clone()];
    entries.extend(std::env::split_paths(&path));
    if let Ok(joined) = std::env::join_paths(entries) {
        std::env::set_var("PATH", joined);
    }
    let doctor = if options.doctor {
        run_doctor(options, platform, release_root)
    } else {
        json!({"skipped": true})
    };

    let ok = !actions.iter().any(Action::is_failed);
    json!({
        "schema": SCHEMA,
        "ok": ok,
        "platform": platform,
        "paths": {
            "home": home.display().to_string(),
            "dataRoot": data_root.display().to_string(),
            "bin": bin.display().to_string(),
            "codeIntelHome": release_root.display().to_string(),
            "artifactRoot": artifact_root,
        },
        "actions": actions.iter().map(Action::to_json).collect::<Vec<_>>(),
        "pathRefresh": persisted.refresh,
        "doctor": doctor,
    })
}

fn install_binary(platform: &str, release_root: &Path, bin: &Path) -> Action {
    let name = paths::binary_name(platform);
    let source = release_root.join("bin").join(&name);
    let destination = bin.join(&name);
    let Ok(bytes) = fs::read(&source) else {
        return Action::failed(
            "binary",
            &format!("release binary missing: {}", source.display()),
            Some("pass --release-root pointing at an extracted release".into()),
        );
    };
    if fs::read(&destination).is_ok_and(|current| current == bytes) {
        // Identical bytes can still have lost the execute bit.
        return match make_executable(&destination) {
            Ok(()) => Action::already_present("binary", &destination.display().to_string()),
            Err(error) => Action::failed(
                "binary",
                &format!("make {} executable: {error}", destination.display()),
                None,
            ),
        };
    }
    let copied = fs::create_dir_all(bin).and_then(|()| {
        // Copy beside the target, then rename, so a running copy of the old
        // binary is never truncated mid-write.
        let staging = bin.join(format!(".{name}.staging-{}", std::process::id()));
        fs::write(&staging, &bytes)?;
        make_executable(&staging)?;
        fs::rename(&staging, &destination)
    });
    match copied {
        Ok(()) => Action::installed("binary", &destination.display().to_string()),
        Err(error) => Action::failed(
            "binary",
            &format!("install {}: {error}", destination.display()),
            None,
        ),
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn run_doctor(options: &Options, platform: &str, release_root: &Path) -> Value {
    let mut doctor_options = doctor_bootstrap::Options::new(release_root.to_path_buf());
    doctor_options.platform = platform.into();
    doctor_options.repo_path = options
        .repo_path
        .as_ref()
        .map(|path| path.display().to_string());
    match doctor_bootstrap::observe(&doctor_options) {
        Ok(observation) => {
            let missing = observation["missing"].clone();
            let hints = missing
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|tool| {
                    install_hint(platform, tool).map(|hint| json!({"tool": tool, "install": hint}))
                })
                .collect::<Vec<_>>();
            json!({
                "ok": observation["ok"],
                "missing": missing,
                "installHints": hints,
                "observation": observation,
            })
        }
        Err(error) => json!({"ok": false, "error": error}),
    }
}

/// The install command for a tool doctor reports missing. Setup never runs
/// these itself (#401): installing system packages is the user's decision.
fn install_hint(platform: &str, tool: &str) -> Option<&'static str> {
    Some(match (platform, tool) {
        ("windows", "rg") => "winget install BurntSushi.ripgrep.MSVC",
        ("macos", "rg") => "brew install ripgrep",
        (_, "rg") => "sudo apt-get install ripgrep  # or your distribution's ripgrep package",
        ("windows", "git") => "winget install Git.Git",
        ("macos", "git") => "xcode-select --install",
        (_, "git") => "sudo apt-get install git",
        ("windows", "python") => "winget install Python.Python.3.13",
        ("macos", "python") => "brew install python",
        (_, "python") => "sudo apt-get install python3",
        (_, "repowise") => "python -m pip install --user \"repowise>=0.55.0\"",
        _ => return None,
    })
}

fn render_human(result: &Value) -> String {
    let mut out = String::new();
    for action in result["actions"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "[{}] {}: {}\n",
            action["status"].as_str().unwrap_or_default(),
            action["name"].as_str().unwrap_or_default(),
            action["detail"].as_str().unwrap_or_default()
        ));
        if let Some(fix) = action["fix"].as_str() {
            out.push_str(&format!("    fix: {fix}\n"));
        }
    }
    for hint in result["doctor"]["installHints"]
        .as_array()
        .into_iter()
        .flatten()
    {
        out.push_str(&format!(
            "[missing] {}: {}\n",
            hint["tool"].as_str().unwrap_or_default(),
            hint["install"].as_str().unwrap_or_default()
        ));
    }
    if let Some(refresh) = result["pathRefresh"].as_str() {
        out.push_str(&format!("To use code-intel in this shell now: {refresh}\n"));
    }
    out.push_str(&format!(
        "Setup {}\n",
        if result["ok"].as_bool() == Some(true) {
            "OK"
        } else {
            "FAILED"
        }
    ));
    out
}
