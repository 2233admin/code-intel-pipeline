use serde_json::{json, Value};
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;

#[path = "../file_gate/mod.rs"]
mod file_gate;

#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Options<'a> {
    pub repo: &'a Path,
    pub language: &'a str,
    pub full: bool,
    pub write: bool,
    pub json: bool,
}

#[derive(Debug)]
struct SourceFile {
    rel: String,
    language: &'static str,
    bytes: u64,
    lines: usize,
    text: String,
}

pub fn run(options: &Options<'_>) -> Result<()> {
    let graph = generate(options.repo, options.language, options.full, options.write)?;
    let graph_path = graph_path(&options.repo.canonicalize()?);

    if options.json {
        println!("{}", serde_json::to_string_pretty(&graph)?);
    } else {
        let summary = &graph["summary"];
        if options.write {
            println!(
                "graph files={} edges={} symbols={} path={}",
                summary["files"].as_u64().unwrap_or(0),
                summary["edges"].as_u64().unwrap_or(0),
                summary["symbols"].as_u64().unwrap_or(0),
                graph_path.display()
            );
        } else {
            println!(
                "graph files={} edges={} symbols={} (not written; pass --write to persist)",
                summary["files"].as_u64().unwrap_or(0),
                summary["edges"].as_u64().unwrap_or(0),
                summary["symbols"].as_u64().unwrap_or(0)
            );
        }
    }

    Ok(())
}

pub fn generate(repo: &Path, language: &str, full: bool, write: bool) -> Result<Value> {
    let repo = repo.canonicalize()?;
    if !repo.is_dir() {
        return Err(format!("repo path is not a directory: {}", repo.display()).into());
    }

    let graph = build_graph(&repo, language, full)?;
    let graph_path = graph_path(&repo);

    if write {
        if let Some(parent) = graph_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&graph_path, serde_json::to_string_pretty(&graph)?)?;
    }

    Ok(graph)
}

pub fn graph_path(repo: &Path) -> std::path::PathBuf {
    repo.join(".understand-anything")
        .join("knowledge-graph.json")
}

fn build_graph(repo: &Path, language: &str, full: bool) -> Result<Value> {
    let file_gate = file_gate::evaluate(repo, &file_gate::GateConfig::built_in())
        .map_err(|error| format!("evaluate graph file gate: {error}"))?;
    let file_gate_summary = json!({
        "schema": "code-intel-file-gate.v1",
        "candidates": file_gate.candidates,
        "included": file_gate.included.len(),
        "excluded": file_gate.candidates - file_gate.included.len() as i64,
        "by_gate": file_gate.by_gate(),
    });
    let mut files = Vec::new();
    collect_source_files(repo, &file_gate.included, &mut files)?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut symbols = Vec::new();

    let known_paths: HashSet<String> = files.iter().map(|file| file.rel.clone()).collect();

    for file in &files {
        nodes.push(json!({
            "id": file.rel,
            "kind": "file",
            "path": file.rel,
            "language": file.language,
            "lines": file.lines,
            "bytes": file.bytes
        }));

        edges.extend(extract_edges(file, &known_paths));
        symbols.extend(extract_symbols(file, full));
    }

    // Deliberately unstamped. This document is embedded verbatim in the
    // content-addressed `observed.evidence.payload`, so a wall-clock field
    // here gave every run a new payload digest for an unchanged tree. Nothing
    // read the stamp; the graph is a pure function of the scanned files, and
    // `.understand-anything/knowledge-graph.json` reports its own write time
    // through the filesystem.
    Ok(json!({
        "schema": "code-intel-understand-graph.v1",
        "provider": "code-intel-rust-graph",
        "repo": normalize_path(repo),
        "language": language,
        "full": full,
        "file_gate": file_gate_summary,
        "summary": {
            "files": nodes.len(),
            "edges": edges.len(),
            "symbols": symbols.len()
        },
        "nodes": nodes,
        "edges": edges,
        "symbols": symbols
    }))
}

fn collect_source_files(
    repo: &Path,
    included: &[String],
    files: &mut Vec<SourceFile>,
) -> Result<()> {
    for relative in included {
        let path = repo.join(relative);
        let Some(language) = language_from_path(&path) else {
            continue;
        };

        let bytes = fs::metadata(&path)?.len();
        if bytes > 1_500_000 {
            continue;
        }

        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };

        files.push(SourceFile {
            rel: normalize_path(Path::new(relative)),
            language,
            bytes,
            lines: text.lines().count(),
            text,
        });
    }
    Ok(())
}

fn language_from_path(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|value| value.to_str())? {
        "rs" => Some("rust"),
        "ps1" | "psm1" | "psd1" => Some("powershell"),
        "py" => Some("python"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "jsx" => Some("javascript-react"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "md" | "mdx" => Some("markdown"),
        "go" => Some("go"),
        "java" => Some("java"),
        "cs" => Some("csharp"),
        "cpp" | "cc" | "cxx" | "hpp" | "h" | "c" => Some("cpp"),
        "vue" => Some("vue"),
        "svelte" => Some("svelte"),
        _ => None,
    }
}

fn extract_edges(file: &SourceFile, known_paths: &HashSet<String>) -> Vec<Value> {
    let mut edges = Vec::new();
    let file_dir = Path::new(&file.rel)
        .parent()
        .unwrap_or_else(|| Path::new(""));

    for (line_index, raw_line) in file.text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }

        let candidates = edge_targets(file.language, line);
        for (kind, target) in candidates {
            let resolved = resolve_target(file_dir, &target, known_paths);
            edges.push(json!({
                "from": file.rel,
                "to": resolved.unwrap_or(target.clone()),
                "kind": kind,
                "rawTarget": target,
                "line": line_index + 1,
                "evidence": truncate(line, 220)
            }));
        }
    }

    edges
}

fn edge_targets(language: &str, line: &str) -> Vec<(&'static str, String)> {
    let mut targets = Vec::new();

    match language {
        "rust" => {
            if let Some(name) = line
                .strip_prefix("mod ")
                .and_then(|value| value.split(';').next())
            {
                targets.push(("module", name.trim().to_string()));
            }
            if let Some(name) = line
                .strip_prefix("pub mod ")
                .and_then(|value| value.split(';').next())
            {
                targets.push(("module", name.trim().to_string()));
            }
            if let Some(name) = line
                .strip_prefix("use ")
                .and_then(|value| value.split(';').next())
            {
                targets.push(("use", name.trim().to_string()));
            }
        }
        "python" => {
            if let Some(rest) = line.strip_prefix("import ") {
                targets.push((
                    "import",
                    rest.split_whitespace().next().unwrap_or(rest).to_string(),
                ));
            }
            if let Some(rest) = line.strip_prefix("from ") {
                targets.push((
                    "import",
                    rest.split_whitespace().next().unwrap_or(rest).to_string(),
                ));
            }
        }
        "javascript" | "javascript-react" | "typescript" => {
            if line.starts_with("import ") {
                if let Some(target) = quoted_tail(line) {
                    targets.push(("import", target));
                }
            }
            if line.contains("require(") {
                if let Some(target) = quoted_after(line, "require(") {
                    targets.push(("require", target));
                }
            }
        }
        "powershell" => {
            if line.starts_with(". ") || line.starts_with("& ") {
                if let Some(target) = first_path_like_token(line) {
                    targets.push(("invoke", target));
                }
            }
            if line.contains("Join-Path") {
                if let Some(target) = quoted_tail(line) {
                    targets.push(("path_reference", target));
                }
            }
        }
        _ => {
            if let Some(target) = quoted_tail(line) {
                if target.contains('/') || target.contains('\\') {
                    targets.push(("reference", target));
                }
            }
        }
    }

    targets
}

fn resolve_target(
    file_dir: &Path,
    raw_target: &str,
    known_paths: &HashSet<String>,
) -> Option<String> {
    let normalized = raw_target.replace('\\', "/");
    let mut candidates = Vec::new();

    if normalized.starts_with('.') {
        candidates.push(normalize_path(file_dir.join(&normalized)));
    } else {
        candidates.push(normalized.clone());
    }

    let extension_candidates = ["rs", "ps1", "py", "js", "ts", "tsx", "json", "md", "toml"];
    let mut expanded = Vec::new();
    for candidate in candidates {
        expanded.push(candidate.clone());
        for ext in extension_candidates {
            expanded.push(format!("{candidate}.{ext}"));
        }
        for ext in extension_candidates {
            expanded.push(format!("{candidate}/mod.{ext}"));
            expanded.push(format!("{candidate}/index.{ext}"));
        }
    }

    expanded
        .into_iter()
        .find(|candidate| known_paths.contains(candidate))
}

fn extract_symbols(file: &SourceFile, full: bool) -> Vec<Value> {
    let mut symbols = Vec::new();
    let max_symbols = if full { usize::MAX } else { 80 };

    for (line_index, raw_line) in file.text.lines().enumerate() {
        if symbols.len() >= max_symbols {
            break;
        }

        let line = raw_line.trim();
        let symbol = match file.language {
            "rust" => rust_symbol(line),
            "powershell" => powershell_symbol(line),
            "python" => python_symbol(line),
            "javascript" | "javascript-react" | "typescript" => js_symbol(line),
            _ => None,
        };

        if let Some((kind, name)) = symbol {
            symbols.push(json!({
                "file": file.rel,
                "kind": kind,
                "name": name,
                "line": line_index + 1
            }));
        }
    }

    symbols
}

fn rust_symbol(line: &str) -> Option<(&'static str, String)> {
    for prefix in ["pub async fn ", "async fn ", "pub fn ", "fn "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(("function", take_ident(rest)));
        }
    }
    for prefix in [
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(("type", take_ident(rest)));
        }
    }
    None
}

fn powershell_symbol(line: &str) -> Option<(&'static str, String)> {
    line.strip_prefix("function ")
        .map(|rest| ("function", take_ident(rest)))
}

fn python_symbol(line: &str) -> Option<(&'static str, String)> {
    if let Some(rest) = line.strip_prefix("def ") {
        return Some(("function", take_ident(rest)));
    }
    if let Some(rest) = line.strip_prefix("class ") {
        return Some(("type", take_ident(rest)));
    }
    None
}

fn js_symbol(line: &str) -> Option<(&'static str, String)> {
    for prefix in [
        "export async function ",
        "export function ",
        "async function ",
        "function ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(("function", take_ident(rest)));
        }
    }
    if let Some((name, _)) = line.split_once("=>") {
        let name = name
            .trim()
            .strip_prefix("export const ")
            .or_else(|| name.trim().strip_prefix("const "))
            .or_else(|| name.trim().strip_prefix("let "))
            .or_else(|| name.trim().strip_prefix("var "));
        if let Some(name) = name {
            return Some(("function", take_ident(name)));
        }
    }
    None
}

fn take_ident(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('&')
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .collect()
}

fn quoted_tail(line: &str) -> Option<String> {
    let first_single = line.rfind('\'');
    let first_double = line.rfind('"');
    let quote = match (first_single, first_double) {
        (Some(single), Some(double)) => {
            if single > double {
                '\''
            } else {
                '"'
            }
        }
        (Some(_), None) => '\'',
        (None, Some(_)) => '"',
        (None, None) => return None,
    };

    let before_end = line.rsplit_once(quote)?.0;
    let (_, target) = before_end.rsplit_once(quote)?;
    Some(target.to_string())
}

fn quoted_after(line: &str, marker: &str) -> Option<String> {
    let start = line.find(marker)? + marker.len();
    let tail = &line[start..];
    let quote = tail.chars().find(|ch| *ch == '\'' || *ch == '"')?;
    let after_quote = tail.split_once(quote)?.1;
    Some(after_quote.split_once(quote)?.0.to_string())
}

fn first_path_like_token(line: &str) -> Option<String> {
    line.split_whitespace()
        .skip(1)
        .map(|value| value.trim_matches('"').trim_matches('\''))
        .find(|value| value.contains(".ps1") || value.contains('/') || value.contains('\\'))
        .map(ToString::to_string)
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max)
        .last()
        .unwrap_or(0);
    format!("{}...", &value[..end])
}

fn normalize_path(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}
