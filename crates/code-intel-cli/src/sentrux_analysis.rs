use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE_EXTENSIONS: [&str; 14] = [
    ".ps1", ".psm1", ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs", ".go", ".java",
    ".cs", ".v",
];

#[derive(Clone, Default)]
struct GitSignal {
    status: &'static str,
    dirty: bool,
    untracked: bool,
    age_days: Option<i64>,
    churn: i64,
    last_commit_unix: Option<i64>,
}

#[derive(Clone, Default)]
struct ModuleMetrics {
    files: i64,
    source_files: i64,
    test_files: i64,
    test_gap: i64,
    avg_age_days: Option<i64>,
    max_age_days: Option<i64>,
    churn: i64,
    dirty_files: i64,
    untracked_files: i64,
    git_files: i64,
    inbound_edges: i64,
    outbound_edges: i64,
    coupling: i64,
    exec_depth: i64,
    blast_radius: i64,
    risk: i64,
}

struct Inventory {
    files: Vec<String>,
    scope: Value,
}

pub fn analyze(target: &Path) -> Result<Value, String> {
    let target = target
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", target.display()))?;
    if !target.is_dir() {
        return Err(format!(
            "DSM target is not a directory: {}",
            target.display()
        ));
    }

    let inventory = source_inventory(&target)?;
    let git = git_signals(&target, &inventory.files);
    let mut file_details = Vec::with_capacity(inventory.files.len());
    let mut contents = BTreeMap::new();
    let mut modules: BTreeMap<String, ModuleMetrics> = BTreeMap::new();
    let mut module_files: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for relative in &inventory.files {
        let source_path = target.join(relative);
        let content = fs::read_to_string(&source_path)
            .map_err(|error| format!("read source {}: {error}", source_path.display()))?;
        let module = module_name(relative);
        let signal = git.get(relative).cloned().unwrap_or_else(untracked_signal);
        let metrics = modules.entry(module.clone()).or_default();
        metrics.files += 1;
        if is_test_file(relative) {
            metrics.test_files += 1;
        } else {
            metrics.source_files += 1;
        }
        metrics.churn += signal.churn;
        metrics.dirty_files += i64::from(signal.dirty);
        metrics.untracked_files += i64::from(signal.untracked);
        metrics.git_files += i64::from(signal.dirty || signal.untracked);
        module_files
            .entry(module)
            .or_default()
            .push(relative.clone());
        file_details.push(file_detail(relative, &content, &signal));
        contents.insert(relative.clone(), content);
    }

    let edges = dsm_edges(&inventory.files, &contents, &modules);
    derive_module_metrics(&mut modules, &module_files, &git, &edges);
    score_risk(&mut modules);

    file_details.sort_by(|left, right| {
        integer(right, "max_complexity")
            .cmp(&integer(left, "max_complexity"))
            .then_with(|| string(left, "path").cmp(string(right, "path")))
    });

    Ok(json!({
        "tool": "dsm",
        "path": cli_path(&target),
        "scope": inventory.scope,
        "default_color_mode": "Risk",
        "color_modes": color_modes(),
        "modules": module_output(&modules),
        "file_details": file_details,
        "edges": edge_output(&edges),
        "note": "Lightweight DSM with 9 color modes. Git-derived modes depend on local git history; use Sentrux/CodeNexus for authoritative graph detail."
    }))
}

fn source_inventory(target: &Path) -> Result<Inventory, String> {
    let output = Command::new("rg")
        .arg("--files")
        .current_dir(target)
        .output()
        .map_err(|error| format!("run rg --files in {}: {error}", target.display()))?;
    if !output.status.success() {
        return Err(format!(
            "rg --files failed in {}: {}",
            target.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let listed = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(normalize_path)
        .collect::<Vec<_>>();

    let mut included = Vec::new();
    let mut excluded: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for relative in listed {
        let extension = extension(&relative);
        if !SOURCE_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        let mut reason = excluded_reason(&relative);
        if reason.is_none()
            && matches!(extension.as_str(), ".js" | ".jsx" | ".mjs" | ".cjs")
            && fs::metadata(target.join(&relative))
                .map(|metadata| metadata.len() > 2_097_152)
                .unwrap_or(false)
        {
            reason = Some("oversized_generated_or_bundle".to_string());
        }
        if let Some(reason) = reason {
            let entry = excluded.entry(reason).or_default();
            entry.0 += 1;
            if entry.1.len() < 8 {
                entry.1.push(relative);
            }
        } else {
            included.push(relative);
        }
    }
    included.sort();
    included.dedup();
    let excluded_total = excluded.values().map(|entry| entry.0).sum::<usize>();
    let mut excluded_by_reason = excluded
        .into_iter()
        .map(|(reason, (files, samples))| json!({"reason": reason, "files": files, "samples": samples}))
        .collect::<Vec<_>>();
    excluded_by_reason.sort_by(|left, right| {
        integer(right, "files")
            .cmp(&integer(left, "files"))
            .then_with(|| string(left, "reason").cmp(string(right, "reason")))
    });
    let included_count = included.len();
    Ok(Inventory {
        files: included,
        scope: json!({
            "mode": "auto_governed_source",
            "included_files": included_count,
            "excluded_files": excluded_total,
            "excluded_by_reason": excluded_by_reason,
            "source_extensions": SOURCE_EXTENSIONS,
            "note": "Root paths are allowed. Dependency, build-output, cache, and bundled static-asset code is excluded from governed source metrics."
        }),
    })
}

fn excluded_reason(relative: &str) -> Option<String> {
    let lower = relative.to_ascii_lowercase();
    let parts = lower
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if let Some(top) = parts.first() {
        if ["tools", "vendor", "third_party", "external"].contains(top) {
            return Some(format!("external_tooling_dir:{top}"));
        }
    }
    let excluded = [
        ".git",
        ".repowise",
        ".understand-anything",
        ".sentrux",
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
    if let Some(part) = parts.iter().find(|part| excluded.contains(part)) {
        return Some(format!("excluded_dir:{part}"));
    }
    if lower.starts_with("static/assets/")
        || lower.starts_with("public/assets/")
        || lower.starts_with("wwwroot/assets/")
    {
        return Some("bundled_static_assets".to_string());
    }
    let leaf = relative.rsplit('/').next().unwrap_or(relative);
    let leaf_lower = leaf.to_ascii_lowercase();
    if [
        ".min.js",
        ".bundle.js",
        ".min.jsx",
        ".bundle.jsx",
        ".min.mjs",
        ".bundle.mjs",
        ".min.cjs",
        ".bundle.cjs",
    ]
    .iter()
    .any(|suffix| leaf_lower.ends_with(suffix))
    {
        return Some("bundled_or_minified_file".to_string());
    }
    None
}

fn git_signals(target: &Path, files: &[String]) -> BTreeMap<String, GitSignal> {
    let mut signals = files
        .iter()
        .map(|file| (file.clone(), untracked_signal()))
        .collect::<BTreeMap<_, _>>();
    if files.is_empty() || !git_ok(target, &["rev-parse", "--is-inside-work-tree"]) {
        return signals;
    }

    for batch in files.chunks(80) {
        for tracked in git_lines(target, "ls-files", &[], batch) {
            if let Some(signal) = resolve_git_key(&tracked, &mut signals) {
                *signal = GitSignal {
                    status: "clean",
                    ..GitSignal::default()
                };
            }
        }
        for modified in git_lines(target, "ls-files", &["--modified"], batch) {
            if let Some(signal) = resolve_git_key(&modified, &mut signals) {
                signal.status = "dirty";
                signal.dirty = true;
                signal.untracked = false;
            }
        }
        for untracked in git_lines(
            target,
            "ls-files",
            &["--others", "--exclude-standard"],
            batch,
        ) {
            if let Some(signal) = resolve_git_key(&untracked, &mut signals) {
                *signal = untracked_signal();
            }
        }
        apply_git_log(target, batch, &mut signals);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    for signal in signals.values_mut() {
        signal.age_days = signal
            .last_commit_unix
            .map(|timestamp| (now - timestamp).max(0) / 86_400);
    }
    signals
}

fn untracked_signal() -> GitSignal {
    GitSignal {
        status: "untracked",
        untracked: true,
        ..GitSignal::default()
    }
}

fn git_ok(target: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(target)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_lines(target: &Path, command: &str, extra: &[&str], files: &[String]) -> Vec<String> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(target)
        .arg(command)
        .args(extra)
        .arg("--")
        .args(files)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(normalize_path)
        .filter(|line| !line.is_empty())
        .collect()
}

fn apply_git_log(target: &Path, files: &[String], signals: &mut BTreeMap<String, GitSignal>) {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["log", "--format=__SENTRUX_COMMIT__%ct", "--name-only", "--"])
        .args(files)
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let mut commit = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if let Some(timestamp) = line.strip_prefix("__SENTRUX_COMMIT__") {
            commit = timestamp.parse::<i64>().ok();
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if let (Some(timestamp), Some(signal)) = (commit, resolve_git_key(line, signals)) {
            signal.churn += 1;
            signal.last_commit_unix = Some(signal.last_commit_unix.unwrap_or(0).max(timestamp));
        }
    }
}

fn resolve_git_key<'a>(
    path: &str,
    signals: &'a mut BTreeMap<String, GitSignal>,
) -> Option<&'a mut GitSignal> {
    let candidate = normalize_path(path);
    if signals.contains_key(&candidate) {
        return signals.get_mut(&candidate);
    }
    let key = signals
        .keys()
        .find(|key| {
            candidate
                .strip_suffix(key.as_str())
                .is_some_and(|prefix| prefix.ends_with('/'))
                || key
                    .strip_suffix(candidate.as_str())
                    .is_some_and(|prefix| prefix.ends_with('/'))
        })?
        .clone();
    signals.get_mut(&key)
}

fn file_detail(relative: &str, content: &str, signal: &GitSignal) -> Value {
    let lines = split_lines(content);
    let language = language(relative);
    let mut functions = functions(language, &lines);
    for function in &mut functions {
        let name = string(function, "name").to_string();
        let start = integer(function, "start_line");
        let end = integer(function, "end_line");
        if let Some(object) = function.as_object_mut() {
            object.insert(
                "id".to_string(),
                json!(stable_id(&format!(
                    "function:{relative}:{name}:{start}:{end}"
                ))),
            );
            object.insert("source_anchor".to_string(), source_anchor(relative, start));
        }
    }
    functions.sort_by(|left, right| integer(right, "complexity").cmp(&integer(left, "complexity")));
    let (lines_count, loc, blank, comments) = line_stats(&lines);
    let total_complexity = functions
        .iter()
        .map(|function| integer(function, "complexity"))
        .sum::<i64>();
    let max_complexity = functions
        .iter()
        .map(|function| integer(function, "complexity"))
        .max()
        .unwrap_or(0);
    let average = if functions.is_empty() {
        0.0
    } else {
        round2(total_complexity as f64 / functions.len() as f64)
    };
    json!({
        "id": stable_id(&format!("file:{relative}")),
        "path": relative,
        "module": module_name(relative),
        "language": language,
        "source_anchor": source_anchor(relative, 1),
        "lines": lines_count,
        "loc": loc,
        "blank_lines": blank,
        "comment_lines": comments,
        "function_count": functions.len(),
        "max_complexity": max_complexity,
        "avg_complexity": average,
        "total_complexity": total_complexity,
        "git": {"status": signal.status, "dirty": signal.dirty, "untracked": signal.untracked, "age_days": signal.age_days, "churn": signal.churn},
        "functions": functions
    })
}

fn functions(language: &str, lines: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        let parsed = match language {
            "python" => parse_python_signature(line),
            "rust" => parse_c_signature(line, "fn ", true),
            "vlang" => parse_c_signature(line, "fn ", false),
            "typescript" | "javascript" => parse_javascript_signature(line),
            "powershell" => parse_powershell_signature(line),
            _ => None,
        };
        if let Some((name, mut params, is_async, is_public)) = parsed {
            let end = if language == "python" {
                python_end(lines, index)
            } else {
                c_like_end(lines, index)
            };
            let body = &lines[index..=end.min(lines.len().saturating_sub(1))];
            if language == "powershell" && params.is_empty() {
                params = body
                    .iter()
                    .find_map(|line| {
                        let trimmed = line.trim_start();
                        trimmed
                            .to_ascii_lowercase()
                            .starts_with("param(")
                            .then(|| trimmed[6..].to_string())
                    })
                    .unwrap_or_default();
            }
            let (count, loc, _, _) = line_stats(body);
            out.push(json!({
                "name": name, "kind": "function", "start_line": index + 1, "end_line": end + 1,
                "lines": count, "loc": loc, "complexity": complexity(language, body), "params": param_count(&params),
                "async": is_async, "public": is_public
            }));
        }
        index += 1;
    }
    out
}

fn parse_python_signature(line: &str) -> Option<(String, String, bool, bool)> {
    let trimmed = line.trim_start();
    let (rest, is_async) = if let Some(rest) = trimmed.strip_prefix("async def ") {
        (rest, true)
    } else {
        (trimmed.strip_prefix("def ")?, false)
    };
    let open = rest.find('(')?;
    let close = rest[open + 1..]
        .find(')')
        .map(|offset| offset + open + 1)
        .unwrap_or(rest.len());
    let name = identifier(&rest[..open])?;
    if rest[..open].trim() != name {
        return None;
    }
    Some((
        name.to_string(),
        rest[open + 1..close].to_string(),
        is_async,
        false,
    ))
}

fn parse_c_signature(
    line: &str,
    _marker: &str,
    rust: bool,
) -> Option<(String, String, bool, bool)> {
    let trimmed = line.trim_start();
    let (rest, is_async, is_public) = if let Some(rest) = trimmed.strip_prefix("fn ") {
        (rest, false, false)
    } else if rust {
        if let Some(rest) = trimmed.strip_prefix("async fn ") {
            (rest, true, false)
        } else if let Some(rest) = trimmed.strip_prefix("pub fn ") {
            (rest, false, true)
        } else if let Some(rest) = trimmed.strip_prefix("pub async fn ") {
            (rest, true, true)
        } else if let Some(scoped) = trimmed.strip_prefix("pub(") {
            let after_scope = scoped.split_once(") ")?.1;
            if let Some(rest) = after_scope.strip_prefix("fn ") {
                (rest, false, true)
            } else {
                (after_scope.strip_prefix("async fn ")?, true, true)
            }
        } else {
            return None;
        }
    } else {
        (trimmed.strip_prefix("pub fn ")?, false, true)
    };
    let open = rest.find('(')?;
    let close = rest[open + 1..]
        .find(')')
        .map(|offset| offset + open + 1)
        .unwrap_or(rest.len());
    let name = identifier(&rest[..open])?;
    if rest[..open].trim() != name {
        return None;
    }
    Some((
        name.to_string(),
        rest[open + 1..close].to_string(),
        is_async,
        is_public,
    ))
}

fn parse_javascript_signature(line: &str) -> Option<(String, String, bool, bool)> {
    let trimmed = line.trim_start();
    let is_public = trimmed.starts_with("export ");
    let is_async = trimmed.contains("async function ") || trimmed.contains("= async ");
    if let Some(position) = trimmed.find("function ") {
        let rest = &trimmed[position + 9..];
        let open = rest.find('(')?;
        let close = rest[open + 1..]
            .find(')')
            .map(|offset| offset + open + 1)
            .unwrap_or(rest.len());
        let name = identifier(&rest[..open])?;
        if rest[..open].trim() != name {
            return None;
        }
        return Some((
            name.to_string(),
            rest[open + 1..close].to_string(),
            is_async,
            is_public,
        ));
    }
    let assignment = trimmed.find("=>")?;
    let before = trimmed[..assignment].trim();
    let equals = before.rfind('=')?;
    let binding = before[..equals].split_whitespace().last()?;
    let params = before[equals + 1..]
        .trim()
        .trim_start_matches("async")
        .trim()
        .trim_matches(['(', ')']);
    Some((
        identifier(binding)?.to_string(),
        params.to_string(),
        is_async,
        is_public,
    ))
}

fn parse_powershell_signature(line: &str) -> Option<(String, String, bool, bool)> {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    let rest = trimmed
        .get(9..)
        .filter(|_| lower.starts_with("function "))?
        .trim_start();
    let name = rest
        .split(|ch: char| ch.is_whitespace() || ch == '{' || ch == '(')
        .next()?;
    (!name.is_empty()).then(|| (name.to_string(), String::new(), false, true))
}

fn python_end(lines: &[String], start: usize) -> usize {
    let indent = leading_whitespace(&lines[start]);
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('@') {
            if leading_whitespace(line) <= indent {
                return index.saturating_sub(1).max(start);
            }
            continue;
        }
        if leading_whitespace(line) <= indent {
            return index.saturating_sub(1).max(start);
        }
    }
    lines.len().saturating_sub(1).max(start)
}

fn c_like_end(lines: &[String], start: usize) -> usize {
    let mut depth = 0i64;
    let mut seen_brace = false;
    let mut block_comment = false;
    for (index, line) in lines.iter().enumerate().skip(start) {
        let structure = structural_line(line, &mut block_comment);
        depth += structure.opens - structure.closes;
        seen_brace |= structure.opens > 0;
        if seen_brace && depth <= 0 {
            return index;
        }
        if !seen_brace && (structure.semicolon || structure.arrow_expression) {
            return index;
        }
        if index > start && looks_like_function_declaration(line) {
            return index.saturating_sub(1).max(start);
        }
    }
    start
}

#[derive(Default)]
struct StructuralLine {
    opens: i64,
    closes: i64,
    semicolon: bool,
    arrow_expression: bool,
}

fn structural_line(line: &str, block_comment: &mut bool) -> StructuralLine {
    let chars = line.chars().collect::<Vec<_>>();
    let mut result = StructuralLine::default();
    let mut quote = None;
    let mut escaped = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        let next = chars.get(index + 1).copied();
        if *block_comment {
            if ch == '*' && next == Some('/') {
                *block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if ch == '/' && next == Some('/') {
            break;
        }
        if ch == '/' && next == Some('*') {
            *block_comment = true;
            index += 2;
            continue;
        }
        if ch == '#' && chars[..index].iter().all(|value| value.is_whitespace()) {
            break;
        }
        let is_rust_lifetime = ch == '\''
            && next.is_some_and(|value| value.is_ascii_alphabetic() || value == '_')
            && !chars[index + 1..].contains(&'\'');
        if matches!(ch, '\'' | '"' | '`') && !is_rust_lifetime {
            quote = Some(ch);
            index += 1;
            continue;
        }
        match ch {
            '{' => result.opens += 1,
            '}' => result.closes += 1,
            ';' => result.semicolon = true,
            '=' if next == Some('>') => result.arrow_expression = true,
            _ => {}
        }
        index += 1;
    }
    result
}

fn looks_like_function_declaration(line: &str) -> bool {
    parse_python_signature(line).is_some()
        || parse_c_signature(line, "fn ", true).is_some()
        || parse_c_signature(line, "fn ", false).is_some()
        || parse_javascript_signature(line).is_some()
        || parse_powershell_signature(line).is_some()
}

fn complexity(language: &str, lines: &[String]) -> i64 {
    let keywords = [
        "if", "elif", "for", "while", "except", "case", "catch", "match", "guard", "when", "with",
    ];
    let mut score = 1;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        score += keywords
            .iter()
            .map(|word| count_word(trimmed, word))
            .sum::<i64>();
        score += trimmed.matches("&&").count() as i64 + trimmed.matches("||").count() as i64;
        if !matches!(language, "javascript" | "typescript") {
            score += i64::from(trimmed.contains(" => "));
        }
    }
    score
}

fn dsm_edges(
    files: &[String],
    contents: &BTreeMap<String, String>,
    modules: &BTreeMap<String, ModuleMetrics>,
) -> BTreeMap<(String, String), i64> {
    let mut edges = BTreeMap::new();
    for file in files {
        let ext = extension(file);
        if !matches!(ext.as_str(), ".py" | ".rs" | ".v") {
            continue;
        }
        let from = module_name(file);
        let Some(content) = contents.get(file) else {
            continue;
        };
        for target in import_targets(&ext, file, content, files, modules) {
            if target != from && modules.contains_key(&target) {
                *edges.entry((from.clone(), target)).or_default() += 1;
            }
        }
    }
    edges
}

fn import_targets(
    extension: &str,
    source: &str,
    content: &str,
    files: &[String],
    modules: &BTreeMap<String, ModuleMetrics>,
) -> Vec<String> {
    let mut targets = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        match extension {
            ".py" => {
                let token = trimmed
                    .strip_prefix("from ")
                    .or_else(|| trimmed.strip_prefix("import "))
                    .and_then(|rest| rest.split_whitespace().next());
                if let Some(token) = token {
                    if let Some(target) = resolve_python_import(source, token, files) {
                        targets.push(target);
                    }
                }
            }
            ".rs" => {
                if let Some(rest) = trimmed.strip_prefix("mod ") {
                    if let Some(name) = identifier(rest) {
                        if let Some(target) = resolve_relative_source(source, name, ".rs", files) {
                            targets.push(module_name(target));
                        }
                    }
                }
                if let Some(rest) = trimmed.strip_prefix("use crate::") {
                    if let Some(name) = identifier(rest).and_then(|path| path.split("::").next()) {
                        if let Some(target) = resolve_crate_source(source, name, files) {
                            targets.push(module_name(target));
                        }
                    }
                } else if let Some(rest) = trimmed.strip_prefix("use ") {
                    if let Some(name) = identifier(rest).and_then(|path| path.split("::").next()) {
                        if !matches!(name, "crate" | "self" | "super" | "std" | "core" | "alloc") {
                            if let Some(target) = resolve_module_token(name, modules) {
                                targets.push(target);
                            }
                        }
                    }
                }
            }
            ".v" => {
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    if let Some(token) = rest.split_whitespace().next() {
                        let root = token.split('.').next().unwrap_or(token);
                        for candidate in [
                            root.to_string(),
                            format!("{root}.v"),
                            token.replace('.', "/"),
                        ] {
                            if let Some(target) = resolve_module_token(&candidate, modules) {
                                targets.push(target);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    targets
}

fn resolve_python_import(source: &str, token: &str, files: &[String]) -> Option<String> {
    let leading_dots = token.chars().take_while(|ch| *ch == '.').count();
    let mut parts = source
        .rsplit_once('/')
        .map(|(directory, _)| directory.split('/').collect::<Vec<_>>())
        .unwrap_or_default();
    if leading_dots == 0 {
        parts.clear();
    } else {
        for _ in 1..leading_dots {
            parts.pop();
        }
    }
    parts.extend(
        token[leading_dots..]
            .split('.')
            .filter(|part| !part.is_empty()),
    );
    if parts.is_empty() {
        return None;
    }
    let base = parts.join("/");
    let file_candidate = format!("{base}.py");
    let init_candidate = format!("{base}/__init__.py");
    files
        .iter()
        .find(|file| *file == &file_candidate || *file == &init_candidate)
        .or_else(|| {
            let prefix = format!("{base}/");
            files.iter().find(|file| file.starts_with(&prefix))
        })
        .map(|file| module_name(file))
}

fn resolve_relative_source<'a>(
    source: &str,
    name: &str,
    extension: &str,
    files: &'a [String],
) -> Option<&'a str> {
    let directory = source.rsplit_once('/').map(|(path, _)| path).unwrap_or("");
    let prefix = if directory.is_empty() {
        name.to_string()
    } else {
        format!("{directory}/{name}")
    };
    let direct = format!("{prefix}{extension}");
    let nested = format!("{prefix}/mod{extension}");
    files
        .iter()
        .find(|file| *file == &direct || *file == &nested)
        .map(String::as_str)
}

fn resolve_crate_source<'a>(source: &str, name: &str, files: &'a [String]) -> Option<&'a str> {
    let crate_root = source.find("/src/").map(|index| &source[..index + 4]);
    let prefix = crate_root.unwrap_or_else(|| {
        source
            .rsplit_once('/')
            .map(|(directory, _)| directory)
            .unwrap_or("")
    });
    let candidate_source = if prefix.is_empty() {
        format!("{name}.rs")
    } else {
        format!("{prefix}/{name}.rs")
    };
    let candidate_module = if prefix.is_empty() {
        format!("{name}/mod.rs")
    } else {
        format!("{prefix}/{name}/mod.rs")
    };
    files
        .iter()
        .find(|file| *file == &candidate_source || *file == &candidate_module)
        .map(String::as_str)
}

fn resolve_module_token(token: &str, modules: &BTreeMap<String, ModuleMetrics>) -> Option<String> {
    let normalized = token.trim_matches(';').replace('.', "/");
    if modules.contains_key(&normalized) {
        return Some(normalized);
    }
    let comparable = normalized.replace('_', "-");
    modules
        .keys()
        .find(|module| {
            module
                .rsplit('/')
                .next()
                .is_some_and(|leaf| leaf.replace('_', "-") == comparable)
        })
        .cloned()
}

fn derive_module_metrics(
    modules: &mut BTreeMap<String, ModuleMetrics>,
    module_files: &BTreeMap<String, Vec<String>>,
    git: &BTreeMap<String, GitSignal>,
    edges: &BTreeMap<(String, String), i64>,
) {
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ((from, to), count) in edges {
        adjacency
            .entry(from.clone())
            .or_default()
            .insert(to.clone());
        reverse.entry(to.clone()).or_default().insert(from.clone());
        if let Some(module) = modules.get_mut(from) {
            module.outbound_edges += count
        }
        if let Some(module) = modules.get_mut(to) {
            module.inbound_edges += count
        }
    }
    let depths = execution_depths(modules.keys(), &adjacency, &reverse);
    for (name, module) in modules.iter_mut() {
        let ages = module_files
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|file| git.get(file)?.age_days)
            .collect::<Vec<_>>();
        if !ages.is_empty() {
            module.avg_age_days =
                Some((ages.iter().sum::<i64>() as f64 / ages.len() as f64).round() as i64);
            module.max_age_days = ages.iter().max().copied();
        }
        module.test_gap = (module.source_files - module.test_files).max(0);
        module.coupling = module.inbound_edges + module.outbound_edges;
        module.exec_depth = depths.get(name).copied().unwrap_or(0);
        module.blast_radius = reachable(name, &reverse) as i64 + module.coupling;
    }
}

fn execution_depths<'a>(
    modules: impl Iterator<Item = &'a String>,
    adjacency: &BTreeMap<String, BTreeSet<String>>,
    reverse: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, i64> {
    let names = modules.cloned().collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut finish_order = Vec::with_capacity(names.len());
    for start in &names {
        if visited.contains(start) {
            continue;
        }
        let mut stack = vec![(start.clone(), false)];
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                finish_order.push(node);
                continue;
            }
            if !visited.insert(node.clone()) {
                continue;
            }
            stack.push((node.clone(), true));
            if let Some(next) = adjacency.get(&node) {
                for dependency in next.iter().rev() {
                    if !visited.contains(dependency) {
                        stack.push((dependency.clone(), false));
                    }
                }
            }
        }
    }

    let mut components = BTreeMap::new();
    let mut component_count = 0usize;
    for start in finish_order.into_iter().rev() {
        if components.contains_key(&start) {
            continue;
        }
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            if components.insert(node.clone(), component_count).is_some() {
                continue;
            }
            if let Some(next) = reverse.get(&node) {
                for dependent in next {
                    if !components.contains_key(dependent) {
                        stack.push(dependent.clone());
                    }
                }
            }
        }
        component_count += 1;
    }

    let mut dependencies = vec![BTreeSet::new(); component_count];
    let mut dependents = vec![BTreeSet::new(); component_count];
    for (from, targets) in adjacency {
        let Some(&from_component) = components.get(from) else {
            continue;
        };
        for target in targets {
            let Some(&to_component) = components.get(target) else {
                continue;
            };
            if from_component != to_component && dependencies[from_component].insert(to_component) {
                dependents[to_component].insert(from_component);
            }
        }
    }

    let mut remaining = dependencies.iter().map(BTreeSet::len).collect::<Vec<_>>();
    let mut component_depths = vec![0i64; component_count];
    let mut queue = VecDeque::new();
    for (component, count) in remaining.iter().enumerate() {
        if *count == 0 {
            queue.push_back(component);
        }
    }
    while let Some(dependency) = queue.pop_front() {
        for &dependent in &dependents[dependency] {
            component_depths[dependent] = component_depths[dependent]
                .max(component_depths[dependency].saturating_add(1).min(99));
            remaining[dependent] -= 1;
            if remaining[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    names
        .into_iter()
        .map(|name| {
            let depth = components
                .get(&name)
                .map(|component| component_depths[*component])
                .unwrap_or(0);
            (name, depth)
        })
        .collect()
}

fn reachable(start: &str, reverse: &BTreeMap<String, BTreeSet<String>>) -> usize {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start.to_string()]);
    while let Some(node) = queue.pop_front() {
        if let Some(next) = reverse.get(&node) {
            for item in next {
                if item != start && seen.insert(item.clone()) {
                    queue.push_back(item.clone());
                }
            }
        }
    }
    seen.len()
}

fn score_risk(modules: &mut BTreeMap<String, ModuleMetrics>) {
    let max = maximums(modules);
    for module in modules.values_mut() {
        module.risk = (heat(module.coupling, max.coupling) * 0.18
            + heat(module.blast_radius, max.blast_radius) * 0.18
            + heat(module.exec_depth, max.exec_depth) * 0.14
            + heat(module.churn, max.churn) * 0.14
            + heat(module.git_files, max.git_files) * 0.12
            + heat(module.test_gap, max.test_gap) * 0.12
            + heat(
                module.avg_age_days.unwrap_or(0),
                max.avg_age_days.unwrap_or(0),
            ) * 0.07
            + heat(module.files, max.files) * 0.05)
            .round() as i64;
    }
}

fn module_output(modules: &BTreeMap<String, ModuleMetrics>) -> Vec<Value> {
    let max = maximums(modules);
    let mut out = modules.iter().map(|(name, module)| {
        let metrics = metrics_json(module);
        json!({
            "id": stable_id(&format!("module:{name}")), "name": name, "files": module.files,
            "metrics": metrics,
            "colors": {
                "Size": color(module.files, heat(module.files, max.files)),
                "Coupling": color(module.coupling, heat(module.coupling, max.coupling)),
                "TestGap": color(module.test_gap, heat(module.test_gap, max.test_gap)),
                "Age": color_optional(module.avg_age_days, heat(module.avg_age_days.unwrap_or(0), max.avg_age_days.unwrap_or(0))),
                "Churn": color(module.churn, heat(module.churn, max.churn)),
                "Risk": color(module.risk, module.risk as f64),
                "Git": color(module.git_files, heat(module.git_files, max.git_files)),
                "ExecDepth": color(module.exec_depth, heat(module.exec_depth, max.exec_depth)),
                "BlastRadius": color(module.blast_radius, heat(module.blast_radius, max.blast_radius))
            }
        })
    }).collect::<Vec<_>>();
    out.sort_by(|left, right| {
        integer(&right["colors"]["Risk"], "score")
            .cmp(&integer(&left["colors"]["Risk"], "score"))
            .then_with(|| string(left, "name").cmp(string(right, "name")))
    });
    out
}

fn metrics_json(module: &ModuleMetrics) -> Value {
    json!({
        "files": module.files, "source_files": module.source_files, "test_files": module.test_files,
        "test_gap": module.test_gap, "avg_age_days": module.avg_age_days, "max_age_days": module.max_age_days,
        "churn": module.churn, "dirty_files": module.dirty_files, "untracked_files": module.untracked_files,
        "git_files": module.git_files, "inbound_edges": module.inbound_edges, "outbound_edges": module.outbound_edges,
        "coupling": module.coupling, "exec_depth": module.exec_depth, "blast_radius": module.blast_radius, "risk": module.risk
    })
}

fn edge_output(edges: &BTreeMap<(String, String), i64>) -> Vec<Value> {
    edges.iter().map(|((from, to), count)| json!({
        "id": stable_id(&format!("edge:{from}->{to}")), "from": from, "to": to, "count": count
    })).collect()
}

fn color_modes() -> Value {
    json!([
        {"name":"Size","key":"size","metric":"files","meaning":"module file count"},
        {"name":"Coupling","key":"coupling","metric":"coupling","meaning":"incoming plus outgoing dependency edges"},
        {"name":"TestGap","key":"test_gap","metric":"test_gap","meaning":"source files without matching test density"},
        {"name":"Age","key":"age","metric":"avg_age_days","meaning":"average days since last git commit touching files in the module"},
        {"name":"Churn","key":"churn","metric":"churn","meaning":"git commit touches for files in the module"},
        {"name":"Risk","key":"risk","metric":"risk","meaning":"composite score from coupling, blast radius, execution depth, churn, git dirtiness, test gap, age, and size"},
        {"name":"Git","key":"git","metric":"git_files","meaning":"dirty or untracked files in the module"},
        {"name":"ExecDepth","key":"exec_depth","metric":"exec_depth","meaning":"approximate dependency-chain depth from this module"},
        {"name":"BlastRadius","key":"blast_radius","metric":"blast_radius","meaning":"reachable dependents plus incident dependency edges"}
    ])
}

fn maximums(modules: &BTreeMap<String, ModuleMetrics>) -> ModuleMetrics {
    let mut max = ModuleMetrics::default();
    for module in modules.values() {
        max.files = max.files.max(module.files);
        max.coupling = max.coupling.max(module.coupling);
        max.test_gap = max.test_gap.max(module.test_gap);
        max.avg_age_days = Some(
            max.avg_age_days
                .unwrap_or(0)
                .max(module.avg_age_days.unwrap_or(0)),
        );
        max.churn = max.churn.max(module.churn);
        max.git_files = max.git_files.max(module.git_files);
        max.exec_depth = max.exec_depth.max(module.exec_depth);
        max.blast_radius = max.blast_radius.max(module.blast_radius);
    }
    max
}

fn color(value: i64, score: f64) -> Value {
    json!({"value": value, "score": score.round() as i64, "color": heat_color(score)})
}
fn color_optional(value: Option<i64>, score: f64) -> Value {
    json!({"value": value, "score": score.round() as i64, "color": heat_color(score)})
}
fn heat(value: i64, max: i64) -> f64 {
    if max <= 0 {
        0.0
    } else {
        ((value as f64 / max as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round()
    }
}

fn heat_color(score: f64) -> String {
    let bounded = score.clamp(0.0, 100.0);
    let (r, g, b) = if bounded <= 50.0 {
        let t = bounded / 50.0;
        (
            34.0 + (245.0 - 34.0) * t,
            197.0 + (158.0 - 197.0) * t,
            94.0 + (11.0 - 94.0) * t,
        )
    } else {
        let t = (bounded - 50.0) / 50.0;
        (
            245.0 + (239.0 - 245.0) * t,
            158.0 + (68.0 - 158.0) * t,
            11.0 + (68.0 - 11.0) * t,
        )
    };
    format!(
        "#{:02X}{:02X}{:02X}",
        r.round() as u8,
        g.round() as u8,
        b.round() as u8
    )
}

fn module_name(relative: &str) -> String {
    let normalized = normalize_path(relative);
    let mut parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if let Some(last) = parts.last_mut() {
        if let Some(rest) = last.strip_prefix("test_") {
            *last = rest
        }
    }
    match parts.as_slice() {
        [] => "root".to_string(),
        [one] => (*one).to_string(),
        [first, _] if ["app", "src", "tests"].contains(first) && parts[1].contains('.') => {
            (*first).to_string()
        }
        ["backend", second, third, ..] if ["app", "src", "tests"].contains(second) => {
            format!("backend/{second}/{third}")
        }
        ["backend", second, ..] => format!("backend/{second}"),
        [first, second, ..] if ["crates", "packages"].contains(first) => {
            format!("{first}/{second}")
        }
        [first, second, third, ..] if ["app", "src", "tests"].contains(first) => {
            format!("{first}/{second}/{third}")
        }
        [first, _]
            if [
                "frontend",
                "integrations",
                "research",
                "scripts",
                "services",
            ]
            .contains(first)
                && parts.len() == 2 =>
        {
            (*first).to_string()
        }
        [first, second, ..]
            if [
                "frontend",
                "integrations",
                "research",
                "scripts",
                "services",
            ]
            .contains(first) =>
        {
            format!("{first}/{second}")
        }
        [first, ..] => (*first).to_string(),
    }
}

fn is_test_file(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    let leaf = lower.rsplit('/').next().unwrap_or(&lower);
    lower.starts_with("test/")
        || lower.starts_with("tests/")
        || lower.contains("/test/")
        || lower.contains("/tests/")
        || lower.contains("/__tests__/")
        || leaf.starts_with("test_")
        || leaf.contains(".test.")
        || leaf.contains(".spec.")
}

fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    content
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

fn line_stats(lines: &[String]) -> (usize, usize, usize, usize) {
    let mut blank = 0;
    let mut comments = 0;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank += 1
        } else if trimmed.starts_with('#')
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
        {
            comments += 1
        }
    }
    (
        lines.len(),
        lines.len().saturating_sub(blank + comments),
        blank,
        comments,
    )
}

fn count_word(text: &str, word: &str) -> i64 {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|token| *token == word)
        .count() as i64
}

fn param_count(params: &str) -> usize {
    params
        .split(',')
        .map(str::trim)
        .filter(|param| {
            !param.is_empty() && !["self", "&self", "mut self", "&mut self", "cls"].contains(param)
        })
        .count()
}

fn leading_whitespace(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}
fn identifier(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let end = trimmed
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == ':' || *ch == '-')
        .last()
        .map(|(index, ch)| index + ch.len_utf8())?;
    let value = &trimmed[..end];
    value
        .chars()
        .next()
        .filter(|ch| ch.is_ascii_alphabetic() || *ch == '_')
        .map(|_| value)
}

fn language(relative: &str) -> &'static str {
    match extension(relative).as_str() {
        ".py" => "python",
        ".rs" => "rust",
        ".ts" | ".tsx" => "typescript",
        ".js" | ".jsx" | ".mjs" | ".cjs" => "javascript",
        ".ps1" | ".psm1" => "powershell",
        ".go" => "go",
        ".java" => "java",
        ".cs" => "csharp",
        ".v" => "vlang",
        _ => "unknown",
    }
}

fn extension(relative: &str) -> String {
    Path::new(relative)
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_ascii_lowercase()))
        .unwrap_or_default()
}
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim()
        .trim_start_matches("./")
        .to_string()
}
fn source_anchor(path: &str, line: i64) -> Value {
    json!({"path": path, "line": line, "label": format!("{path}:{line}")})
}
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
fn integer(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn cli_path(path: &Path) -> String {
    path.to_string_lossy()
        .strip_prefix(r"\\?\")
        .unwrap_or(&path.to_string_lossy())
        .to_string()
}

fn stable_id(text: &str) -> String {
    sha1(text.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha1(input: &[u8]) -> [u8; 20] {
    let mut message = input.to_vec();
    let bit_len = (message.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0)
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xEFCDAB89,
        0x98BADCFE,
        0x10325476,
        0xC3D2E1F0,
    ];
    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            w[index] = (w[index - 3] ^ w[index - 8] ^ w[index - 14] ^ w[index - 16]).rotate_left(1)
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (index, word) in w.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (index, word) in h.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes())
    }
    out
}
