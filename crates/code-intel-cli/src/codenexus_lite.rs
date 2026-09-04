//! Pure-Rust implementation of the CodeNexus-lite context generator.
//!
//! This is the Rust replacement for `legacy/Invoke-CodeNexusLite.ps1`
//! (issue #275, PS1 retirement campaign). The PowerShell facade was the
//! repository-owned adapter that produced `codenexus-context.json` from
//! sentrux hotspots/DSM signals; `builtin_provider_evidence::codenexus_admission`
//! shelled out to it. The contract — the `codenexus-context.json` document
//! shape, the selection semantics, and the exclusion rules — is preserved
//! verbatim so existing consumers (evidence payloads, artifact refs) are
//! byte-compatible.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::capability::sha256_hex;
pub(crate) fn normalized_canonical_path(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(canonical)
}
#[path = "hardened_git.rs"]
mod hardened_git;

/// File extensions the fallback "largest code file" selector considers.
const CODE_EXTENSIONS: [&str; 10] = [
    ".ps1", ".py", ".rs", ".go", ".ts", ".tsx", ".js", ".jsx", ".java", ".cs",
];

/// Paths that must never enter the CodeNexus context (build output, VCS,
/// tool staging). Mirrors `Test-CodeNexusGeneratedPath` in the facade.
fn is_generated_path(relative: &str) -> bool {
    let mut normalized = relative.replace('\\', "/").to_ascii_lowercase();
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    let segments: Vec<&str> = normalized.split('/').collect();
    segments.iter().any(|segment| {
        matches!(
            *segment,
            "work"
                | "artifact"
                | "artifacts"
                | "staging"
                | ".code-intel"
                | ".git"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".venv"
                | "__pycache__"
        )
    })
}

/// Select up to `max_files` files: sentrux hotspots first, then DSM modules
/// ranked by risk, then largest code files as fallback. Mirrors
/// `Select-HotspotFiles` in the facade (same ordering, same dedup).
fn select_hotspot_files(
    repo: &Path,
    target: &Path,
    hotspots: Option<&Value>,
    dsm: Option<&Value>,
    max_files: usize,
) -> Vec<Value> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut items: Vec<Value> = Vec::new();

    if let Some(hotspots) = hotspots {
        if let Some(files) = hotspots.get("files").and_then(Value::as_array) {
            for file in files {
                if items.len() >= max_files {
                    break;
                }
                let path = file.get("path").and_then(Value::as_str).unwrap_or("");
                if path.is_empty() || is_generated_path(path) || seen.contains(path) {
                    continue;
                }
                seen.insert(path.to_string());
                items.push(json!({
                    "path": path,
                    "reason": "sentrux_hotspot",
                    "maxComplexity": file.get("maxComplexity").and_then(Value::as_i64),
                    "functionCount": file.get("functionCount").and_then(Value::as_i64),
                    "riskScore": Value::Null,
                }));
            }
        }
    }

    if items.len() < max_files {
        if let Some(dsm) = dsm {
            if let Some(modules) = dsm.get("modules").and_then(Value::as_array) {
                let mut ranked: Vec<&Value> = modules.iter().collect();
                ranked.sort_by(|a, b| {
                    let ar = a
                        .pointer("/metrics/risk")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let br = b
                        .pointer("/metrics/risk")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    br.cmp(&ar)
                });
                for module in ranked {
                    if items.len() >= max_files {
                        break;
                    }
                    if let Some(files) = module.get("files").and_then(Value::as_array) {
                        for path in files {
                            if items.len() >= max_files {
                                break;
                            }
                            let path_text = path.as_str().unwrap_or("");
                            if path_text.is_empty()
                                || is_generated_path(path_text)
                                || seen.contains(path_text)
                            {
                                continue;
                            }
                            seen.insert(path_text.to_string());
                            items.push(json!({
                                "path": path_text,
                                "reason": "sentrux_module_risk",
                                "maxComplexity": Value::Null,
                                "functionCount": Value::Null,
                                "riskScore": module.pointer("/metrics/risk"),
                            }));
                        }
                    }
                }
            }
        }
    }

    if items.len() < max_files {
        let root = if target.is_dir() { target } else { repo };
        let mut candidates: Vec<(String, u64)> = Vec::new();
        if let Ok(entries) = walk_code_files(root) {
            for (path, size) in entries {
                let relative = relative_path(repo, &path);
                if relative.is_empty() || is_generated_path(&relative) {
                    continue;
                }
                if !CODE_EXTENSIONS
                    .iter()
                    .any(|ext| path.to_string_lossy().to_lowercase().ends_with(ext))
                {
                    continue;
                }
                candidates.push((relative, size));
            }
        }
        // Sort by size descending, then by path for determinism.
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (path, _) in candidates {
            if items.len() >= max_files {
                break;
            }
            if seen.contains(&path) {
                continue;
            }
            seen.insert(path.clone());
            items.push(json!({
                "path": path,
                "reason": "largest_code_file",
                "maxComplexity": Value::Null,
                "functionCount": Value::Null,
                "riskScore": Value::Null,
            }));
        }
    }

    items
}

/// Recursively walk `root` collecting (path, size) for regular files.
fn walk_code_files(root: &Path) -> std::io::Result<Vec<(PathBuf, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let size = match entry.metadata() {
                    Ok(metadata) => metadata.len(),
                    Err(_) => continue,
                };
                if size <= 1_048_576 {
                    out.push((path, size));
                }
            }
        }
    }
    Ok(out)
}

/// Relative path with / separators; preserves .. when the target is outside the repository.
fn relative_path(base: &Path, path: &Path) -> String {
    let base_components: Vec<_> = base.components().collect();
    let path_components: Vec<_> = path.components().collect();
    if base_components.first() != path_components.first() {
        return path.to_string_lossy().replace('\\', "/");
    }

    let common = base_components
        .iter()
        .zip(&path_components)
        .take_while(|(base, path)| base == path)
        .count();
    let mut relative = PathBuf::new();
    for component in &base_components[common..] {
        if !matches!(component, Component::CurDir) {
            relative.push("..");
        }
    }
    for component in &path_components[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    }
}

/// `git log --oneline --max-count=$limit -- $relative` with the hardened git
/// wrapper. Returns the commit summary lines, newest first.
fn recent_commits(repo: &Path, relative: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut command = hardened_git::command(repo);
    command
        .arg("--no-pager")
        .arg("log")
        .arg("--oneline")
        .arg(format!("--max-count={limit}"))
        .arg("--")
        .arg(relative);
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// `rg -n -m $limit --hidden` with the exclusion globs, searching for the
/// file stem as a fixed string. Returns the first `limit` match lines.
fn references(repo: &Path, relative: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let stem = Path::new(relative)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if stem.len() < 3 {
        return Vec::new();
    }

    let mut command = Command::new("rg");
    command
        .env_remove("RIPGREP_CONFIG_PATH")
        .arg("-n")
        .arg("-m")
        .arg(limit.to_string())
        .arg("--hidden")
        .arg("--sort")
        .arg("path")
        .arg("-g")
        .arg("!**/work/**")
        .arg("-g")
        .arg("!**/artifact/**")
        .arg("-g")
        .arg("!**/artifacts/**")
        .arg("-g")
        .arg("!**/staging/**")
        .arg("-g")
        .arg("!**/.code-intel/**")
        .arg("-g")
        .arg("!**/.git/**")
        .arg("-g")
        .arg("!**/node_modules/**")
        .arg("-g")
        .arg("!**/target/**")
        .arg("-g")
        .arg("!**/dist/**")
        .arg("-g")
        .arg("!**/build/**")
        .arg("-g")
        .arg("!**/.venv/**")
        .arg("-g")
        .arg("!**/__pycache__/**")
        .arg("--fixed-strings")
        .arg(&stem)
        .arg(repo);
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .take(limit)
        .collect()
}

/// Digest entry for one file: existence, LOC, first 12 lines.
fn file_digest(repo: &Path, relative: &str) -> Value {
    let path = repo.join(relative);
    if !path.is_file() {
        return json!({ "exists": false, "loc": 0, "firstLines": [] });
    }
    let content = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => String::new(),
    };
    let loc = content.lines().count();
    let first_lines: Vec<String> = content.lines().take(12).map(str::to_string).collect();
    json!({
        "exists": true,
        "loc": loc,
        "firstLines": first_lines,
    })
}

/// Build the CodeNexus-lite context document. Mirrors the facade payload
/// construction exactly (field names, nesting, summary counters).
pub(crate) fn build_context(
    repo: &Path,
    target: &Path,
    dsm_path: Option<&Path>,
    hotspots_path: Option<&Path>,
    max_files: usize,
    max_references_per_file: usize,
    max_commits_per_file: usize,
) -> Value {
    let read_json = |path: Option<&Path>| -> Option<Value> {
        let path = path?;
        if !path.is_file() {
            return None;
        }
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
    };
    let hotspots = read_json(hotspots_path);
    let dsm = read_json(dsm_path);

    let selected = select_hotspot_files(repo, target, hotspots.as_ref(), dsm.as_ref(), max_files);

    let mut file_contexts: Vec<Value> = Vec::new();
    for file in &selected {
        let relative = file["path"].as_str().unwrap_or("").to_string();
        if relative.is_empty() {
            continue;
        }
        file_contexts.push(json!({
            "path": relative,
            "reason": file["reason"],
            "maxComplexity": file["maxComplexity"],
            "functionCount": file["functionCount"],
            "riskScore": file["riskScore"],
            "digest": file_digest(repo, &relative),
            "recentCommits": recent_commits(repo, &relative, max_commits_per_file),
            "references": references(repo, &relative, max_references_per_file),
        }));
    }

    let mut total_references = 0usize;
    let mut total_commits = 0usize;
    for context in &file_contexts {
        total_references += context["references"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
        total_commits += context["recentCommits"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
    }

    let dsm_path_str = dsm_path
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let hotspots_path_str = hotspots_path
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    json!({
        "tool": "codenexus-lite",
        "generatedAt": iso_now(),
        "repo": repo.to_string_lossy(),
        "target": target.to_string_lossy(),
        "output": "",
        "sources": {
            "dsm": dsm_path_str,
            "hotspots": hotspots_path_str,
        },
        "summary": {
            "files": file_contexts.len(),
            "references": total_references,
            "recentCommits": total_commits,
        },
        "files": file_contexts,
        "nextQueries": [
            "Inspect top files by reason=sentrux_hotspot before editing.",
            "Use references to estimate blast radius before changing public functions.",
            "Use recentCommits to identify ownership or churn before accepting a baseline."
        ],
        "limitations": [
            "This is deterministic CodeNexus-lite context, not a semantic embedding graph.",
            "It is designed to be portable on a fresh machine and can be replaced by a full CodeNexus backend later."
        ],
    })
}

/// Build only the behavior reachable from the production compatibility
/// facade: largest-code-file fallback ranking, no Git history, and bounded
/// text references. DSM/hotspot inputs and history limits deliberately are
/// not accepted by this API (issue #337).
pub(crate) fn build_active_context(
    repo: &Path,
    target: &Path,
    output: &Path,
    generated_at: String,
    max_files: usize,
    max_references_per_file: usize,
) -> Value {
    let mut document = build_context(
        repo,
        target,
        None,
        None,
        max_files,
        max_references_per_file,
        0,
    );
    document["generatedAt"] = Value::String(generated_at);
    document["output"] = Value::String(output.to_string_lossy().into_owned());
    document
}

/// UTC ISO-8601 timestamp with millisecond precision (`2026-08-17T12:34:56.789Z`).
pub(crate) fn iso_now() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let secs = (nanos / 1_000_000_000) as i64;
    let millis = (nanos / 1_000_000 % 1_000) as i64;
    iso_from_unix_parts(secs, millis)
}

pub(crate) fn iso_from_unix_seconds(seconds: i64) -> String {
    iso_from_unix_parts(seconds, 0)
}

fn iso_from_unix_parts(secs: i64, millis: i64) -> String {
    // seconds since epoch -> civil time (days-from-civil, Howard Hinnant)
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The `implementation.id` reported in evidence. Was `invoke-codenexus-lite.ps1`
/// when the facade produced the document; the Rust implementation keeps the
/// same id so contract validators that admit `codenexus.lite-compat` keep
/// matching (issue #275 changes the executor, not the contract).
pub(crate) const IMPLEMENTATION_ID: &str = "invoke-codenexus-lite.ps1";

/// SHA-256 of this source file at build time — the digest the admission
/// route reports as `implementation.digest`. Kept in a function so the value
/// is computed once per process.
pub(crate) fn implementation_digest() -> String {
    sha256_hex(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/codenexus_lite.rs"
    )))
}

#[cfg(test)]
#[path = "codenexus_lite_tests.rs"]
mod tests;
