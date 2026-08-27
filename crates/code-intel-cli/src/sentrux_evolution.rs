//! Real Rust engines for Sentrux `evolution` and `what_if`.
//!
//! Ports `Invoke-EvolutionTool`/`Invoke-WhatIfTool` from
//! `legacy/Invoke-SentruxAgentTool.ps1` so `legacy/run-code-intel.ps1` can
//! call `code-intel sentrux evolution|what_if <path>` the same low-risk,
//! rust-first-then-ps1-fallback way it already calls `sentrux dsm` (see
//! `orchestration.rs`'s `structure.sentrux` participant).
//!
//! `what_if` (issue #374) is also nested directly into
//! `builtin_provider_evidence.rs`'s `run_sentrux` and is what the
//! `code-intel run execute` DAG path's `sentrux.what_if` capability now
//! calls, replacing `sentrux_lite_capabilities.rs`'s retired
//! `what_if_json` degraded fallback. `evolution` is not yet wired the same
//! way: the DAG's `"evolution"` arm still calls
//! `sentrux_lite_capabilities::evolution_json`, an intentionally
//! simplified ("lite") fallback not shape-compatible with what the PS1
//! orchestrator's `sentrux-evolution.json` artifact and downstream
//! hospital-report generation expect — see DR-0008 and its follow-up
//! tracking issue for that asymmetry.
//!
//! Split across two files (this one and `sentrux_what_if.rs`, included as a
//! private submodule below) purely to stay under this repo's monolith file
//! gate — `evolution` and `what_if` share the git/bus-factor helpers here
//! but are otherwise independent.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::sentrux_analysis;

#[path = "hardened_git.rs"]
mod hardened_git;

#[path = "sentrux_what_if.rs"]
mod what_if_impl;
pub use what_if_impl::what_if;

const SESSION_RECENT_DEFAULT: usize = 10;

// ---------------------------------------------------------------- git glue

fn run_git(repo: &Path, args: &[&str]) -> (i32, String) {
    match hardened_git::command(repo).args(args).output() {
        Ok(output) => {
            let code = output.status.code().unwrap_or(-1);
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            (code, text)
        }
        Err(_) => (-1, String::new()),
    }
}

fn git_show_prefix(repo: &Path) -> String {
    let (code, out) = run_git(repo, &["rev-parse", "--show-prefix"]);
    if code == 0 {
        out.trim().to_string()
    } else {
        String::new()
    }
}

fn git_author_log_for_files(repo: &Path, files: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    for batch in files.chunks(80) {
        let mut args: Vec<&str> = vec![
            "log",
            "--format=__SENTRUX_COMMIT__%ct\t%an <%ae>",
            "--name-only",
            "--",
        ];
        for file in batch {
            args.push(file.as_str());
        }
        let (code, output) = run_git(repo, &args);
        if code == 0 && !output.trim().is_empty() {
            lines.extend(
                output
                    .split('\n')
                    .map(|line| line.trim_end_matches('\r').to_string()),
            );
        }
    }
    lines
}

fn normalize_relative_file_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/").trim().to_string();
    while let Some(rest) = normalized.strip_prefix("./") {
        normalized = rest.to_string();
    }
    normalized
}

fn resolve_git_file_key(
    path_from_git: &str,
    prefix: &str,
    known: &HashMap<String, AuthorSignal>,
) -> Option<String> {
    let mut candidate = normalize_relative_file_path(path_from_git);
    if !prefix.trim().is_empty() {
        let normalized_prefix = normalize_relative_file_path(prefix);
        if candidate
            .to_ascii_lowercase()
            .starts_with(&normalized_prefix.to_ascii_lowercase())
        {
            candidate = candidate[normalized_prefix.len()..]
                .trim_start_matches('/')
                .to_string();
        }
    }
    if known.contains_key(&candidate) {
        return Some(candidate);
    }
    let candidate_lower = candidate.to_ascii_lowercase();
    for key in known.keys() {
        let key_lower = key.to_ascii_lowercase();
        if candidate_lower.ends_with(&format!("/{key_lower}"))
            || key_lower.ends_with(&format!("/{candidate_lower}"))
        {
            return Some(key.clone());
        }
    }
    None
}

#[derive(Default, Clone)]
struct AuthorSignal {
    authors: HashMap<String, i64>,
}

/// Per-file commit-author touch counts, from one batched `git log --name-only`
/// walk (mirrors `Get-GitAuthorSignals`). `last_author`/`last_commit_unix`
/// from the PS1 version are dropped: nothing downstream reads them there
/// either (`New-BusFactorEntry` only consumes the `authors` map).
fn git_author_signals(repo: &Path, files: &[String]) -> HashMap<String, AuthorSignal> {
    let mut signals: HashMap<String, AuthorSignal> = HashMap::new();
    for file in files {
        signals
            .entry(normalize_relative_file_path(file))
            .or_default();
    }
    if signals.is_empty() {
        return signals;
    }
    let prefix = git_show_prefix(repo);
    let known: Vec<String> = signals.keys().cloned().collect();
    let lines = git_author_log_for_files(repo, &known);

    let mut current_author: Option<String> = None;
    for raw_line in lines {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("__SENTRUX_COMMIT__") {
            if let Some((_, author)) = rest.split_once('\t') {
                current_author = Some(author.trim().to_string());
            }
            continue;
        }
        let Some(author) = current_author.clone() else {
            continue;
        };
        if author.is_empty() {
            continue;
        }
        let Some(key) = resolve_git_file_key(trimmed, &prefix, &signals) else {
            continue;
        };
        if let Some(entry) = signals.get_mut(&key) {
            *entry.authors.entry(author).or_insert(0) += 1;
        }
    }
    signals
}

// ------------------------------------------------------------- bus factor

fn bus_factor_risk(author_count: i64, top_author_share: f64) -> i64 {
    if author_count <= 0 {
        return 100;
    }
    let share = top_author_share.clamp(0.0, 1.0);
    let risk = (1.0 / author_count as f64) * 55.0 + share * 45.0;
    risk.clamp(0.0, 100.0).round() as i64
}

fn author_counts(authors: &HashMap<String, i64>) -> Vec<Value> {
    let mut list: Vec<(&String, &i64)> = authors.iter().collect();
    list.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    list.into_iter()
        .map(|(author, touches)| json!({"author": author, "touches": touches}))
        .collect()
}

fn bus_factor_entry(
    id: &str,
    name: &str,
    authors: &HashMap<String, i64>,
    files: i64,
    extra: Vec<(&str, Value)>,
) -> Value {
    let author_list = author_counts(authors);
    let touches: i64 = authors.values().sum();
    let top_touches = author_list
        .first()
        .and_then(|entry| entry["touches"].as_i64())
        .unwrap_or(0);
    let top_share = if touches > 0 {
        (top_touches as f64 / touches as f64 * 10000.0).round() / 10000.0
    } else {
        1.0
    };
    let top_author = author_list
        .first()
        .and_then(|entry| entry["author"].as_str())
        .map(str::to_string);

    let mut object = serde_json::Map::new();
    object.insert("id".into(), json!(id));
    object.insert("name".into(), json!(name));
    object.insert("files".into(), json!(files));
    object.insert("bus_factor".into(), json!(author_list.len()));
    object.insert(
        "bus_factor_risk".into(),
        json!(bus_factor_risk(author_list.len() as i64, top_share)),
    );
    object.insert("touches".into(), json!(touches));
    object.insert("top_author".into(), json!(top_author));
    object.insert("top_author_share".into(), json!(top_share));
    object.insert("authors".into(), json!(author_list));
    for (key, value) in extra {
        object.insert(key.to_string(), value);
    }
    Value::Object(object)
}

pub(super) fn as_array(value: &Value, key: &str) -> Vec<Value> {
    value[key]
        .as_array()
        .map(|items| items.clone())
        .unwrap_or_default()
}

fn sort_desc_by_f64(items: &mut [Value], path: &[&str]) {
    items.sort_by(|a, b| {
        let mut av = a;
        let mut bv = b;
        for key in path {
            av = &av[*key];
            bv = &bv[*key];
        }
        bv.as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&av.as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

// -------------------------------------------------------------- evolution

fn evolution_hotspots(dsm: &Value) -> Value {
    let mut function_hotspots: Vec<Value> = Vec::new();
    for file in as_array(dsm, "file_details") {
        for function in as_array(&file, "functions") {
            function_hotspots.push(json!({
                "id": function["id"],
                "fileId": file["id"],
                "file": file["path"],
                "name": function["name"],
                "complexity": function["complexity"],
                "loc": function["loc"],
                "params": function["params"],
                "sourceAnchor": function["source_anchor"],
            }));
        }
    }
    sort_desc_by_f64(&mut function_hotspots, &["complexity"]);
    function_hotspots.truncate(50);

    let mut modules = as_array(dsm, "modules");
    sort_desc_by_f64(&mut modules, &["colors", "Risk", "score"]);
    let top_modules: Vec<Value> = modules
        .into_iter()
        .take(20)
        .map(|module| {
            json!({
                "id": module["id"],
                "name": module["name"],
                "risk": module["metrics"]["risk"],
                "riskScore": module["colors"]["Risk"]["score"],
                "files": module["files"],
                "coupling": module["metrics"]["coupling"],
                "blastRadius": module["metrics"]["blast_radius"],
                "gitFiles": module["metrics"]["git_files"],
            })
        })
        .collect();

    let mut files = as_array(dsm, "file_details");
    sort_desc_by_f64(&mut files, &["max_complexity"]);
    let top_files: Vec<Value> = files
        .into_iter()
        .take(30)
        .map(|file| {
            json!({
                "id": file["id"],
                "path": file["path"],
                "sourceAnchor": file["source_anchor"],
                "functionCount": file["function_count"],
                "maxComplexity": file["max_complexity"],
                "avgComplexity": file["avg_complexity"],
                "loc": file["loc"],
                "git": file["git"],
            })
        })
        .collect();

    json!({
        "modules": top_modules,
        "files": top_files,
        "functions": function_hotspots,
    })
}

fn evolution_coupling(dsm: &Value) -> Value {
    let total_modules = as_array(dsm, "modules").len();
    let total_edges = as_array(dsm, "edges").len();

    let mut modules = as_array(dsm, "modules");
    sort_desc_by_f64(&mut modules, &["metrics", "coupling"]);
    let top_modules: Vec<Value> = modules
        .into_iter()
        .take(30)
        .map(|module| {
            json!({
                "id": module["id"],
                "name": module["name"],
                "coupling": module["metrics"]["coupling"],
                "inbound": module["metrics"]["inbound_edges"],
                "outbound": module["metrics"]["outbound_edges"],
                "blastRadius": module["metrics"]["blast_radius"],
                "execDepth": module["metrics"]["exec_depth"],
                "risk": module["metrics"]["risk"],
            })
        })
        .collect();

    let mut edges = as_array(dsm, "edges");
    sort_desc_by_f64(&mut edges, &["count"]);
    let top_edges: Vec<Value> = edges
        .into_iter()
        .take(50)
        .map(|edge| {
            json!({
                "id": edge["id"], "from": edge["from"], "to": edge["to"], "count": edge["count"],
            })
        })
        .collect();

    let max_coupling = top_modules
        .first()
        .map(|m| m["coupling"].clone())
        .unwrap_or(json!(0));
    let top_module = top_modules.first().map(|m| m["name"].clone());

    json!({
        "summary": {
            "modules": total_modules,
            "edges": total_edges,
            "maxCoupling": max_coupling,
            "topModule": top_module,
        },
        "modules": top_modules,
        "edges": top_edges,
    })
}

pub(super) fn evolution_bus_factor(repo: &Path, dsm: &Value) -> Value {
    let file_details = as_array(dsm, "file_details");
    let files: Vec<String> = file_details
        .iter()
        .filter_map(|f| f["path"].as_str().map(str::to_string))
        .collect();
    let author_signals = git_author_signals(repo, &files);

    let mut file_entries: Vec<Value> = Vec::new();
    let mut module_state: HashMap<String, (HashMap<String, i64>, i64, Vec<String>)> =
        HashMap::new();

    for file in &file_details {
        let path = file["path"].as_str().unwrap_or_default().to_string();
        let module = file["module"].as_str().unwrap_or_default().to_string();
        let module_entry = module_state
            .entry(module.clone())
            .or_insert_with(|| (HashMap::new(), 0, Vec::new()));
        module_entry.1 += 1;
        module_entry.2.push(path.clone());

        let key = normalize_relative_file_path(&path);
        let authors = author_signals
            .get(&key)
            .map(|signal| signal.authors.clone())
            .unwrap_or_default();
        for (author, count) in &authors {
            *module_entry.0.entry(author.clone()).or_insert(0) += count;
        }

        file_entries.push(bus_factor_entry(
            file["id"].as_str().unwrap_or_default(),
            &path,
            &authors,
            1,
            vec![
                ("path", json!(path)),
                ("module", json!(module)),
                ("sourceAnchor", file["source_anchor"].clone()),
                ("functionCount", file["function_count"].clone()),
                ("maxComplexity", file["max_complexity"].clone()),
                ("git", file["git"].clone()),
            ],
        ));
    }

    let mut module_entries: Vec<Value> = module_state
        .into_iter()
        .map(|(module, (authors, files_count, paths))| {
            bus_factor_entry(
                &sentrux_analysis::stable_id(&format!("module:{module}")),
                &module,
                &authors,
                files_count,
                vec![("paths", json!(paths))],
            )
        })
        .collect();

    module_entries.sort_by(|a, b| {
        b["bus_factor_risk"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["bus_factor_risk"].as_i64().unwrap_or(0))
    });
    file_entries.sort_by(|a, b| {
        b["bus_factor_risk"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["bus_factor_risk"].as_i64().unwrap_or(0))
    });

    let highest_module_risk = module_entries
        .first()
        .map(|m| m["bus_factor_risk"].clone())
        .unwrap_or(json!(0));
    let highest_file_risk = file_entries
        .first()
        .map(|f| f["bus_factor_risk"].clone())
        .unwrap_or(json!(0));
    let module_count = module_entries.len();
    let file_count = file_entries.len();
    module_entries.truncate(30);
    file_entries.truncate(50);

    json!({
        "summary": {
            "modules": module_count,
            "files": file_count,
            "highestModuleRisk": highest_module_risk,
            "highestFileRisk": highest_file_risk,
        },
        "modules": module_entries,
        "files": file_entries,
    })
}

fn session_dir(repo: &Path) -> PathBuf {
    repo.join(".sentrux").join("agent-sessions")
}

fn read_json_file(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn quality_signal(value: Option<&Value>) -> Value {
    let Some(number) = value.and_then(Value::as_f64) else {
        return Value::Null;
    };
    if number <= 1.0 {
        json!((number * 10000.0).round() as i64)
    } else {
        json!(number.round() as i64)
    }
}

fn read_sessions(repo: &Path, limit: usize) -> Vec<Value> {
    let dir = session_dir(repo);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut starts: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".start.json"))
        })
        .collect();
    starts.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    starts.truncate(limit);

    starts
        .into_iter()
        .map(|start_path| {
            let file_name = start_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let id = file_name.trim_end_matches(".start.json");
            let end_path = dir.join(format!("{id}.end.json"));
            let start = read_json_file(&start_path);
            let end = read_json_file(&end_path);
            json!({
                "session_id": id,
                "start_signal": quality_signal(start.as_ref().and_then(|s| s.get("quality_signal"))),
                "end_signal": quality_signal(end.as_ref().and_then(|e| e.get("signal_after"))),
                "pass": end.as_ref().and_then(|e| e.get("pass")).cloned().unwrap_or(Value::Null),
                "started_at": start.as_ref().and_then(|s| s.get("started_at")).cloned().unwrap_or(Value::Null),
                "ended_at": end.as_ref().and_then(|e| e.get("ended_at")).cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn session_trend(sessions: &[Value]) -> Value {
    let completed: Vec<&Value> = sessions
        .iter()
        .filter(|session| !session["end_signal"].is_null())
        .collect();
    let failed_count = sessions
        .iter()
        .filter(|session| session["pass"] == json!(false))
        .count();
    let deltas: Vec<i64> = completed
        .iter()
        .filter_map(|session| {
            match (
                session["start_signal"].as_i64(),
                session["end_signal"].as_i64(),
            ) {
                (Some(start), Some(end)) => Some(end - start),
                _ => None,
            }
        })
        .collect();
    let total_delta: i64 = deltas.iter().sum();
    json!({
        "sessions": sessions.len(),
        "completed": completed.len(),
        "failed": failed_count,
        "totalSignalDelta": total_delta,
        "lastSignalDelta": deltas.first(),
        "direction": if total_delta > 0 { "improving" } else if total_delta < 0 { "degrading" } else { "stable" },
    })
}

pub fn evolution(repo: &Path) -> Result<Value, String> {
    let sessions = read_sessions(repo, SESSION_RECENT_DEFAULT);
    let trend = session_trend(&sessions);
    let dsm = sentrux_analysis::analyze(repo)?;
    let hotspots = evolution_hotspots(&dsm);
    let coupling = evolution_coupling(&dsm);
    let bus_factor = evolution_bus_factor(repo, &dsm);
    Ok(json!({
        "tool": "evolution",
        "path": repo.display().to_string(),
        "sessions": sessions,
        "count": sessions.len(),
        "trend": trend,
        "hotspots": hotspots,
        "coupling": coupling,
        "bus_factor": bus_factor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    pub(super) fn init_repo(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "code-intel-sentrux-evolution-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("src dir");
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .output()
            .expect("git init");
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&root)
            .output()
            .expect("git config email");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&root)
            .output()
            .expect("git config name");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn sample() { if true { println!(\"x\"); } }\n",
        )
        .expect("write source");
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&root)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(&root)
            .output()
            .expect("git commit");
        root
    }

    #[test]
    fn evolution_produces_hotspots_coupling_and_bus_factor_shapes() {
        let repo = init_repo("evolution");
        let doc = evolution(&repo).expect("evolution should run against a real git repo");
        assert_eq!(doc["tool"], "evolution");
        assert_eq!(doc["count"], 0);
        assert!(doc["hotspots"]["functions"].is_array());
        assert!(doc["coupling"]["modules"].is_array());
        assert!(doc["bus_factor"]["files"].is_array());
        assert!(doc["bus_factor"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |entry| entry["path"] == "src/lib.rs" && entry["touches"].as_i64().unwrap_or(0) > 0
            ));
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn bus_factor_risk_matches_ps1_formula() {
        assert_eq!(bus_factor_risk(0, 0.0), 100);
        assert_eq!(bus_factor_risk(1, 1.0), 100);
        assert_eq!(bus_factor_risk(2, 0.5), 50);
    }
}
