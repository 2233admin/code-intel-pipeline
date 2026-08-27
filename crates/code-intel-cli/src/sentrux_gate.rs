//! Built-in structural gate engine.
//!
//! This is the default engine behind `code-intel sentrux check|gate` and the
//! `evidence.sentrux` DAG node. It replaces the PATH-resolved `sentrux`
//! forwarder as the default so the gate verdict is a function of the snapshot
//! and this binary alone. An external Sentrux can still be injected through
//! `options.toolPathPrefix` (tests, V-lang overlay).
//!
//! Metric formulas continue the sentrux-lite scale (quality 0..10000,
//! coupling = imports per file x 10) so `.sentrux/rules.toml` thresholds keep
//! their meaning, with two deliberate differences: `cycle_count` is computed
//! from resolved Rust module imports (sentrux-lite hardcoded it to zero), and
//! coupling counts only files written in a language whose dependency syntax
//! this scanner actually models (`IMPORT_MODELED_EXTENSIONS`).
//!
//! The coupling denominator matters more than it looks. `is_import_line` reads
//! `import` / `from` / `use` / `mod` / `require(` / `#include` / `using`; none
//! of those is how PowerShell declares a dependency (it dot-sources and calls
//! `Import-Module`). Counting `.ps1` files in the denominator therefore made
//! the score fall whenever PowerShell grew and rise whenever it shrank: the
//! metric read "share of PowerShell", not "coupling". In a repository midway
//! through replacing PowerShell with Rust, that inverted the gate — every
//! correct migration step registered as structural regression. Restricting
//! both numerator and denominator to modelled languages leaves the score
//! unchanged for repositories that have none of the unmodelled ones.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[path = "boundary_rules.rs"]
mod boundary_rules;
#[path = "file_gate/mod.rs"]
mod file_gate;
#[path = "hardened_git.rs"]
mod hardened_git;
#[path = "sentrux_quality_signal.rs"]
mod sentrux_quality_signal;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub(crate) const ENGINE_ID: &str = "sentrux-native";
pub(crate) const ENGINE_VERSION: &str = "3.0.0";
// The schema tracks metric *definitions*, not just field names, because every
// gate comparison is a comparison against numbers a previous engine produced.
//   v3 (2.1.0): `coupling_score` changed denominator.
//   v4 (2.2.0): `complexity_terms` stopped counting comments and string
//               bodies, which raises `quality_signal` on any tree that had
//               prose in it.
//   v5 (2.2.0): baselines record the `godFiles` identity list. A v4 count
//               ratchet left slack a brand-new god file could hide in, and
//               a fix+regress swap kept the count flat while a new monolith
//               appeared (#165) — the ratchet now compares file identities,
//               which a count-only baseline cannot express.
//   v6 (3.0.0): `quality_signal` is no longer `10000 - penalty` (a proxy
//               score with no upstream equivalent, issue #385). It is now
//               the upstream-compatible Quality Signal: the geometric mean
//               of five normalized root-cause scores (modularity,
//               acyclicity, depth, equality, redundancy), ported from
//               https://sentrux.dev/docs/quality-signal/ and pinned source
//               commit `6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11` (see
//               `sentrux_quality_signal.rs`). The two scales are not
//               comparable numbers — an older baseline's `quality_signal`
//               was never a Quality Signal at all — so this is a schema
//               bump, not just an engine-version bump, and a v5 baseline
//               must fail closed via `baseline_engine_mismatch` rather than
//               produce a fabricated before/after delta. See DR-0011.
// These moves make an older baseline describe a tree this engine cannot
// reproduce. Comparing anyway is worse than refusing: v2 would have reported
// a fabricated coupling regression, and v3 would silently pocket a quality
// gain instead of re-anchoring the ratchet. The mismatch turns either case
// into `baseline_engine_mismatch` with the re-baseline instruction.
pub(crate) const BASELINE_SCHEMA: &str = "code-intel-sentrux-baseline.v6";

// Metric keys the gate comparisons in `run_gate` read from the baseline.
// `number()` defaults an absent key to 0.0, so a baseline missing any of
// these would silently disable its comparison instead of failing closed.
const GATED_METRIC_KEYS: [&str; 4] = [
    "quality_signal",
    "coupling_score",
    "cycle_count",
    "god_file_count",
];

// Subset of `file_gate::CODE_EXTENSIONS` whose dependency declarations `is_import_line`
// recognises. Everything measured except coupling (size, functions,
// complexity, god files) still covers the full CODE_EXTENSIONS set — a
// PowerShell god file is still a god file.
// `every_code_extension_is_classified_as_import_modeled_or_not` keeps this
// list from silently drifting when CODE_EXTENSIONS grows.
const IMPORT_MODELED_EXTENSIONS: &[&str] = &[
    "py", "rs", "go", "ts", "tsx", "js", "jsx", "mjs", "cjs", "java", "cs", "cpp", "c", "h", "hpp",
    "v",
];
// PowerShell declares dependencies by dot-sourcing and `Import-Module`; this
// scanner reads neither, so counting .ps1/.psm1 files would only dilute the
// average. Adding either form to `is_import_line` is the prerequisite for
// moving these into IMPORT_MODELED_EXTENSIONS.
const IMPORT_UNMODELED_EXTENSIONS: &[&str] = &["ps1", "psm1"];
const MAX_VIOLATION_TARGETS: usize = 8;
// The god-file rule, single source for both measurement (`measure_file`) and
// reporting (`god_rule_label`) so the printed rule branch can never drift
// from the judged one. Function counting deliberately includes test
// functions; the decision and its rationale live in `.sentrux/rules.toml`.
const GOD_FILE_LOC_LIMIT: i64 = 800;
const GOD_FILE_FN_LIMIT: i64 = 25;
const GOD_FILE_FN_LOC_LIMIT: i64 = 400;

#[derive(Clone, Debug)]
pub(crate) struct Violation {
    pub(crate) rule: String,
    pub(crate) message: String,
    pub(crate) targets: Vec<String>,
}

impl Violation {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "rule": self.rule,
            "message": self.message,
            "targets": self.targets,
        })
    }
}

pub(crate) struct EngineRun {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) violations: Vec<Violation>,
    /// False when the repository never configured the governance this run
    /// needs: no `.sentrux/rules.toml` for `check`, no `.sentrux/baseline.json`
    /// for `gate`. Ungoverned is not the same as violated — no rule was
    /// evaluated and no regression was measurable. The CLI still exits nonzero
    /// so an operator who asked for a gate is told to save a baseline, but
    /// evidence consumers must not read that exit code as a structural verdict.
    pub(crate) governed: bool,
}

#[derive(Default)]
struct FileMetrics {
    path: String,
    loc: i64,
    functions: i64,
    max_complexity: i64,
    imports: i64,
    calls: i64,
    god_file: bool,
    complex_file: bool,
    /// Whether this file's language declares dependencies in a form
    /// `is_import_line` reads. False files are excluded from both sides of
    /// the coupling ratio.
    import_modeled: bool,
}

struct ProjectMetrics {
    files: Vec<FileMetrics>,
    file_count: i64,
    import_modeled_file_count: i64,
    function_count: i64,
    total_import_edges: i64,
    call_edges: i64,
    god_file_count: i64,
    complex_fn_count: i64,
    max_complexity: i64,
    coupling_score: f64,
    /// Upstream-compatible Quality Signal (issue #385): geometric mean of
    /// the five normalized root causes, 0..10000. Replaces the old
    /// `10000 - penalty` proxy — see `sentrux_quality_signal.rs`.
    quality_signal: i64,
    quality_signal_detail: sentrux_quality_signal::QualitySignal,
    quality_signal_raw: sentrux_quality_signal::RootCauseRaw,
    cycle_count: i64,
    cycles: Vec<Vec<String>>,
}

pub(crate) fn run_check(repo: &Path) -> Result<EngineRun, String> {
    let (metrics, _file_gate) = measure_project(repo)?;
    let mut out = String::new();
    resolve_header(&mut out, &metrics);
    let rules_path = repo.join(".sentrux").join("rules.toml");
    if !rules_path.is_file() {
        out.push_str("No .sentrux/rules.toml found\nQuality: not gated\n");
        return Ok(EngineRun {
            success: true,
            stdout: out,
            violations: Vec::new(),
            governed: false,
        });
    }
    let rules = fs::read_to_string(&rules_path)
        .map_err(|error| format!("read {}: {error}", rules_path.display()))?;
    let violations = evaluate_rules(&rules, &metrics, repo);
    if violations.is_empty() {
        out.push_str(&format!(
            "All rules passed - Quality: {}\n",
            metrics.quality_signal
        ));
        return Ok(EngineRun {
            success: true,
            stdout: out,
            violations,
            governed: true,
        });
    }
    out.push_str("Sentrux check failed\n");
    for violation in &violations {
        out.push_str(&format!("- {}\n", violation.message));
        for target in &violation.targets {
            out.push_str(&format!("  target: {target}\n"));
        }
    }
    Ok(EngineRun {
        success: false,
        stdout: out,
        violations,
        governed: true,
    })
}

/// Test-only wrapper around `run_check`: an `Err` here means the sentrux
/// engine itself did not complete (subprocess spawn, git, or path-resolution
/// problem -- e.g. the #178 shared-temp-path collision, or any other reason
/// `measure_project` bailed), which is not the same failure as a real rule
/// violation and must never be reported as one (#192). Panics with a message
/// that cannot be mistaken for the business assertion below it: readers
/// hitting this text go straight to the actual error, not the dependency
/// graph. Deliberately does not guess a specific cause (subprocess vs. path
/// vs. something else) beyond what `error` itself already says -- claiming a
/// cause this helper cannot verify would just be a differently-shaped
/// misdiagnosis.
#[cfg(test)]
pub(crate) fn expect_check_ran(repo: &Path) -> EngineRun {
    match run_check(repo) {
        Ok(run) => run,
        Err(error) => panic!(
            "sentrux engine did not complete against {} -- nothing below was evaluated this \
             run; this is NOT a rule-violation failure, do not chase the dependency graph. \
             The underlying error names the actual cause: {error}",
            repo.display()
        ),
    }
}

pub(crate) fn run_gate(repo: &Path, save: bool) -> Result<EngineRun, String> {
    let (metrics, _file_gate) = measure_project(repo)?;
    let baseline_path = repo.join(".sentrux").join("baseline.json");
    if save {
        let baseline = baseline_document(repo, &metrics)?;
        let directory = baseline_path
            .parent()
            .ok_or("baseline path has no parent directory")?;
        fs::create_dir_all(directory)
            .map_err(|error| format!("create {}: {error}", directory.display()))?;
        let mut bytes = serde_json::to_vec_pretty(&baseline)
            .map_err(|error| format!("serialize baseline: {error}"))?;
        bytes.push(b'\n');
        fs::write(&baseline_path, bytes)
            .map_err(|error| format!("write {}: {error}", baseline_path.display()))?;
        let mut out = String::new();
        resolve_header(&mut out, &metrics);
        gate_pairs(&mut out, &metrics, &metrics);
        out.push_str(&format!("Baseline saved: {}\n", baseline_path.display()));
        return Ok(EngineRun {
            success: true,
            stdout: out,
            violations: Vec::new(),
            governed: true,
        });
    }
    if !baseline_path.is_file() {
        let message = format!("Sentrux baseline missing at {}", baseline_path.display());
        return Ok(EngineRun {
            success: false,
            stdout: format!(
                "{message}\nRun code-intel sentrux --operation save_baseline --repo {}\n",
                repo.display()
            ),
            violations: vec![Violation {
                rule: "baseline_missing".into(),
                message,
                targets: vec![".sentrux/baseline.json".into()],
            }],
            governed: false,
        });
    }
    let raw = fs::read(&baseline_path)
        .map_err(|error| format!("read {}: {error}", baseline_path.display()))?;
    let baseline: Value = serde_json::from_slice(&raw)
        .map_err(|error| format!("parse {}: {error}", baseline_path.display()))?;
    let mut out = String::new();
    resolve_header(&mut out, &metrics);
    if baseline["schema"] != BASELINE_SCHEMA
        || baseline["engine"]["id"] != ENGINE_ID
        || baseline["metrics"].as_object().is_none()
        || GATED_METRIC_KEYS
            .iter()
            .any(|key| baseline["metrics"][*key].as_f64().is_none())
        || baseline["godFiles"].as_array().is_none()
    {
        let message = format!(
            "baseline engine mismatch: {} requires schema {BASELINE_SCHEMA} with engine {ENGINE_ID}, numeric {} metrics, and a godFiles identity list; found schema {} engine {}",
            baseline_path.display(),
            GATED_METRIC_KEYS.join("/"),
            baseline["schema"].as_str().unwrap_or("unknown"),
            baseline["engine"]["id"].as_str().unwrap_or("unknown"),
        );
        out.push_str(&format!("{message}\n"));
        out.push_str("Re-baseline intentionally with: code-intel sentrux --operation save_baseline --repo <repo>\n");
        return Ok(EngineRun {
            success: false,
            stdout: out,
            violations: vec![Violation {
                rule: "baseline_engine_mismatch".into(),
                message,
                targets: vec![".sentrux/baseline.json".into()],
            }],
            // A baseline exists but this engine cannot read it. That is a real
            // gate failure: re-baselining must stay a deliberate decision.
            governed: true,
        });
    }
    let before = &baseline["metrics"];
    gate_pairs_values(&mut out, before, &metrics);
    let baseline_god_paths: std::collections::BTreeSet<String> = baseline["godFiles"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry["path"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // A baseline that still lists files no longer over threshold grandfathers
    // a regression back into them silently. The gate cannot rewrite the
    // baseline itself — a scan must not mutate the repository — so it says
    // out loud that the ratchet has slack to reclaim.
    let resolved_since_baseline = baseline_god_paths
        .iter()
        .filter(|path| {
            !metrics
                .files
                .iter()
                .any(|file| file.god_file && &file.path == *path)
        })
        .count();
    if resolved_since_baseline > 0 {
        out.push_str(&format!(
            "Baseline lists {resolved_since_baseline} god file(s) no longer over threshold; tighten with: code-intel sentrux --operation save_baseline\n"
        ));
    }
    let mut violations = Vec::new();
    let quality_before = number(before, "quality_signal");
    if (metrics.quality_signal as f64) < quality_before {
        violations.push(Violation {
            rule: "quality_degraded".into(),
            message: format!("Quality: {} -> {}", quality_before, metrics.quality_signal),
            targets: Vec::new(),
        });
    }
    if metrics.coupling_score > number(before, "coupling_score") {
        violations.push(Violation {
            rule: "coupling_increased".into(),
            message: format!(
                "Coupling: {} -> {}",
                number(before, "coupling_score"),
                metrics.coupling_score
            ),
            targets: top_import_files(&metrics),
        });
    }
    if (metrics.cycle_count as f64) > number(before, "cycle_count") {
        violations.push(Violation {
            rule: "cycles_increased".into(),
            message: format!(
                "Cycles: {} -> {}",
                number(before, "cycle_count"),
                metrics.cycle_count
            ),
            targets: cycle_targets(&metrics),
        });
    }
    // Ratchet by identity, not by count (#165). The count comparison this
    // replaces had two leaks: a loose baseline count left slack a brand-new
    // god file could ship inside, and a same-change fix+regress swap kept the
    // count flat while a new monolith appeared. Any current god file whose
    // path the baseline does not list is a regression; listed files are
    // tolerated standing debt. A rename reads as a new path and trips this —
    // re-baselining after a rename is a deliberate operator decision.
    let mut new_gods: Vec<&FileMetrics> = metrics
        .files
        .iter()
        .filter(|file| file.god_file && !baseline_god_paths.contains(&file.path))
        .collect();
    new_gods.sort_by(|left, right| {
        right
            .loc
            .cmp(&left.loc)
            .then_with(|| left.path.cmp(&right.path))
    });
    if !new_gods.is_empty() {
        let mut described: Vec<String> = new_gods
            .iter()
            .take(MAX_VIOLATION_TARGETS)
            .map(|file| {
                format!(
                    "{} (loc {}, functions {}; rule: {})",
                    file.path,
                    file.loc,
                    file.functions,
                    god_rule_label(file.loc, file.functions)
                )
            })
            .collect();
        if new_gods.len() > MAX_VIOLATION_TARGETS {
            described.push(format!("+{} more", new_gods.len() - MAX_VIOLATION_TARGETS));
        }
        violations.push(Violation {
            rule: "god_files_increased".into(),
            message: format!(
                "God files: {} -> {}; new since baseline: {}",
                number(before, "god_file_count"),
                metrics.god_file_count,
                described.join(", ")
            ),
            targets: new_gods
                .iter()
                .take(MAX_VIOLATION_TARGETS)
                .map(|file| file.path.clone())
                .collect(),
        });
    }
    if violations.is_empty() {
        out.push_str("No degradation detected\n");
        return Ok(EngineRun {
            success: true,
            stdout: out,
            violations,
            governed: true,
        });
    }
    out.push_str("Quality degraded during this session\n");
    for violation in &violations {
        out.push_str(&format!("- {}\n", violation.message));
    }
    Ok(EngineRun {
        success: false,
        stdout: out,
        violations,
        governed: true,
    })
}

/// Test-only wrapper around `run_gate`, same rationale as
/// `expect_check_ran` (#192): an engine-completion failure here must never
/// read as a baseline/ratchet business failure, and this deliberately does
/// not guess a cause beyond what `error` already says.
#[cfg(test)]
pub(crate) fn expect_gate_ran(repo: &Path, save: bool) -> EngineRun {
    match run_gate(repo, save) {
        Ok(run) => run,
        Err(error) => panic!(
            "sentrux engine did not complete against {} -- nothing below was evaluated this \
             run; this is NOT a rule-violation failure, do not chase the dependency graph. \
             The underlying error names the actual cause: {error}",
            repo.display()
        ),
    }
}

/// The CLI-level `check` operation: `run_check`'s static `.sentrux/rules.toml`
/// verdict, composed with `run_gate`'s baseline ratchet verdict so the two
/// can never disagree for the same tree (issue #106).
///
/// Before this, `code-intel sentrux check` called only `run_check` and never
/// opened `.sentrux/baseline.json`, so it could report green on a tree the
/// authoritative `evidence.sentrux` DAG node -- which always evaluates
/// `run_check` *and* `run_gate` as two independent rules, see
/// `builtin_provider_evidence::sentrux_admission` -- correctly failed on.
/// `.sentrux/rules.toml` in this very repository documents the assumption
/// that made that possible: `no_god_files` is deliberately left `false`
/// because god-file monotonicity was meant to be enforced by the baseline
/// ratchet, not the static rule -- a promise only `run_gate` was keeping.
///
/// `ratchet=false` (the CLI's explicit `--no-ratchet` escape hatch) restores
/// the pre-fix static-only verdict and says so in `stdout`, so a caller can
/// never mistake it for the honest default.
///
/// An ungoverned gate (no `.sentrux/baseline.json` saved yet) does not fail
/// this check: absence of a baseline is an operator affordance, not a
/// structural verdict, mirroring `command_rule` in
/// `builtin_provider_evidence.rs`.
///
/// `builtin_provider_evidence::run_sentrux` deliberately keeps calling plain
/// `run_check` for its own `"check"` rule -- it already runs `run_gate`
/// separately as the `"sentrux_gate"` rule, so composing the ratchet in here
/// too would just report the same violation twice under two rule kinds.
pub(crate) fn run_check_aligned(repo: &Path, ratchet: bool) -> Result<EngineRun, String> {
    let mut check = run_check(repo)?;
    if !ratchet {
        check.stdout.push_str(
            "Ratchet comparison skipped (--no-ratchet): this verdict does not reflect any \
             regression against .sentrux/baseline.json. The authoritative gate always \
             evaluates that ratchet; do not treat this verdict as a substitute for it.\n",
        );
        return Ok(check);
    }
    let gate = run_gate(repo, false)?;
    let gate_pass = gate.success || !gate.governed;
    let mut stdout = String::new();
    stdout.push_str("-- .sentrux/rules.toml --\n");
    stdout.push_str(&check.stdout);
    stdout.push_str("-- .sentrux/baseline.json ratchet --\n");
    stdout.push_str(&gate.stdout);
    let mut violations = check.violations;
    if !gate_pass {
        violations.extend(gate.violations);
    }
    Ok(EngineRun {
        success: check.success && gate_pass,
        stdout,
        violations,
        governed: check.governed || gate.governed,
    })
}

pub(crate) fn scan_json(repo: &Path) -> Result<Value, String> {
    let (metrics, file_gate) = measure_project(repo)?;
    let mut value = metrics_json(repo, &metrics);
    if let Value::Object(ref mut map) = value {
        map.insert("file_gate".to_string(), file_gate.to_json());
    }
    Ok(value)
}

fn baseline_document(repo: &Path, metrics: &ProjectMetrics) -> Result<Value, String> {
    let saved_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("read clock: {error}"))?
        .as_secs();
    Ok(json!({
        "schema": BASELINE_SCHEMA,
        "engine": {"id": ENGINE_ID, "version": ENGINE_VERSION},
        "sourceCommit": git_head(repo),
        "savedAt": saved_at,
        "scope": ".",
        "metrics": {
            "quality_signal": metrics.quality_signal,
            "coupling_score": metrics.coupling_score,
            "cycle_count": metrics.cycle_count,
            "god_file_count": metrics.god_file_count,
            "complex_fn_count": metrics.complex_fn_count,
            "max_complexity": metrics.max_complexity,
            "branch_density_per_fn": metrics.max_complexity,
            "total_import_edges": metrics.total_import_edges,
            "cross_module_edges": metrics.total_import_edges,
            "files": metrics.file_count,
            "import_modeled_files": metrics.import_modeled_file_count,
            "functions": metrics.function_count,
        },
        "godFiles": god_file_entries(metrics),
    }))
}

fn root_cause_json(cause: sentrux_quality_signal::RootCause, raw: Value) -> Value {
    json!({
        "score": cause.score,
        "raw": raw,
        "completeness": "full",
    })
}

fn quality_signal_json(metrics: &ProjectMetrics) -> Value {
    let detail = &metrics.quality_signal_detail;
    let raw = &metrics.quality_signal_raw;
    json!({
        "schema": "code-intel-sentrux-quality-signal.v1",
        "formula_version": sentrux_quality_signal::FORMULA_VERSION,
        "provider_version": sentrux_quality_signal::PROVIDER_VERSION,
        "score": detail.quality_signal,
        "bottleneck": detail.bottleneck,
        // Worst of the five root causes' own completeness. Only
        // `redundancy` is ever less than "full" today (see
        // `sentrux_quality_signal.rs` module doc: dead-function detection
        // is not implemented, so redundancy is duplicate-only).
        "completeness": "partial",
        "root_causes": {
            "modularity": root_cause_json(detail.modularity, json!(round2(raw.modularity_q))),
            "acyclicity": root_cause_json(detail.acyclicity, json!(raw.cycle_count)),
            "depth": root_cause_json(detail.depth, json!(raw.max_depth)),
            "equality": {
                "score": detail.equality.score,
                "raw": round2(raw.equality_gini),
                "completeness": "full",
                "basis": "file_loc_fallback",
            },
            "redundancy": {
                "score": detail.redundancy.score,
                "raw": round2(raw.redundancy_ratio),
                "completeness": "partial",
                "note": "dead-function detection not implemented; ratio reflects duplicate-file redundancy only",
            },
        },
    })
}

fn metrics_json(repo: &Path, metrics: &ProjectMetrics) -> Value {
    json!({
        "tool": ENGINE_ID,
        "engine": {"id": ENGINE_ID, "version": ENGINE_VERSION},
        "path": repo.display().to_string(),
        "quality_signal": metrics.quality_signal,
        "quality_signal_detail": quality_signal_json(metrics),
        "files": metrics.file_count,
        "import_modeled_files": metrics.import_modeled_file_count,
        "functions": metrics.function_count,
        "coupling_score": metrics.coupling_score,
        "cycle_count": metrics.cycle_count,
        "cycles": metrics.cycles,
        "god_file_count": metrics.god_file_count,
        "complex_fn_count": metrics.complex_fn_count,
        "max_complexity": metrics.max_complexity,
        // Same number, honest name. `max_complexity` is what the sentrux-lite
        // scale called it and what `max_cc` gates, but the value is branch
        // tokens divided by recognised functions in one file — a per-file
        // density, not any function's cyclomatic complexity. A true max needs
        // per-function body ranges, which no scanner in this repository has.
        "branch_density_per_fn": metrics.max_complexity,
        "total_import_edges": metrics.total_import_edges,
        "cross_module_edges": metrics.total_import_edges,
        "call_edges": metrics.call_edges,
        // `unresolved_imports` was a constant `0` here (and, before that, in
        // the sentrux-lite shim this engine replaced --
        // `legacy/tools/sentrux-shim/sentrux-lite-core.ps1:248` has the exact
        // same literal). No import-resolution pass has ever populated it
        // anywhere in this repository's history (issue #375). `null` says
        // "not computed" instead of the previous `0`, which read as "computed
        // and found zero unresolved imports" to any consumer that did not
        // know better -- `evidence.sentrux`/`diagnosis.hospital`/`report`/
        // `pr_gate`/`release_gate` all read this field via
        // `capability_structured_data`. A real value needs an actual
        // import-resolution pass; building one here would duplicate #297's
        // separate (Sentrux-excluded) effort, so real implementation is
        // scoped to a follow-up issue instead. See DR-0009.
        "unresolved_imports": null,
        "unresolved_imports_status": "not_implemented",
    })
}

fn resolve_header(out: &mut String, metrics: &ProjectMetrics) {
    out.push_str(&format!(
        "[resolve] {} resolved, 0 unresolved\n",
        metrics.total_import_edges
    ));
    out.push_str(&format!(
        "[build_graphs] {} files | {} import, {} call, 0 inherit edges\n",
        metrics.file_count, metrics.total_import_edges, metrics.call_edges
    ));
    // Coupling divides by this count, not by the file count above, so print
    // the basis rather than leaving the reader to derive it from the ratio.
    out.push_str(&format!(
        "[coupling_basis] {} of {} files in import-modelled languages\n",
        metrics.import_modeled_file_count, metrics.file_count
    ));
}

fn gate_pairs(out: &mut String, before: &ProjectMetrics, after: &ProjectMetrics) {
    out.push_str(&format!(
        "Quality: {} -> {}\n",
        before.quality_signal, after.quality_signal
    ));
    out.push_str(&format!(
        "Coupling: {} -> {}\n",
        before.coupling_score, after.coupling_score
    ));
    out.push_str(&format!(
        "Cycles: {} -> {}\n",
        before.cycle_count, after.cycle_count
    ));
    out.push_str(&format!(
        "God files: {} -> {}\n",
        before.god_file_count, after.god_file_count
    ));
}

fn gate_pairs_values(out: &mut String, before: &Value, after: &ProjectMetrics) {
    out.push_str(&format!(
        "Quality: {} -> {}\n",
        number(before, "quality_signal"),
        after.quality_signal
    ));
    out.push_str(&format!(
        "Coupling: {} -> {}\n",
        number(before, "coupling_score"),
        after.coupling_score
    ));
    out.push_str(&format!(
        "Cycles: {} -> {}\n",
        number(before, "cycle_count"),
        after.cycle_count
    ));
    out.push_str(&format!(
        "God files: {} -> {}\n",
        number(before, "god_file_count"),
        after.god_file_count
    ));
}

fn evaluate_rules(rules: &str, metrics: &ProjectMetrics, repo: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    if let Some(limit) = integer_rule(rules, "max_cc") {
        if metrics.max_complexity > limit {
            violations.push(Violation {
                rule: "max_cc".into(),
                message: format!("max_cc exceeded: {} > {limit}", metrics.max_complexity),
                targets: complexity_targets(metrics),
            });
        }
    }
    if boolean_rule(rules, "no_god_files") && metrics.god_file_count > 0 {
        violations.push(Violation {
            rule: "no_god_files".into(),
            message: format!("god files detected: {}", metrics.god_file_count),
            targets: god_file_targets(metrics),
        });
    }
    if let Some(limit) = integer_rule(rules, "max_cycles") {
        if metrics.cycle_count > limit {
            violations.push(Violation {
                rule: "max_cycles".into(),
                message: format!("cycles exceeded: {} > {limit}", metrics.cycle_count),
                targets: cycle_targets(metrics),
            });
        }
    }
    if let Some(configured) = string_rule(rules, "max_coupling") {
        // The A..D ladder tops out at 6 imports per file, which no idiomatic
        // Rust tree stays under, so the rule also accepts a bare number. A
        // repository that has outgrown the ladder records the honest measured
        // ceiling instead of dropping the absolute check entirely.
        let limit = coupling_limit(&configured);
        if let Some(limit) = limit {
            if metrics.coupling_score > limit {
                violations.push(Violation {
                    rule: "max_coupling".into(),
                    message: format!(
                        "coupling limit exceeded: {} > {limit} for {configured}",
                        metrics.coupling_score
                    ),
                    targets: top_import_files(metrics),
                });
            }
        }
    }
    let boundaries = boundary_rules::parse_boundaries(rules);
    let layers = boundary_rules::parse_layers(rules);
    if !boundaries.is_empty() || !layers.is_empty() {
        let rust_files: BTreeSet<String> = metrics
            .files
            .iter()
            .filter(|file| file.path.ends_with(".rs"))
            .map(|file| file.path.clone())
            .collect();
        let edges = boundary_rules::crate_edges(repo, &rust_files);
        let modules_by_file: BTreeMap<String, String> = rust_files
            .iter()
            .filter_map(|path| {
                boundary_rules::owning_module(path, &rust_files)
                    .map(|module| (path.clone(), module))
            })
            .collect();
        violations.extend(boundary_rules::boundary_violations(
            &boundaries,
            &edges,
            &modules_by_file,
        ));
        violations.extend(boundary_rules::layer_violations(
            &layers,
            &edges,
            &modules_by_file,
        ));
    }
    violations
}

fn complexity_targets(metrics: &ProjectMetrics) -> Vec<String> {
    let mut files = metrics
        .files
        .iter()
        .filter(|file| file.max_complexity == metrics.max_complexity)
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    files.sort();
    files.truncate(MAX_VIOLATION_TARGETS);
    files
}

fn god_file_targets(metrics: &ProjectMetrics) -> Vec<String> {
    let mut files = metrics
        .files
        .iter()
        .filter(|file| file.god_file)
        .map(|file| (file.loc, file.path.clone()))
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    files
        .into_iter()
        .take(MAX_VIOLATION_TARGETS)
        .map(|(_, path)| path)
        .collect()
}

/// Which branch of the god-file rule a file trips — gate output names the
/// rule and the measured values, never just a count (#165, #148 C1).
fn god_rule_label(loc: i64, functions: i64) -> String {
    let mut parts = Vec::new();
    if loc > GOD_FILE_LOC_LIMIT {
        parts.push(format!("loc>{GOD_FILE_LOC_LIMIT}"));
    }
    if functions > GOD_FILE_FN_LIMIT && loc > GOD_FILE_FN_LOC_LIMIT {
        parts.push(format!(
            "functions>{GOD_FILE_FN_LIMIT}&&loc>{GOD_FILE_FN_LOC_LIMIT}"
        ));
    }
    parts.join("; ")
}

/// Per-file identity for every current god file, recorded in the baseline so
/// the ratchet can tell a brand-new god file from tolerated standing debt.
/// Sorted by path so a saved baseline is deterministic for the same tree.
fn god_file_entries(metrics: &ProjectMetrics) -> Vec<Value> {
    let mut files: Vec<&FileMetrics> = metrics.files.iter().filter(|file| file.god_file).collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
        .into_iter()
        .map(|file| {
            json!({
                "path": file.path,
                "loc": file.loc,
                "functions": file.functions,
                "rule": god_rule_label(file.loc, file.functions),
            })
        })
        .collect()
}

fn top_import_files(metrics: &ProjectMetrics) -> Vec<String> {
    let mut files = metrics
        .files
        .iter()
        .filter(|file| file.import_modeled)
        .map(|file| (file.imports, file.path.clone()))
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    files.into_iter().take(3).map(|(_, path)| path).collect()
}

fn cycle_targets(metrics: &ProjectMetrics) -> Vec<String> {
    let mut targets = metrics.cycles.iter().flatten().cloned().collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets.truncate(MAX_VIOLATION_TARGETS);
    targets
}

fn measure_project(repo: &Path) -> Result<(ProjectMetrics, file_gate::GateReport), String> {
    let repo = repo
        .canonicalize()
        .map_err(|error| format!("resolve repository path: {error}"))?;
    let config = file_gate::GateConfig::built_in();
    let report = file_gate::evaluate(&repo, &config)?;
    let paths = report.included.clone();
    let known_paths: BTreeSet<String> = paths.iter().cloned().collect();
    let mut files = Vec::new();
    // Raw import-target candidates (from, raw target text) and each file's
    // duplicate-check normalized content, gathered in the same pass that
    // already reads every file's content -- feeds `sentrux_quality_signal`'s
    // modularity/acyclicity/depth/redundancy raw inputs without a second
    // filesystem read.
    let mut edge_candidates: Vec<(String, String)> = Vec::new();
    let mut normalized_content: BTreeMap<String, String> = BTreeMap::new();
    for relative in &paths {
        let content = fs::read(repo.join(relative)).unwrap_or_default();
        let content = String::from_utf8_lossy(&content);
        for line in content.lines() {
            let trimmed = line.trim_start();
            if is_import_line(trimmed) {
                if let Some(candidate) = sentrux_quality_signal::import_target_candidate(trimmed) {
                    edge_candidates.push((relative.clone(), candidate));
                }
            }
        }
        let normalized =
            sentrux_quality_signal::normalize_for_duplicate_check(&strip_comments_and_strings(
                relative, &content,
            ));
        normalized_content.insert(relative.clone(), normalized);
        files.push(measure_file(relative, &content));
    }
    let file_count = files.len() as i64;
    let import_modeled_file_count = files.iter().filter(|file| file.import_modeled).count() as i64;
    let function_count = files.iter().map(|file| file.functions).sum();
    let total_import_edges: i64 = files
        .iter()
        .filter(|file| file.import_modeled)
        .map(|file| file.imports)
        .sum();
    let call_edges = files.iter().map(|file| file.calls).sum();
    let god_file_count = files.iter().filter(|file| file.god_file).count() as i64;
    let complex_fn_count = files.iter().filter(|file| file.complex_file).count() as i64;
    let max_complexity = files
        .iter()
        .map(|file| file.max_complexity)
        .max()
        .unwrap_or(0);
    let coupling_score = if import_modeled_file_count > 0 {
        round2(total_import_edges as f64 / import_modeled_file_count as f64 * 10.0)
    } else {
        0.0
    };
    let cycles = rust_import_cycles(&repo, &paths);

    // ── Quality Signal (issue #385): resolve the multi-language file
    // dependency graph, then feed the five root-cause computations. ──
    let quality_signal_edges: BTreeSet<(String, String)> = edge_candidates
        .iter()
        .filter_map(|(from, raw_target)| {
            sentrux_quality_signal::resolve_edge(from, raw_target, &known_paths)
                .filter(|to| to != from)
                .map(|to| (from.clone(), to))
        })
        .collect();
    let mut qs_adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, to) in &quality_signal_edges {
        qs_adjacency.entry(from.clone()).or_default().insert(to.clone());
    }
    let qs_cycles = strongly_connected_cycles(&qs_adjacency);
    let (qs_cycle_count, qs_max_depth) =
        sentrux_quality_signal::compute_cycles_and_depth(&quality_signal_edges, &qs_cycles);
    let modularity_q = sentrux_quality_signal::compute_modularity_q(&quality_signal_edges);
    let file_loc: Vec<f64> = files.iter().map(|file| file.loc as f64).collect();
    let equality_gini = sentrux_quality_signal::gini(&file_loc);

    // Duplicate-file groups (redundancy's `duplicate` half — see
    // `sentrux_quality_signal.rs` module doc for why `dead` is absent
    // rather than fabricated).
    let functions_by_path: BTreeMap<&str, i64> = files
        .iter()
        .map(|file| (file.path.as_str(), file.functions))
        .collect();
    let mut duplicate_groups: BTreeMap<&String, Vec<&String>> = BTreeMap::new();
    for (path, normalized) in &normalized_content {
        if normalized.len() >= sentrux_quality_signal::MIN_DUPLICATE_CONTENT_LEN {
            duplicate_groups.entry(normalized).or_default().push(path);
        }
    }
    let duplicate_function_total: i64 = duplicate_groups
        .values()
        .filter(|members| members.len() >= 2)
        .flat_map(|members| members.iter())
        .filter_map(|path| functions_by_path.get(path.as_str()).copied())
        .sum();
    let redundancy_ratio =
        sentrux_quality_signal::redundancy_ratio(duplicate_function_total, function_count);

    let quality_signal_raw = sentrux_quality_signal::RootCauseRaw {
        modularity_q,
        cycle_count: qs_cycle_count,
        max_depth: qs_max_depth,
        equality_gini,
        redundancy_ratio,
    };
    let quality_signal_detail =
        sentrux_quality_signal::normalize_and_aggregate(&quality_signal_raw);
    let quality_signal = quality_signal_detail.quality_signal;

    let metrics = ProjectMetrics {
        cycle_count: cycles.len() as i64,
        cycles,
        files,
        file_count,
        import_modeled_file_count,
        function_count,
        total_import_edges,
        call_edges,
        god_file_count,
        complex_fn_count,
        max_complexity,
        coupling_score,
        quality_signal,
        quality_signal_detail,
        quality_signal_raw,
    };
    Ok((metrics, report))
}

fn measure_file(relative: &str, content: &str) -> FileMetrics {
    let mut loc = 0i64;
    let mut functions = 0i64;
    let mut imports = 0i64;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        loc += 1;
        if is_function_line(trimmed) {
            functions += 1;
        }
        if is_import_line(trimmed) {
            imports += 1;
        }
    }
    let complexity_terms = complexity_terms(relative, content);
    let max_complexity = if functions > 0 {
        ((complexity_terms as f64 / functions as f64).ceil() as i64 + 1).max(1)
    } else if complexity_terms > 0 {
        complexity_terms + 1
    } else {
        1
    };
    let calls = call_sites(content);
    FileMetrics {
        path: relative.to_string(),
        loc,
        functions,
        max_complexity,
        imports,
        calls,
        god_file: loc > GOD_FILE_LOC_LIMIT
            || (functions > GOD_FILE_FN_LIMIT && loc > GOD_FILE_FN_LOC_LIMIT),
        complex_file: max_complexity > 25,
        import_modeled: imports_modeled(relative),
    }
}

fn imports_modeled(relative: &str) -> bool {
    let extension = relative
        .rsplit('/')
        .next()
        .and_then(|leaf| leaf.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    IMPORT_MODELED_EXTENSIONS.contains(&extension.as_str())
}

fn is_function_line(trimmed: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "function ",
        "def ",
        "fn ",
        "pub fn ",
        "pub(crate) fn ",
        "pub(super) fn ",
        "func ",
        "class ",
        "interface ",
        "export function ",
    ];
    if PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix)) {
        return true;
    }
    if trimmed.starts_with("const ") && trimmed.contains('=') {
        let after = trimmed.split_once('=').map(|(_, rest)| rest.trim_start());
        if let Some(after) = after {
            if after.starts_with('(') || after.starts_with("async ") || after.starts_with("async(")
            {
                return true;
            }
        }
    }
    if trimmed.starts_with("public ") && trimmed.contains('(') {
        return true;
    }
    false
}

fn is_import_line(trimmed: &str) -> bool {
    trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("mod ")
        || trimmed.starts_with("require(")
        || trimmed.starts_with("#include ")
        || trimmed.starts_with("using ")
}

/// Branch-ish tokens per file, the numerator of the complexity metric.
///
/// Two things this deliberately does that the first version did not.
///
/// **Prose does not branch.** The original tokenised the raw file, so every
/// `if`, `or` and `and` inside a comment or a string literal counted. That is
/// how `.sentrux/rules.toml` ended up ratcheted at `max_cc = 162`: the number
/// came from `legacy/scripts/tests/test-code-intel-pipeline.ps1`, 465 lines
/// with one recognised function, whose 161 "terms" included 45 bare `or` and
/// 11 bare `and` — English words in comments, since PowerShell spells the
/// operators `-or` and `-and`. Comments and string bodies are stripped first.
///
/// **`and`/`or` are only operators in some languages.** They branch in Python;
/// in PowerShell, Rust, C-family and JS they are ordinary words, and the
/// operators are `-and`/`-or` or `&&`/`||`. Counting by extension stops one
/// language's prose from inflating another language's threshold.
fn complexity_terms(relative: &str, content: &str) -> i64 {
    const WORDS: &[&str] = &[
        "if", "elif", "elseif", "for", "foreach", "while", "switch", "case", "catch", "except",
        "match",
    ];
    const WORD_OPERATOR_LANGUAGES: &[&str] = &["py"];

    let stripped = strip_comments_and_strings(relative, content);
    let word_operators = WORD_OPERATOR_LANGUAGES.contains(&extension_of(relative).as_str());
    let mut count = 0i64;
    for token in stripped.split(|character: char| !character.is_alphanumeric() && character != '_')
    {
        if WORDS.contains(&token) || (word_operators && (token == "and" || token == "or")) {
            count += 1;
        }
    }
    let bytes = stripped.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        let window = &bytes[index..index + 2];
        if window == b"&&" || window == b"||" {
            count += 1;
            // Skip the pair so `&&&&` reads as two operators, not three.
            index += 2;
            continue;
        }
        if window == b"-a" || window == b"-o" {
            // PowerShell's `-and` / `-or`.
            let rest = &stripped[index..];
            if rest.starts_with("-and") || rest.starts_with("-or") {
                count += 1;
            }
        }
        index += 1;
    }
    count
}

fn extension_of(relative: &str) -> String {
    relative
        .rsplit('/')
        .next()
        .and_then(|leaf| leaf.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Replaces comment and string-literal bodies with spaces, preserving byte
/// offsets and line structure so every other measurement is unaffected.
///
/// This is a scanner, not a parser: it knows line comments, block comments and
/// single/double/backtick quoted runs, and it does not attempt heredocs,
/// nested block comments, or regex literals. That is enough to stop prose from
/// being counted as control flow, which is the whole point; a file that
/// defeats it degrades to the old behaviour rather than to a wrong parse.
fn strip_comments_and_strings(relative: &str, content: &str) -> String {
    let hash_comments = !matches!(
        extension_of(relative).as_str(),
        "rs" | "go"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "java"
            | "cs"
            | "cpp"
            | "c"
            | "h"
            | "hpp"
            | "v"
    );
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut quote: Option<char> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    while let Some(character) = chars.next() {
        if in_line_comment {
            if character == '\n' {
                in_line_comment = false;
                out.push('\n');
            } else {
                out.push(' ');
            }
            continue;
        }
        if in_block_comment {
            if character == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
                out.push_str("  ");
            } else {
                out.push(if character == '\n' { '\n' } else { ' ' });
            }
            continue;
        }
        if let Some(open) = quote {
            if character == '\\' {
                chars.next();
                out.push_str("  ");
                continue;
            }
            if character == open {
                quote = None;
            }
            out.push(if character == '\n' { '\n' } else { ' ' });
            continue;
        }
        match character {
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                in_line_comment = true;
                out.push_str("  ");
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_block_comment = true;
                out.push_str("  ");
            }
            '#' if hash_comments => {
                in_line_comment = true;
                out.push(' ');
            }
            '"' | '\'' | '`' => {
                quote = Some(character);
                out.push(' ');
            }
            other => out.push(other),
        }
    }
    out
}

fn call_sites(content: &str) -> i64 {
    let bytes = content.as_bytes();
    let mut count = 0i64;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'(' {
            continue;
        }
        let mut cursor = index;
        while cursor > 0 && bytes[cursor - 1] == b' ' {
            cursor -= 1;
        }
        if cursor > 0 {
            let previous = bytes[cursor - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' {
                count += 1;
            }
        }
    }
    count
}

/// Cycles between Rust source files, resolved from `use crate::segment`
/// statements within each crate `src/` tree. sentrux-lite reported a constant
/// zero here; this graph is deliberately conservative (first path segment
/// only) so every reported cycle is a real module dependency cycle.
fn rust_import_cycles(repo: &Path, paths: &[String]) -> Vec<Vec<String>> {
    let rust_files = paths
        .iter()
        .filter(|path| path.ends_with(".rs"))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in &rust_files {
        let Some(source_root) = crate_source_root(path) else {
            continue;
        };
        let content = fs::read(repo.join(path)).unwrap_or_default();
        let content = String::from_utf8_lossy(&content);
        for segment in crate_use_segments(&content) {
            let module_file = format!("{source_root}/{segment}.rs");
            let module_directory = format!("{source_root}/{segment}/mod.rs");
            let target = if rust_files.contains(&module_file) {
                module_file
            } else if rust_files.contains(&module_directory) {
                module_directory
            } else {
                continue;
            };
            if &target != path {
                edges.entry(path.clone()).or_default().insert(target);
            }
        }
    }
    strongly_connected_cycles(&edges)
}

fn crate_source_root(path: &str) -> Option<String> {
    let mut segments = path.split('/').collect::<Vec<_>>();
    segments.pop()?;
    while !segments.is_empty() {
        if segments.last() == Some(&"src") {
            return Some(segments.join("/"));
        }
        segments.pop();
    }
    None
}

fn crate_use_segments(content: &str) -> BTreeSet<String> {
    let mut segments = BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("use crate::")
            .or_else(|| trimmed.strip_prefix("pub use crate::"))
            .or_else(|| trimmed.strip_prefix("pub(crate) use crate::"))
        else {
            continue;
        };
        if let Some(list) = rest.strip_prefix('{') {
            for entry in list.trim_end_matches(|c| matches!(c, '}' | ';')).split(',') {
                if let Some(segment) = first_segment(entry.trim()) {
                    segments.insert(segment);
                }
            }
        } else if let Some(segment) = first_segment(rest) {
            segments.insert(segment);
        }
    }
    segments
}

fn first_segment(text: &str) -> Option<String> {
    let segment = text
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .next()?
        .to_string();
    (!segment.is_empty() && segment != "self" && segment != "super").then_some(segment)
}

fn strongly_connected_cycles(edges: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    let mut nodes = BTreeSet::new();
    for (source, targets) in edges {
        nodes.insert(source.clone());
        nodes.extend(targets.iter().cloned());
    }
    let nodes = nodes.into_iter().collect::<Vec<_>>();
    let index_of = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let adjacency = nodes
        .iter()
        .map(|node| {
            edges
                .get(node)
                .map(|targets| {
                    targets
                        .iter()
                        .filter_map(|target| index_of.get(target).copied())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();

    struct Tarjan<'graph> {
        adjacency: &'graph [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next_index: usize,
        components: Vec<Vec<usize>>,
    }

    impl Tarjan<'_> {
        fn visit(&mut self, node: usize) {
            self.index[node] = Some(self.next_index);
            self.low[node] = self.next_index;
            self.next_index += 1;
            self.stack.push(node);
            self.on_stack[node] = true;
            for target in self.adjacency[node].clone() {
                if self.index[target].is_none() {
                    self.visit(target);
                    self.low[node] = self.low[node].min(self.low[target]);
                } else if self.on_stack[target] {
                    self.low[node] = self.low[node].min(self.index[target].expect("indexed"));
                }
            }
            if Some(self.low[node]) == self.index[node] {
                let mut component = Vec::new();
                while let Some(member) = self.stack.pop() {
                    self.on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                self.components.push(component);
            }
        }
    }

    let mut tarjan = Tarjan {
        adjacency: &adjacency,
        index: vec![None; nodes.len()],
        low: vec![0; nodes.len()],
        on_stack: vec![false; nodes.len()],
        stack: Vec::new(),
        next_index: 0,
        components: Vec::new(),
    };
    for node in 0..nodes.len() {
        if tarjan.index[node].is_none() {
            tarjan.visit(node);
        }
    }
    let mut cycles = tarjan
        .components
        .into_iter()
        .filter(|component| component.len() > 1)
        .map(|component| {
            let mut members = component
                .into_iter()
                .map(|index| nodes[index].clone())
                .collect::<Vec<_>>();
            members.sort();
            members
        })
        .collect::<Vec<_>>();
    cycles.sort();
    cycles
}

/// Absolute coupling ceiling from a `max_coupling` value: a letter grade on
/// the inherited sentrux-lite ladder, or a number on the same scale. An
/// unreadable value yields `None`, which leaves the absolute check disabled —
/// the no-degradation gate against the baseline still runs.
fn coupling_limit(configured: &str) -> Option<f64> {
    match configured.trim() {
        "A" => Some(5.0),
        "B" => Some(15.0),
        "C" => Some(30.0),
        "D" => Some(60.0),
        other => other.parse::<f64>().ok().filter(|limit| *limit >= 0.0),
    }
}

fn integer_rule(rules: &str, name: &str) -> Option<i64> {
    rule_value(rules, name)?.trim().parse().ok()
}

fn boolean_rule(rules: &str, name: &str) -> bool {
    rule_value(rules, name)
        .map(|value| value.trim() == "true")
        .unwrap_or(false)
}

fn string_rule(rules: &str, name: &str) -> Option<String> {
    let value = rule_value(rules, name)?;
    Some(value.trim().trim_matches('"').to_string())
}

fn rule_value(rules: &str, name: &str) -> Option<String> {
    for line in rules.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(name) {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.split('#').next().unwrap_or("");
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn git_head(repo: &Path) -> String {
    hardened_git::command(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| !head.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or(0.0)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture");
        root
    }

    #[test]
    fn crate_use_segments_cover_plain_grouped_and_visibility_forms() {
        let content = "use crate::alpha::Beta;\npub(crate) use crate::gamma;\nuse crate::{delta, epsilon::Zeta};\nuse std::fs;\n";
        let segments = crate_use_segments(content);
        assert_eq!(
            segments.into_iter().collect::<Vec<_>>(),
            vec!["alpha", "delta", "epsilon", "gamma"]
        );
    }

    #[test]
    fn two_file_mutual_imports_form_one_cycle() {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        edges
            .entry("src/a.rs".into())
            .or_default()
            .insert("src/b.rs".into());
        edges
            .entry("src/b.rs".into())
            .or_default()
            .insert("src/a.rs".into());
        edges
            .entry("src/c.rs".into())
            .or_default()
            .insert("src/a.rs".into());
        let cycles = strongly_connected_cycles(&edges);
        assert_eq!(
            cycles,
            vec![vec!["src/a.rs".to_string(), "src/b.rs".into()]]
        );
    }

    #[test]
    #[should_panic(expected = "this is NOT a rule-violation failure")]
    fn expect_check_ran_reports_engine_failure_distinctly_from_a_violation() {
        // #192 regression: an engine that could not run (here, a repository
        // path that cannot resolve) must panic with wording that sends the
        // reader to the underlying error, never with the shape of the
        // `resolved Rust import cycle(s) detected` business assertion below.
        let missing = std::env::temp_dir().join(format!(
            "code-intel-192-missing-repo-{}",
            std::process::id()
        ));
        expect_check_ran(&missing);
    }

    #[test]
    #[should_panic(expected = "this is NOT a rule-violation failure")]
    fn expect_gate_ran_reports_engine_failure_distinctly_from_a_violation() {
        let missing = std::env::temp_dir().join(format!(
            "code-intel-192-missing-repo-gate-{}",
            std::process::id()
        ));
        expect_gate_ran(&missing, false);
    }

    #[test]
    fn this_repository_has_no_resolved_import_cycles() {
        // Regression guard for the `dag_run <-> execution_kernel` cycle (and
        // the `artifact_ref <-> capability` cycle found alongside it)
        // removed to reach the absolute `max_cycles = 0` rule recorded in
        // `.sentrux/rules.toml`. This calls the exact engine
        // `code-intel sentrux --operation check` and the CI/release
        // self-scan both run, directly against this crate's own repository
        // — the same mechanism, not a new one — so a reintroduced cycle
        // between any two source files fails a plain `cargo test` in
        // seconds instead of only surfacing later in a full self-scan
        // build.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let run = expect_check_ran(&repo);
        let cycle_violations: Vec<&Violation> = run
            .violations
            .iter()
            .filter(|violation| {
                violation.rule == "max_cycles" || violation.rule == "cycles_increased"
            })
            .collect();
        assert!(
            cycle_violations.is_empty(),
            "resolved Rust import cycle(s) detected: {cycle_violations:?}"
        );
    }

    #[test]
    fn every_code_extension_is_classified_as_import_modeled_or_not() {
        // A new extension added to file_gate::CODE_EXTENSIONS without a
        // decision here would silently land in the coupling denominator
        // with zero imports, which is the exact defect this split exists
        // to prevent.
        for extension in file_gate::CODE_EXTENSIONS {
            let modeled = IMPORT_MODELED_EXTENSIONS.contains(extension);
            let unmodeled = IMPORT_UNMODELED_EXTENSIONS.contains(extension);
            assert!(
                modeled ^ unmodeled,
                "{extension} must appear in exactly one of IMPORT_MODELED_EXTENSIONS / IMPORT_UNMODELED_EXTENSIONS"
            );
        }
        for extension in IMPORT_MODELED_EXTENSIONS
            .iter()
            .chain(IMPORT_UNMODELED_EXTENSIONS)
        {
            assert!(
                file_gate::CODE_EXTENSIONS.contains(extension),
                "{extension} is classified but never collected"
            );
        }
    }

    #[test]
    fn coupling_ignores_files_whose_language_has_no_modeled_imports() {
        let root = fixture_root("sentrux-native-coupling");
        fs::write(
            root.join("lib.rs"),
            "use std::fs;\nuse std::io;\npub fn f() {}\n",
        )
        .expect("write rust fixture");
        let (rust_only, _) = measure_project(&root).expect("measure rust-only tree");
        assert_eq!(rust_only.import_modeled_file_count, 1);
        assert_eq!(rust_only.coupling_score, 20.0);

        // Ten PowerShell files, each carrying a real dot-source dependency
        // this scanner cannot read. Under the old all-files denominator the
        // score would fall from 20.0 to 1.82 purely for gaining PowerShell.
        for index in 0..10 {
            fs::write(
                root.join(format!("helper{index}.ps1")),
                ". $PSScriptRoot/other.ps1\nfunction Invoke-Thing {}\n",
            )
            .expect("write powershell fixture");
        }
        let (mixed, _) = measure_project(&root).expect("measure mixed tree");
        assert_eq!(mixed.file_count, 11);
        assert_eq!(mixed.import_modeled_file_count, 1);
        assert_eq!(mixed.coupling_score, rust_only.coupling_score);

        // Deleting PowerShell — the whole point of the migration — must not
        // register as a coupling regression either.
        fs::remove_file(root.join("helper0.ps1")).expect("remove one powershell fixture");
        let (after_deletion, _) = measure_project(&root).expect("measure tree after deletion");
        assert_eq!(after_deletion.coupling_score, mixed.coupling_score);
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn scan_json_reports_unresolved_imports_as_unmeasured_not_zero() {
        // #375: `unresolved_imports` was a hardcoded `0`, which reads as "an
        // import-resolution pass ran and found nothing unresolved" to any
        // consumer of `evidence.sentrux`/`diagnosis.hospital`/`report`/
        // `pr_gate`/`release_gate`. No such pass exists in this engine (or
        // ever existed in the sentrux-lite shim it replaced), so the field
        // must say "not computed", not fabricate a count.
        let root = fixture_root("sentrux-native-scan-unresolved-imports");
        fs::write(root.join("lib.rs"), "use std::fs;\npub fn f() {}\n")
            .expect("write rust fixture");
        let value = scan_json(&root).expect("scan_json on fixture");
        assert_eq!(value["unresolved_imports"], Value::Null);
        assert_eq!(value["unresolved_imports_status"], "not_implemented");
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn prose_in_comments_and_strings_does_not_count_as_branching() {
        // The shape that produced this repository's max_cc = 162: PowerShell
        // spells its operators -and / -or, so bare "and"/"or" in a comment is
        // English, not control flow.
        let powershell = "# choose one branch or another, this and that\n$message = \"if or and while\"\nif ($x) { }\n";
        assert_eq!(complexity_terms("script.ps1", powershell), 1);

        // Real PowerShell operators still count.
        assert_eq!(
            complexity_terms("script.ps1", "if ($a -and $b -or $c) { }\n"),
            3
        );

        // Python is the language where the words really are operators.
        assert_eq!(complexity_terms("m.py", "if a and b or c:\n    pass\n"), 3);
        assert_eq!(
            complexity_terms("m.py", "# and or if\nvalue = 'and or if'\n"),
            0
        );

        // C-family block and line comments, and && / || pairs.
        assert_eq!(
            complexity_terms("m.rs", "/* if while */\n// for match\nif a && b || c { }\n"),
            3
        );
    }

    #[test]
    fn stripping_preserves_line_structure_so_other_metrics_are_unaffected() {
        let source = "let a = \"x\";\n// comment\nlet b = 1;\n";
        let stripped = strip_comments_and_strings("m.rs", source);
        assert_eq!(stripped.lines().count(), source.lines().count());
        assert_eq!(stripped.len(), source.len());
        assert!(!stripped.contains("comment"));
    }

    #[test]
    fn coupling_limit_reads_ladder_grades_and_bare_numbers() {
        assert_eq!(coupling_limit("D"), Some(60.0));
        assert_eq!(coupling_limit("76.0"), Some(76.0));
        assert_eq!(coupling_limit(" 76 "), Some(76.0));
        assert_eq!(coupling_limit("E"), None);
        assert_eq!(coupling_limit("-1"), None);
    }

    #[test]
    fn rule_parsing_reads_integer_boolean_and_grade_values() {
        let rules = "[constraints]\nmax_cycles = 0\nmax_coupling = \"B\"\nmax_cc = 70 # comment\nno_god_files = true\n";
        assert_eq!(integer_rule(rules, "max_cycles"), Some(0));
        assert_eq!(integer_rule(rules, "max_cc"), Some(70));
        assert!(boolean_rule(rules, "no_god_files"));
        assert_eq!(string_rule(rules, "max_coupling").as_deref(), Some("B"));
    }

    #[test]
    fn gate_rejects_legacy_baseline_without_engine_identity() {
        let root = fixture_root("sentrux-native-gate");
        fs::create_dir_all(root.join(".sentrux")).expect("create fixture");
        fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").expect("write fixture source");
        fs::write(
            root.join(".sentrux/baseline.json"),
            b"{\"timestamp\":1.0,\"quality_signal\":0.5}",
        )
        .expect("write legacy baseline");
        let run = expect_gate_ran(&root, false);
        assert!(!run.success);
        assert_eq!(run.violations[0].rule, "baseline_engine_mismatch");
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn gate_rejects_current_schema_baseline_missing_a_gated_metric() {
        let root = fixture_root("sentrux-native-metrics");
        fs::create_dir_all(root.join(".sentrux")).expect("create fixture");
        fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").expect("write fixture source");
        // Correct schema and engine but no quality_signal: `number()` would
        // read the absent key as 0.0 and silently disable the quality gate.
        fs::write(
            root.join(".sentrux/baseline.json"),
            format!(
                "{{\"schema\":\"{BASELINE_SCHEMA}\",\"engine\":{{\"id\":\"{ENGINE_ID}\",\"version\":\"{ENGINE_VERSION}\"}},\"metrics\":{{\"coupling_score\":0.0,\"cycle_count\":0,\"god_file_count\":0}}}}"
            ),
        )
        .expect("write baseline");
        let run = expect_gate_ran(&root, false);
        assert!(!run.success);
        assert_eq!(run.violations[0].rule, "baseline_engine_mismatch");
        assert!(
            run.violations[0].message.contains("quality_signal"),
            "{}",
            run.violations[0].message
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn save_then_gate_is_clean_and_check_reports_targets() {
        let root = std::env::temp_dir().join(format!(
            "sentrux-native-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(
            root.join("src/a.rs"),
            "use crate::b;\npub fn a() { b::b(); }\n",
        )
        .expect("write a.rs");
        fs::write(
            root.join("src/b.rs"),
            "use crate::a;\npub fn b() { let _ = crate::a::a; }\n",
        )
        .expect("write b.rs");
        fs::write(root.join("Cargo.toml"), "[package]\n").expect("write manifest");

        let saved = expect_gate_ran(&root, true);
        assert!(saved.success);
        let clean = expect_gate_ran(&root, false);
        assert!(clean.success, "stdout: {}", clean.stdout);

        fs::create_dir_all(root.join(".sentrux")).expect("sentrux dir exists");
        fs::write(
            root.join(".sentrux/rules.toml"),
            "[constraints]\nmax_cycles = 0\n",
        )
        .expect("write rules");
        let check = expect_check_ran(&root);
        assert!(!check.success);
        assert_eq!(check.violations[0].rule, "max_cycles");
        assert_eq!(
            check.violations[0].targets,
            vec!["src/a.rs".to_string(), "src/b.rs".into()]
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn check_aligned_matches_gate_verdict_when_a_new_god_file_violates_the_ratchet() {
        // Regression test for issue #106: `code-intel sentrux check` used to
        // call only `run_check` (the static `.sentrux/rules.toml` rules) and
        // never opened `.sentrux/baseline.json`, so it reported green on a
        // tree the authoritative `evidence.sentrux` DAG node -- which always
        // runs `run_gate` too, as a second independent rule (see
        // `builtin_provider_evidence::sentrux_admission`) -- correctly
        // failed. This repository's own `.sentrux/rules.toml` sets
        // `no_god_files = false` for exactly the documented reason that
        // god-file monotonicity is meant to be enforced by the baseline
        // ratchet, not the static rule, so a plain `run_check` can never
        // catch a new god file by itself.
        let root = fixture_root("sentrux-native-check-aligned");
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(root.join("src/a.rs"), "pub fn a() {}\n").expect("write a.rs");
        fs::create_dir_all(root.join(".sentrux")).expect("sentrux dir");
        fs::write(
            root.join(".sentrux/rules.toml"),
            "[constraints]\nmax_cycles = 0\nno_god_files = false\n",
        )
        .expect("write rules");

        let saved = expect_gate_ran(&root, true);
        assert!(saved.success, "stdout: {}", saved.stdout);

        // Grow src/a.rs past the god-file threshold (loc > 800) without
        // tripping any static rule: `no_god_files` is off, and nothing else
        // in this rules.toml gates file size.
        let mut big = String::from("pub fn a() {}\n");
        for line in 0..850 {
            big.push_str(&format!("// padding {line}\n"));
        }
        fs::write(root.join("src/a.rs"), &big).expect("grow a.rs past the god-file threshold");

        // Pre-fix behavior, still reachable directly: the static-only check
        // never looks at the baseline and stays green.
        let static_only = expect_check_ran(&root);
        assert!(
            static_only.success,
            "static rules alone should still pass (no_god_files is off): {}",
            static_only.stdout
        );

        // The authoritative ratchet catches it directly.
        let gate = expect_gate_ran(&root, false);
        assert!(!gate.success, "stdout: {}", gate.stdout);
        let gate_rules: BTreeSet<&str> = gate.violations.iter().map(|v| v.rule.as_str()).collect();
        assert!(gate_rules.contains("god_files_increased"), "{gate_rules:?}");

        // The aligned `check` operation must reach the same verdict as the
        // authoritative gate for this tree -- the parity issue #106 asks for.
        let aligned = run_check_aligned(&root, true).expect("aligned check runs");
        assert!(
            !aligned.success,
            "aligned check must fail when the baseline ratchet fails: {}",
            aligned.stdout
        );
        let aligned_rules: BTreeSet<&str> =
            aligned.violations.iter().map(|v| v.rule.as_str()).collect();
        assert_eq!(
            aligned_rules, gate_rules,
            "check and the authoritative gate must fail on the same rules"
        );

        // The explicit opt-out reverts to the static-only verdict and says so.
        let no_ratchet = run_check_aligned(&root, false).expect("no-ratchet check runs");
        assert!(
            no_ratchet.success,
            "explicit --no-ratchet should skip the baseline comparison: {}",
            no_ratchet.stdout
        );
        assert!(
            no_ratchet.stdout.to_lowercase().contains("skipped"),
            "no-ratchet output must say the ratchet was skipped: {}",
            no_ratchet.stdout
        );

        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn ratchet_names_a_new_god_file_even_when_the_count_has_slack() {
        // The #165 leak: three real branches each added a god file while the
        // authoritative self-scan stayed green, because the ratchet compared
        // a count against a stale baseline with slack. The sharpest version
        // is a fix+regress swap — count flat, yet a brand-new monolith
        // appeared. Identity comparison must catch it and must name the file.
        let root = fixture_root("sentrux-native-god-swap");
        fs::create_dir_all(root.join("src")).expect("create fixture");
        let mut old_god = String::from("pub fn old_entry() {}\n");
        for line in 0..850 {
            old_god.push_str(&format!("// padding {line}\n"));
        }
        fs::write(root.join("src/old_god.rs"), &old_god).expect("write old god file");
        fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");

        let saved = expect_gate_ran(&root, true);
        assert!(saved.success, "stdout: {}", saved.stdout);

        // Fix the tolerated god file and introduce a brand-new one: the god
        // count stays exactly flat, which the old count ratchet waved green.
        fs::write(root.join("src/old_god.rs"), "pub fn old_entry() {}\n")
            .expect("shrink old god file");
        let mut new_god = String::from("pub fn new_entry() {}\n");
        for line in 0..900 {
            new_god.push_str(&format!("// padding {line}\n"));
        }
        fs::write(root.join("src/new_god.rs"), &new_god).expect("write new god file");

        let gate = expect_gate_ran(&root, false);
        assert!(!gate.success, "stdout: {}", gate.stdout);
        let violation = gate
            .violations
            .iter()
            .find(|violation| violation.rule == "god_files_increased")
            .expect("god_files_increased fires on a new identity");
        assert!(
            violation
                .message
                .contains("src/new_god.rs (loc 901, functions 1; rule: loc>800)"),
            "message must name the new file with its measured values: {}",
            violation.message
        );
        assert!(
            !violation.message.contains("old_god"),
            "the fixed file is not part of the regression: {}",
            violation.message
        );
        assert_eq!(violation.targets, vec!["src/new_god.rs".to_string()]);
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn ratchet_reports_the_functions_branch_and_counts_test_functions() {
        // Decision recorded in .sentrux/rules.toml (#165 item 3, option B):
        // `functions` counts every recognised function, test functions
        // included. The repository convention — tests live in a module
        // directory's own tests.rs — is what keeps production files under
        // the limit, so the count stays honest instead of needing cfg-aware
        // parsing. This test pins both the branch label and the inclusive
        // count.
        let root = fixture_root("sentrux-native-god-functions");
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");
        let saved = expect_gate_ran(&root, true);
        assert!(saved.success, "stdout: {}", saved.stdout);

        let mut dense = String::new();
        for index in 0..20 {
            dense.push_str(&format!("pub fn production_{index}() {{}}\n"));
        }
        dense.push_str("#[cfg(test)]\nmod tests {\n");
        for index in 0..6 {
            dense.push_str(&format!("    fn test_case_{index}() {{}}\n"));
        }
        dense.push_str("}\n");
        for line in 0..380 {
            dense.push_str(&format!("// padding {line}\n"));
        }
        fs::write(root.join("src/dense.rs"), &dense).expect("write dense file");

        let gate = expect_gate_ran(&root, false);
        assert!(!gate.success, "stdout: {}", gate.stdout);
        let violation = gate
            .violations
            .iter()
            .find(|violation| violation.rule == "god_files_increased")
            .expect("god_files_increased fires");
        assert!(
            violation
                .message
                .contains("src/dense.rs (loc 409, functions 26; rule: functions>25&&loc>400)"),
            "message must name the functions branch with measured values: {}",
            violation.message
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn green_gate_advises_tightening_when_baseline_god_entries_resolved() {
        // The gate must not rewrite the baseline (a scan does not mutate the
        // repository), but a baseline still listing resolved files is slack a
        // regression could hide in — the gate says so on an otherwise green
        // run instead of staying silent.
        let root = fixture_root("sentrux-native-god-resolved");
        fs::create_dir_all(root.join("src")).expect("create fixture");
        let mut god = String::from("pub fn entry() {}\n");
        for line in 0..850 {
            god.push_str(&format!("// padding {line}\n"));
        }
        fs::write(root.join("src/was_god.rs"), &god).expect("write god file");
        let saved = expect_gate_ran(&root, true);
        assert!(saved.success, "stdout: {}", saved.stdout);

        fs::write(root.join("src/was_god.rs"), "pub fn entry() {}\n").expect("shrink god file");
        let gate = expect_gate_ran(&root, false);
        assert!(gate.success, "stdout: {}", gate.stdout);
        assert!(
            gate.stdout.contains("no longer over threshold"),
            "green run must surface reclaimable ratchet slack: {}",
            gate.stdout
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn gate_rejects_a_baseline_without_god_file_identities() {
        // A v5-schema baseline that carries counts but no godFiles list can
        // only support the count ratchet #165 retired — refusing it keeps the
        // identity comparison from silently degrading to the leaky one.
        let root = fixture_root("sentrux-native-god-identities");
        fs::create_dir_all(root.join(".sentrux")).expect("create fixture");
        fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").expect("write fixture source");
        fs::write(
            root.join(".sentrux/baseline.json"),
            format!(
                "{{\"schema\":\"{BASELINE_SCHEMA}\",\"engine\":{{\"id\":\"{ENGINE_ID}\",\"version\":\"{ENGINE_VERSION}\"}},\"metrics\":{{\"quality_signal\":1.0,\"coupling_score\":0.0,\"cycle_count\":0,\"god_file_count\":0}}}}"
            ),
        )
        .expect("write baseline");
        let run = expect_gate_ran(&root, false);
        assert!(!run.success);
        assert_eq!(run.violations[0].rule, "baseline_engine_mismatch");
        assert!(
            run.violations[0].message.contains("godFiles"),
            "{}",
            run.violations[0].message
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn gate_rejects_a_v5_schema_baseline_from_before_the_quality_signal_rewrite() {
        // #385/DR-0011: `quality_signal` changed meaning (10000-penalty ->
        // upstream-compatible geometric mean), so a v5-schema baseline's
        // `quality_signal` number is not comparable to this engine's output
        // at all -- comparing it would silently report a fabricated
        // before/after delta rather than the real one. The schema bump
        // (v5 -> v6) alone must fail this closed.
        let root = fixture_root("sentrux-native-v5-baseline-rejected");
        fs::create_dir_all(root.join(".sentrux")).expect("create fixture");
        fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").expect("write fixture source");
        fs::write(
            root.join(".sentrux/baseline.json"),
            format!(
                "{{\"schema\":\"code-intel-sentrux-baseline.v5\",\"engine\":{{\"id\":\"{ENGINE_ID}\",\"version\":\"2.2.0\"}},\"metrics\":{{\"quality_signal\":9500,\"coupling_score\":0.0,\"cycle_count\":0,\"god_file_count\":0}},\"godFiles\":[]}}"
            ),
        )
        .expect("write v5 baseline");
        let run = expect_gate_ran(&root, false);
        assert!(!run.success);
        assert_eq!(run.violations[0].rule, "baseline_engine_mismatch");
        assert!(
            run.violations[0].message.contains(BASELINE_SCHEMA),
            "rejection reason must name the required schema so it is machine-readable: {}",
            run.violations[0].message
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn scan_json_exposes_the_full_quality_signal_v1_contract() {
        // #385 acceptance: scan/health's structured result must carry
        // quality_signal, bottleneck, and root_causes.<metric>.{score,raw}
        // for all five root causes, plus the formula/provider version and
        // completeness the issue names explicitly.
        let root = fixture_root("sentrux-native-quality-signal-contract");
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(
            root.join("src/a.rs"),
            "use crate::b;\npub fn a() { b::b(); }\n",
        )
        .expect("write a.rs");
        fs::write(root.join("src/b.rs"), "pub fn b() {}\n").expect("write b.rs");
        let value = scan_json(&root).expect("scan_json on fixture");
        assert!(value["quality_signal"].as_i64().is_some());
        let detail = &value["quality_signal_detail"];
        assert_eq!(detail["schema"], "code-intel-sentrux-quality-signal.v1");
        assert_eq!(
            detail["formula_version"],
            sentrux_quality_signal::FORMULA_VERSION
        );
        assert_eq!(
            detail["provider_version"],
            sentrux_quality_signal::PROVIDER_VERSION
        );
        assert!(detail["bottleneck"].as_str().is_some());
        for metric in ["modularity", "acyclicity", "depth", "equality", "redundancy"] {
            let cause = &detail["root_causes"][metric];
            assert!(
                cause["score"].as_i64().is_some(),
                "{metric}.score missing: {cause}"
            );
            assert!(!cause["raw"].is_null(), "{metric}.raw missing: {cause}");
            assert!(
                cause["completeness"].as_str().is_some(),
                "{metric}.completeness missing: {cause}"
            );
        }
        // Redundancy honestly discloses its known gap instead of a fake 0.
        assert_eq!(detail["root_causes"]["redundancy"]["completeness"], "partial");
        assert!(detail["root_causes"]["redundancy"]["note"]
            .as_str()
            .unwrap_or_default()
            .contains("dead-function detection not implemented"));
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn empty_repository_quality_signal_has_no_nan_and_reports_trivial_defaults() {
        // #385 acceptance: empty repo / zero functions must not produce
        // NaN/Inf, and the JSON must still parse and stay in [0, 10000].
        let root = fixture_root("sentrux-native-quality-signal-empty");
        fs::create_dir_all(&root).expect("create empty fixture root");
        let value = scan_json(&root).expect("scan_json on empty repository");
        let score = value["quality_signal"].as_i64().expect("integer score");
        assert!((0..=10000).contains(&score));
        let detail = &value["quality_signal_detail"];
        for metric in ["modularity", "acyclicity", "depth", "equality", "redundancy"] {
            let raw = &detail["root_causes"][metric]["raw"];
            assert!(raw.is_number(), "{metric}.raw must be a number, not NaN/null: {raw}");
        }
        // No edges, no cycles, no functions: every trivial-input convention
        // upstream itself defines yields a perfect score.
        assert_eq!(score, 10000);
        fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn quality_signal_bottleneck_names_the_true_lowest_root_cause_on_a_real_cycle() {
        // End-to-end: a real mutual-import cycle between two files must
        // depress `acyclicity` and surface as the bottleneck, proving the
        // resolved multi-language edge graph -> SCC -> aggregate pipeline
        // is wired correctly, not just its pure-math unit tests.
        let root = fixture_root("sentrux-native-quality-signal-cycle");
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(
            root.join("src/a.rs"),
            "use crate::b;\npub fn a() { crate::b::b(); }\n",
        )
        .expect("write a.rs");
        fs::write(
            root.join("src/b.rs"),
            "use crate::a;\npub fn b() { crate::a::a(); }\n",
        )
        .expect("write b.rs");
        let value = scan_json(&root).expect("scan_json on cyclic fixture");
        let raw_cycles = value["quality_signal_detail"]["root_causes"]["acyclicity"]["raw"]
            .as_i64()
            .expect("acyclicity raw cycle count");
        assert!(raw_cycles >= 1, "expected at least one detected cycle, got {raw_cycles}");
        fs::remove_dir_all(&root).expect("remove fixture");
    }
}
