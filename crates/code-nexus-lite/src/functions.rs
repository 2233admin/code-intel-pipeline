//! Three Agent-callable functions.
//!
//! Each function returns `Result<Value, iii_sdk::IIIError>`. iii deserialises the
//! incoming JSON into the closure's input type automatically (via the
//! `JsonSchema` + `Deserialize` derives).

use iii_sdk::IIIError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{info, warn};

pub mod repowise;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanInput {
    /// Absolute path to the repository to scan.
    pub repo: String,
    /// Optional: skip running `repowise init` if a `.repowise/` cache already exists.
    #[serde(default)]
    pub skip_init_if_cached: bool,
}

#[derive(Debug, Serialize)]
pub struct ScanOutput {
    pub repo: String,
    pub files: usize,
    pub languages: Vec<String>,
    pub commits: u32,
    pub cache_path: String,
    pub elapsed_ms: u128,
}

/// `codenexus::scan` — call Repowise `augment` on a repo, write a graph snapshot.
pub async fn scan(input: ScanInput) -> Result<Value, IIIError> {
    let t0 = std::time::Instant::now();
    info!(
        "codenexus::scan repo={} skip_init_if_cached={}",
        input.repo, input.skip_init_if_cached
    );

    // Run `repowise init` (or `augment` if cache exists + skip_init_if_cached)
    let cache_path = format!("{}/.repowise", input.repo.trim_end_matches('/'));
    let cache_exists = std::path::Path::new(&cache_path).exists();

    let cmd_output = if cache_exists && input.skip_init_if_cached {
        repowise::run_augment(&input.repo).await
    } else {
        repowise::run_init(&input.repo).await
    }
    .map_err(|e| IIIError::Handler(format!("repowise failed: {e}")))?;

    let elapsed_ms = t0.elapsed().as_millis();
    let parsed_out: ScanOutput = ScanOutput {
        repo: input.repo.clone(),
        files: cmd_output.files,
        languages: cmd_output.languages,
        commits: cmd_output.commits,
        cache_path,
        elapsed_ms,
    };

    Ok(json!({
        "ok": true,
        "scan": parsed_out,
        "raw_stdout_tail": cmd_output.stdout_tail,
    }))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LiteInput {
    /// Absolute path to the repository whose `.repowise/wiki.db` we should read.
    pub repo: String,
    /// Optional: cap how many files / functions to include. Default 8 / 12 (matches PS1).
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    #[serde(default = "default_max_refs")]
    pub max_references_per_file: usize,
}

fn default_max_files() -> usize {
    8
}
fn default_max_refs() -> usize {
    12
}

/// `codenexus::lite` — read a stored graph snapshot, return a compact Agent context.
pub async fn lite(input: LiteInput) -> Result<Value, IIIError> {
    info!(
        "codenexus::lite repo={} max_files={} max_refs={}",
        input.repo, input.max_files, input.max_references_per_file
    );

    let db_path = std::path::Path::new(&input.repo).join(".repowise/wiki.db");
    if !db_path.exists() {
        warn!(
            "wiki.db not found at {}; suggest calling codenexus::scan first",
            db_path.display()
        );
        return Ok(json!({
            "ok": false,
            "hint": "run codenexus::scan first",
            "expected_cache": db_path.display().to_string(),
        }));
    }

    // Read the SQLite summary. We keep this minimal (no full schema query) — just
    // enough to give an Agent a useful starting point.
    let summary = repowise::read_wiki_summary(
        &db_path,
        input.max_files,
        input.max_references_per_file,
    )
    .await
    .map_err(|e| IIIError::Handler(format!("wiki read failed: {e}")))?;

    Ok(json!({
        "ok": true,
        "lite": summary,
    }))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DoctorInput {
    /// Reserved for future options; currently unused.
    #[serde(default)]
    pub _placeholder: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DoctorOutput {
    pub repowise_version: String,
    pub repowise_path: String,
    pub sentrux_present: bool,
    pub sentrux_path: Option<String>,
    pub rg_present: bool,
    pub rg_path: Option<String>,
    pub ok: bool,
}

/// `codenexus::doctor` — run Repowise/Sentrux doctor checks, return JSON.
pub async fn doctor(_input: DoctorInput) -> Result<Value, IIIError> {
    info!("codenexus::doctor");

    let version = repowise::version()
        .map_err(|e| IIIError::Handler(format!("repowise --version failed: {e}")))?;
    let path = repowise::binary_path();

    let sentrux = which("sentrux");
    let rg = which("rg");

    let out = DoctorOutput {
        repowise_version: version,
        repowise_path: path,
        sentrux_present: sentrux.is_some(),
        sentrux_path: sentrux.clone(),
        rg_present: rg.is_some(),
        rg_path: rg.clone(),
        ok: true,
    };

    Ok(json!({ "ok": true, "doctor": out }))
}

/// Lightweight `which`-equivalent: checks if a binary is on PATH by trying to run `--version`.
/// We don't want a `which` crate dependency for one helper.
fn which(name: &str) -> Option<String> {
    let out = std::process::Command::new(name).arg("--version").output();
    match out {
        Ok(o) if o.status.success() => Some(name.to_string()),
        _ => None,
    }
}
