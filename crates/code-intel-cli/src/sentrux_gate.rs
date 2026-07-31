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

#[path = "hardened_git.rs"]
mod hardened_git;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub(crate) const ENGINE_ID: &str = "sentrux-native";
pub(crate) const ENGINE_VERSION: &str = "2.1.0";
// v3 because `coupling_score` changed denominator in 2.1.0. A v2 baseline
// holds a number this engine can no longer produce, so comparing against it
// would report a large fabricated coupling regression on the first run after
// upgrade. The schema bump turns that into `baseline_engine_mismatch`, which
// tells the operator to re-baseline deliberately.
pub(crate) const BASELINE_SCHEMA: &str = "code-intel-sentrux-baseline.v3";

// Metric keys the gate comparisons in `run_gate` read from the baseline.
// `number()` defaults an absent key to 0.0, so a baseline missing any of
// these would silently disable its comparison instead of failing closed.
const GATED_METRIC_KEYS: [&str; 4] = [
    "quality_signal",
    "coupling_score",
    "cycle_count",
    "god_file_count",
];

const CODE_EXTENSIONS: &[&str] = &[
    "ps1", "psm1", "py", "rs", "go", "ts", "tsx", "js", "jsx", "mjs", "cjs", "java", "cs", "cpp",
    "c", "h", "hpp", "v",
];
// Subset of CODE_EXTENSIONS whose dependency declarations `is_import_line`
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
const SKIP_DIRECTORIES: &[&str] = &[
    ".git",
    ".repowise",
    ".understand-anything",
    ".sentrux",
    "tools",
    "vendor",
    "vendors",
    "third_party",
    "third-party",
    "external",
    "node_modules",
    ".pnpm",
    ".yarn",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".venv",
    "venv",
    "env",
    ".tox",
    "__pycache__",
    ".next",
    ".nuxt",
    ".turbo",
    ".cache",
];
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_VIOLATION_TARGETS: usize = 8;

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
    quality_signal: i64,
    cycle_count: i64,
    cycles: Vec<Vec<String>>,
}

pub(crate) fn run_check(repo: &Path) -> Result<EngineRun, String> {
    let metrics = measure_project(repo)?;
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
    let violations = evaluate_rules(&rules, &metrics);
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

pub(crate) fn run_gate(repo: &Path, save: bool) -> Result<EngineRun, String> {
    let metrics = measure_project(repo)?;
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
    {
        let message = format!(
            "baseline engine mismatch: {} requires schema {BASELINE_SCHEMA} with engine {ENGINE_ID} and numeric {} metrics; found schema {} engine {}",
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
    if (metrics.god_file_count as f64) > number(before, "god_file_count") {
        violations.push(Violation {
            rule: "god_files_increased".into(),
            message: format!(
                "God files: {} -> {}",
                number(before, "god_file_count"),
                metrics.god_file_count
            ),
            targets: god_file_targets(&metrics),
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

pub(crate) fn scan_json(repo: &Path) -> Result<Value, String> {
    let metrics = measure_project(repo)?;
    Ok(metrics_json(repo, &metrics))
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
            "total_import_edges": metrics.total_import_edges,
            "cross_module_edges": metrics.total_import_edges,
            "files": metrics.file_count,
            "import_modeled_files": metrics.import_modeled_file_count,
            "functions": metrics.function_count,
        }
    }))
}

fn metrics_json(repo: &Path, metrics: &ProjectMetrics) -> Value {
    json!({
        "tool": ENGINE_ID,
        "engine": {"id": ENGINE_ID, "version": ENGINE_VERSION},
        "path": repo.display().to_string(),
        "quality_signal": metrics.quality_signal,
        "files": metrics.file_count,
        "import_modeled_files": metrics.import_modeled_file_count,
        "functions": metrics.function_count,
        "coupling_score": metrics.coupling_score,
        "cycle_count": metrics.cycle_count,
        "cycles": metrics.cycles,
        "god_file_count": metrics.god_file_count,
        "complex_fn_count": metrics.complex_fn_count,
        "max_complexity": metrics.max_complexity,
        "total_import_edges": metrics.total_import_edges,
        "cross_module_edges": metrics.total_import_edges,
        "call_edges": metrics.call_edges,
        "unresolved_imports": 0,
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

fn evaluate_rules(rules: &str, metrics: &ProjectMetrics) -> Vec<Violation> {
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

fn measure_project(repo: &Path) -> Result<ProjectMetrics, String> {
    let repo = repo
        .canonicalize()
        .map_err(|error| format!("resolve repository path: {error}"))?;
    let mut paths = Vec::new();
    collect_files(&repo, &repo, &mut paths)?;
    paths.sort();
    let mut files = Vec::new();
    for relative in &paths {
        let content = fs::read(repo.join(relative)).unwrap_or_default();
        let content = String::from_utf8_lossy(&content);
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
    let quality_signal = (10000.0
        - coupling_score * 8.0
        - complex_fn_count as f64 * 60.0
        - god_file_count as f64 * 120.0
        - ((max_complexity - 15).max(0) as f64) * 10.0)
        .max(0.0) as i64;
    let cycles = rust_import_cycles(&repo, &paths);
    Ok(ProjectMetrics {
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
    })
}

fn collect_files(root: &Path, directory: &Path, paths: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read {}: {error}", directory.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if SKIP_DIRECTORIES.contains(&name.as_str())
                || (name.starts_with('.') && name.len() > 1)
                || name.starts_with(".venv-")
                || name.starts_with("venv-")
                || name.starts_with("env-")
            {
                continue;
            }
            collect_files(root, &path, paths)?;
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !CODE_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        if entry
            .metadata()
            .map(|metadata| metadata.len() > MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("relativize {}: {error}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        if is_skipped_relative(&relative) {
            continue;
        }
        paths.push(relative);
    }
    Ok(())
}

fn is_skipped_relative(relative: &str) -> bool {
    let lowered = relative.to_ascii_lowercase();
    if lowered.starts_with("static/")
        || lowered.starts_with("public/")
        || lowered.starts_with("wwwroot/")
        || lowered.contains("/static/")
        || lowered.contains("/public/")
        || lowered.contains("/wwwroot/")
    {
        return true;
    }
    let leaf = lowered.rsplit('/').next().unwrap_or(&lowered);
    if leaf.ends_with(".min.js") || leaf.ends_with(".bundle.js") {
        return true;
    }
    false
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
    let complexity_terms = complexity_terms(content);
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
        god_file: loc > 800 || (functions > 25 && loc > 400),
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

fn complexity_terms(content: &str) -> i64 {
    const WORDS: &[&str] = &[
        "if", "for", "foreach", "while", "switch", "case", "catch", "except", "match", "and", "or",
    ];
    let mut count = 0i64;
    for token in content.split(|character: char| !character.is_alphanumeric() && character != '_') {
        if WORDS.contains(&token) {
            count += 1;
        }
    }
    let bytes = content.as_bytes();
    for window in bytes.windows(2) {
        if window == b"&&" || window == b"||" {
            count += 1;
        }
    }
    count
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
        let run = run_check(&repo).expect("sentrux check should run against this repository");
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
        // A new extension added to CODE_EXTENSIONS without a decision here
        // would silently land in the coupling denominator with zero imports,
        // which is the exact defect this split exists to prevent.
        for extension in CODE_EXTENSIONS {
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
                CODE_EXTENSIONS.contains(extension),
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
        let rust_only = measure_project(&root).expect("measure rust-only tree");
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
        let mixed = measure_project(&root).expect("measure mixed tree");
        assert_eq!(mixed.file_count, 11);
        assert_eq!(mixed.import_modeled_file_count, 1);
        assert_eq!(mixed.coupling_score, rust_only.coupling_score);

        // Deleting PowerShell — the whole point of the migration — must not
        // register as a coupling regression either.
        fs::remove_file(root.join("helper0.ps1")).expect("remove one powershell fixture");
        let after_deletion = measure_project(&root).expect("measure tree after deletion");
        assert_eq!(after_deletion.coupling_score, mixed.coupling_score);
        fs::remove_dir_all(&root).expect("remove fixture");
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
        let run = run_gate(&root, false).expect("gate runs");
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
        let run = run_gate(&root, false).expect("gate runs");
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

        let saved = run_gate(&root, true).expect("save baseline");
        assert!(saved.success);
        let clean = run_gate(&root, false).expect("gate after save");
        assert!(clean.success, "stdout: {}", clean.stdout);

        fs::create_dir_all(root.join(".sentrux")).expect("sentrux dir exists");
        fs::write(
            root.join(".sentrux/rules.toml"),
            "[constraints]\nmax_cycles = 0\n",
        )
        .expect("write rules");
        let check = run_check(&root).expect("check runs");
        assert!(!check.success);
        assert_eq!(check.violations[0].rule, "max_cycles");
        assert_eq!(
            check.violations[0].targets,
            vec!["src/a.rs".to_string(), "src/b.rs".into()]
        );
        fs::remove_dir_all(&root).expect("remove fixture");
    }
}
