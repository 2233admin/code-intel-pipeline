use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{AdapterArtifact, AdapterError, AdapterOutput};
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::snapshot;

const SUPPORTED_HEURISTICS: [&str; 7] = [
    "powershell",
    "python",
    "javascript",
    "typescript",
    "rust",
    "go",
    "java",
];

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.len() != 1 || !options.contains_key("repoPath") {
        return Err(AdapterError::InvalidOptions(
            "evidence.native-code accepts only options.repoPath".into(),
        ));
    }
    let repo = options["repoPath"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    let inventory = match verified_inputs {
        [inventory] if inventory.artifact_type() == "inventory.files" => inventory,
        _ => {
            return Err(AdapterError::Contract(
                "evidence.native-code requires exactly one A03-verified inventory.files input"
                    .into(),
            ))
        }
    };
    let lease = snapshot::begin_consumption(repo, &request["snapshot"])
        .map_err(|message| AdapterError::Contract(format!("snapshot consumption: {message}")))?;
    let paths = inventory_paths(inventory.bytes())?;
    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut chunks = Vec::new();
    let mut mappings = Vec::new();
    let mut imports = Vec::new();
    let mut unsupported_files = Vec::new();

    for relative in paths {
        let full = safe_join(repo, &relative)?;
        if !full.is_file() {
            return Err(AdapterError::Contract(format!(
                "inventory path is not a readable file: {relative}"
            )));
        }
        let bytes = fs::read(&full).map_err(|error| {
            AdapterError::Io(format!("read native code evidence {relative}: {error}"))
        })?;
        let language = language(&relative);
        let hash = sha256_hex(&bytes);
        let content = match std::str::from_utf8(&bytes) {
            Ok(content) => content,
            Err(_) if !SUPPORTED_HEURISTICS.contains(&language) => {
                unsupported_files.push(relative.clone());
                files.push(json!({
                    "path":relative,
                    "language":language,
                    "bytes":bytes.len(),
                    "lines":0,
                    "textHash":hash,
                    "source":"native-minimal"
                }));
                chunks.push(json!({
                    "id":format!("{relative}#file"),
                    "file":relative,
                    "startLine":1,
                    "endLine":1,
                    "kind":"file",
                    "containsSymbols":[],
                    "textHash":hash,
                    "source":"native-minimal"
                }));
                continue;
            }
            Err(_) => {
                return Err(AdapterError::Contract(format!(
                    "supported source file is not UTF-8 text: {relative}"
                )))
            }
        };
        let lines = lines(content);
        if !SUPPORTED_HEURISTICS.contains(&language) {
            unsupported_files.push(relative.clone());
        }
        files.push(json!({
            "path":relative,
            "language":language,
            "bytes":bytes.len(),
            "lines":lines.len(),
            "textHash":hash,
            "source":"native-minimal"
        }));
        let file_symbols = extract_symbols(&relative, language, &lines);
        let chunk_id = format!("{relative}#file");
        chunks.push(json!({
            "id":chunk_id,
            "file":relative,
            "startLine":1,
            "endLine":lines.len().max(1),
            "kind":"file",
            "containsSymbols":file_symbols.iter().map(|symbol| symbol["id"].clone()).collect::<Vec<_>>(),
            "textHash":hash,
            "source":"native-minimal"
        }));
        for symbol in &file_symbols {
            mappings.push(json!({
                "symbolId":symbol["id"],
                "chunkId":chunk_id,
                "relation":"contained_by",
                "confidence":0.55
            }));
        }
        symbols.extend(file_symbols);
        imports.extend(extract_imports(&relative, language, &lines));
    }
    lease
        .verify_after(repo)
        .map_err(|message| AdapterError::Contract(format!("snapshot changed: {message}")))?;

    canonicalize_evidence_arrays(
        &mut files,
        &mut symbols,
        &mut chunks,
        &mut mappings,
        &mut imports,
        &mut unsupported_files,
    );
    let ranking = ranking(&files, &symbols, &imports);
    let coverage = json!({
        "schema":"code-evidence-coverage.v1",
        "producer":"native-minimal",
        "parserKind":"line-heuristic",
        "supportedHeuristics":SUPPORTED_HEURISTICS,
        "unsupportedFiles":unsupported_files,
        "symbolPrecision":"heuristic",
        "importPrecision":"heuristic",
        "relationshipPrecision":"unknown",
        "callGraph":"unknown",
        "effects":["repo_read","local_write"]
    });
    let coco_outcome = json!({
        "schema":"code-evidence-adapter-outcome.v1",
        "adapter":"cocoindex-code",
        "enabled":false,
        "required":false,
        "status":"skipped",
        "fatal":false,
        "reasonCode":"reviewed_deletion",
        "reason":"cocoindex-code is a reviewed retirement tombstone; legacy configuration cannot restore discovery or invocation.",
        "command":""
    });
    let scorecard = json!({
        "schema":"code-evidence-scorecard.v1",
        "status":"ok",
        "nativeMinimal":true,
        "adapters":[coco_outcome.clone()],
        "metrics":{
            "files":files.len(),
            "symbols":symbols.len(),
            "chunks":chunks.len(),
            "imports":imports.len(),
            "symbolContainmentRate":if symbols.is_empty() { Value::Null } else { json!(1.0) },
            "fallbackChunkRate":1.0
        }
    });
    let agent_views = render_agent_views(&ranking, &files);
    let documents = vec![
        (
            "code-evidence/merged/full/files.json",
            "code-evidence-files.v1",
            "code_evidence.files",
            json!({"schema":"code-evidence-files.v1","files":files}),
        ),
        (
            "code-evidence/merged/full/symbols.json",
            "code-evidence-symbols.v1",
            "code_evidence.symbols",
            json!({"schema":"code-evidence-symbols.v1","symbols":symbols}),
        ),
        (
            "code-evidence/merged/full/chunks.json",
            "code-evidence-chunks.v1",
            "code_evidence.chunks",
            json!({"schema":"code-evidence-chunks.v1","chunks":chunks}),
        ),
        (
            "code-evidence/merged/full/symbol-chunks.json",
            "code-evidence-symbol-chunks.v1",
            "code_evidence.symbol_chunks",
            json!({"schema":"code-evidence-symbol-chunks.v1","mappings":mappings}),
        ),
        (
            "code-evidence/merged/full/imports.json",
            "code-evidence-imports.v1",
            "code_evidence.imports",
            json!({"schema":"code-evidence-imports.v1","imports":imports}),
        ),
        (
            "code-evidence/merged/scorecard.json",
            "code-evidence-scorecard.v1",
            "code_evidence.scorecard",
            scorecard,
        ),
        (
            "code-evidence/coverage.json",
            "code-evidence-coverage.v1",
            "code_evidence.coverage",
            coverage,
        ),
        (
            "code-evidence/merged/agent/ranking.json",
            "agent-code-slice-ranking.v1",
            "code_evidence.agent_slice",
            ranking,
        ),
    ];
    let mut artifacts = Vec::new();
    for (relative_path, artifact_schema, artifact_type, document) in documents {
        let bytes = serde_json::to_vec(&document).map_err(|error| {
            AdapterError::Internal(format!("serialize {artifact_schema}: {error}"))
        })?;
        publish(out, relative_path, &bytes)?;
        artifacts.push(AdapterArtifact {
            artifact_schema: artifact_schema.into(),
            artifact_type: artifact_type.into(),
            relative_path: relative_path.into(),
            bytes,
        });
    }
    let coco_bytes = serde_json::to_vec(&coco_outcome)
        .map_err(|error| AdapterError::Internal(format!("serialize cocoindex outcome: {error}")))?;
    publish(
        out,
        "code-evidence/adapters/cocoindex-code/outcome.json",
        &coco_bytes,
    )?;
    for (relative, content) in agent_views {
        publish(out, relative, content.as_bytes())?;
    }
    Ok(AdapterOutput {
        artifacts,
        observed_effects: vec!["repo_read".into(), "local_write".into()],
        domain_verdict: crate::capability_inventory::AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn canonicalize_evidence_arrays(
    files: &mut [Value],
    symbols: &mut [Value],
    chunks: &mut [Value],
    mappings: &mut [Value],
    imports: &mut [Value],
    unsupported_files: &mut [String],
) {
    files.sort_by(|left, right| string_field(left, "path").cmp(string_field(right, "path")));
    symbols.sort_by(|left, right| {
        string_field(left, "file")
            .cmp(string_field(right, "file"))
            .then_with(|| u64_field(left, "startLine").cmp(&u64_field(right, "startLine")))
            .then_with(|| string_field(left, "kind").cmp(string_field(right, "kind")))
            .then_with(|| string_field(left, "name").cmp(string_field(right, "name")))
    });
    chunks.sort_by(|left, right| {
        string_field(left, "file")
            .cmp(string_field(right, "file"))
            .then_with(|| u64_field(left, "startLine").cmp(&u64_field(right, "startLine")))
            .then_with(|| u64_field(left, "endLine").cmp(&u64_field(right, "endLine")))
            .then_with(|| string_field(left, "id").cmp(string_field(right, "id")))
    });
    mappings.sort_by(|left, right| {
        string_field(left, "symbolId")
            .cmp(string_field(right, "symbolId"))
            .then_with(|| string_field(left, "chunkId").cmp(string_field(right, "chunkId")))
    });
    imports.sort_by(|left, right| {
        string_field(left, "file")
            .cmp(string_field(right, "file"))
            .then_with(|| u64_field(left, "line").cmp(&u64_field(right, "line")))
            .then_with(|| string_field(left, "target").cmp(string_field(right, "target")))
    });
    unsupported_files.sort();
}

fn string_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("")
}

fn u64_field(value: &Value, field: &str) -> u64 {
    value[field].as_u64().unwrap_or_default()
}

fn render_agent_views(ranking: &Value, files: &[Value]) -> Vec<(&'static str, String)> {
    let ranked_lines = ranking["files"]
        .as_array()
        .expect("ranking files")
        .iter()
        .take(20)
        .map(|file| {
            let reasons = file["reasons"]
                .as_array()
                .expect("ranking reasons")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "- {} score={} reasons={reasons}",
                file["path"].as_str().expect("ranking path"),
                file["score"].as_u64().expect("ranking score")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let entrypoints = file_lines(files, entrypoint, 20);
    let tests = file_lines(files, test_file, 30);
    vec![
        (
            "code-evidence/merged/agent/index.md",
            "# Agent Code Map\n\n## Status\n- Code Evidence Layer: ok\n- Native minimal layer: enabled\n- Parser coverage: line-heuristic\n- Call graph precision: unknown\n- Ranking: [ranking.json](ranking.json)\n- Native retrieval slice: [native-retrieval](slices/native-retrieval.md)\n\n## Full Dumps\n- [files](../full/files.json)\n- [symbols](../full/symbols.json)\n- [chunks](../full/chunks.json)\n- [symbol chunks](../full/symbol-chunks.json)\n- [imports](../full/imports.json)\n".into(),
        ),
        (
            "code-evidence/merged/agent/slices/native-retrieval.md",
            format!("# Native Retrieval Slice\n\n- Strategy: native-evidence-default\n- Source: Code Evidence files/symbols/imports only\n\n## Ranked Files\n{ranked_lines}\n"),
        ),
        (
            "code-evidence/merged/agent/slices/entrypoints.md",
            format!("# Entrypoints\n\n{entrypoints}\n"),
        ),
        (
            "code-evidence/merged/agent/slices/tests.md",
            format!("# Tests\n\n{tests}\n"),
        ),
        (
            "code-evidence/merged/agent/slices/risk-hotspots.md",
            "# Risk Hotspots\n\n- Native minimal layer does not calculate complexity.\n- Treat file-sized chunks as fallback evidence.\n- Call graph and cross-file relationship precision are unknown.\n".into(),
        ),
    ]
}

fn file_lines(files: &[Value], predicate: fn(&str) -> bool, limit: usize) -> String {
    files
        .iter()
        .filter(|file| predicate(file["path"].as_str().expect("file path")))
        .take(limit)
        .map(|file| {
            format!(
                "- {} ({})",
                file["path"].as_str().expect("file path"),
                file["language"].as_str().expect("file language")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn inventory_paths(bytes: &[u8]) -> Result<Vec<String>, AdapterError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| AdapterError::Contract("inventory.files must be UTF-8 paths".into()))?;
    let mut paths = text
        .split(['\0', '\n'])
        .map(|path| path.trim_end_matches('\r').replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    // File traversal order is not semantic and `rg --files` may vary with filesystem enumeration.
    // Publish one portable canonical order instead of preserving an incidental legacy traversal.
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn safe_join(repo: &Path, relative: &str) -> Result<PathBuf, AdapterError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(AdapterError::Contract(format!(
            "inventory path escapes repository: {relative}"
        )));
    }
    Ok(repo.join(path))
}

fn lines(content: &str) -> Vec<&str> {
    if content.is_empty() {
        Vec::new()
    } else {
        content
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .collect()
    }
}

fn language(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "ps1" | "psm1" => "powershell",
        "py" => "python",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "cs" => "csharp",
        _ => "text",
    }
}

fn extract_symbols(path: &str, language: &str, lines: &[&str]) -> Vec<Value> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            symbol_candidate(language, line).map(|(kind, name)| {
                json!({
                    "id":format!("{path}#{kind}:{name}"),
                    "kind":kind,
                    "name":name,
                    "file":path,
                    "startLine":index + 1,
                    "endLine":index + 1,
                    "language":language,
                    "confidence":0.55,
                    "source":"native-minimal"
                })
            })
        })
        .collect()
}

fn symbol_candidate<'a>(language: &str, line: &'a str) -> Option<(&'static str, &'a str)> {
    let line = line.trim_start();
    match language {
        "powershell" => word_after_ci(line, "function ").map(|name| ("function", name)),
        "python" => word_after(line, "def ")
            .map(|name| ("function", name))
            .or_else(|| word_after(line, "class ").map(|name| ("class", name))),
        "javascript" | "typescript" => {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let line = line.strip_prefix("async ").unwrap_or(line);
            word_after(line, "function ")
                .map(|name| ("function", name))
                .or_else(|| word_after(line, "class ").map(|name| ("class", name)))
                .or_else(|| word_after(line, "interface ").map(|name| ("interface", name)))
                .or_else(|| arrow_name(line).map(|name| ("function", name)))
        }
        "rust" => {
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let line = line.strip_prefix("async ").unwrap_or(line);
            word_after(line, "fn ").map(|name| ("function", name))
        }
        "go" => {
            let rest = line.strip_prefix("func ")?;
            let rest = if rest.starts_with('(') {
                rest.split_once(')')?.1.trim_start()
            } else {
                rest
            };
            identifier(rest).map(|name| ("function", name))
        }
        "java" => {
            let mut rest = line;
            for prefix in ["public ", "private ", "protected "] {
                rest = rest.strip_prefix(prefix).unwrap_or(rest);
            }
            word_after(rest, "class ")
                .map(|name| ("class", name))
                .or_else(|| word_after(rest, "interface ").map(|name| ("interface", name)))
                .or_else(|| word_after(rest, "enum ").map(|name| ("enum", name)))
        }
        _ => None,
    }
}

fn word_after<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    identifier(line.strip_prefix(prefix)?)
}

fn word_after_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.get(..prefix.len())
        .filter(|actual| actual.eq_ignore_ascii_case(prefix))?;
    identifier(&line[prefix.len()..])
}

fn identifier(text: &str) -> Option<&str> {
    let end = text
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '$' | '-' | ':')
        })
        .map(|(index, character)| index + character.len_utf8())
        .last()?;
    Some(&text[..end])
}

fn arrow_name(line: &str) -> Option<&str> {
    let line = ["const ", "let ", "var "]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))?;
    let (name, value) = line.split_once('=')?;
    value.contains("=>").then(|| name.trim()).filter(|name| {
        !name.is_empty()
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
            })
    })
}

fn extract_imports(path: &str, language: &str, lines: &[&str]) -> Vec<Value> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            import_target(language, line).map(|target| {
                json!({
                    "file":path,
                    "line":index + 1,
                    "target":target,
                    "language":language,
                    "confidence":0.6,
                    "source":"native-minimal"
                })
            })
        })
        .collect()
}

fn import_target<'a>(language: &str, line: &'a str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    if matches!(language, "javascript" | "typescript") {
        if let Some(target) = quoted_after(trimmed, "from") {
            return Some(target);
        }
        if let Some(start) = trimmed.find("require(") {
            return first_quoted(&trimmed[start + "require(".len()..]);
        }
    }
    if language == "python" {
        if let Some(rest) = trimmed.strip_prefix("from ") {
            return rest.split_ascii_whitespace().next();
        }
        if let Some(rest) = trimmed.strip_prefix("import ") {
            return rest.split_ascii_whitespace().next();
        }
    }
    if language == "rust" {
        return trimmed
            .strip_prefix("use ")
            .and_then(|rest| rest.strip_suffix(';'))
            .map(str::trim);
    }
    if language == "go" {
        return trimmed.strip_prefix("import ").and_then(first_quoted);
    }
    if let Some(rest) = trimmed.strip_prefix("#include") {
        let rest = rest.trim_start();
        if let Some(value) = first_quoted(rest) {
            return Some(value);
        }
        return rest.strip_prefix('<')?.split_once('>').map(|pair| pair.0);
    }
    None
}

fn quoted_after<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let start = line.find(token)? + token.len();
    first_quoted(&line[start..])
}

fn first_quoted(text: &str) -> Option<&str> {
    let (start, quote) = text
        .char_indices()
        .find(|(_, character)| matches!(character, '\'' | '"'))?;
    let rest = &text[start + quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

fn ranking(files: &[Value], symbols: &[Value], imports: &[Value]) -> Value {
    let mut symbols_by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut imports_by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for symbol in symbols {
        symbols_by_file
            .entry(symbol["file"].as_str().unwrap())
            .or_default()
            .push(symbol["name"].as_str().unwrap());
    }
    for import in imports {
        imports_by_file
            .entry(import["file"].as_str().unwrap())
            .or_default()
            .push(import["target"].as_str().unwrap());
    }
    let ranked = files
        .iter()
        .map(|file| {
            let path = file["path"].as_str().unwrap();
            let mut score = 0u64;
            let mut reasons = Vec::new();
            if entrypoint(path) {
                reasons.push("entrypoint");
                score += 40;
            }
            if test_file(path) {
                reasons.push("test");
                score += 35;
            }
            let file_symbols = symbols_by_file.get(path).cloned().unwrap_or_default();
            let file_imports = imports_by_file.get(path).cloned().unwrap_or_default();
            if !file_symbols.is_empty() {
                reasons.push("symbols");
                score += (5 * file_symbols.len() as u64).min(20);
            }
            if !file_imports.is_empty() {
                reasons.push("imports");
                score += (5 * file_imports.len() as u64).min(15);
            }
            if score == 0 {
                reasons.push("inventory");
                score = 1;
            }
            json!({
                "path":path,
                "language":file["language"],
                "score":score,
                "reasons":reasons,
                "symbols":legacy_collapsed_strings(&file_symbols),
                "imports":legacy_collapsed_strings(&file_imports)
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema":"agent-code-slice-ranking.v1",
        "strategy":"native-evidence-default",
        "files":ranked
    })
}

fn legacy_collapsed_strings(values: &[&str]) -> Value {
    match values {
        [] => Value::Null,
        [value] => Value::String((*value).to_string()),
        values => Value::Array(
            values
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    }
}

fn entrypoint(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    ["index.", "main.", "app.", "server.", "cli."]
        .iter()
        .any(|prefix| file.starts_with(prefix))
}

fn test_file(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.contains("test.")
        || file.contains("spec.")
        || path.starts_with("test/")
        || path.starts_with("tests/")
        || path.starts_with("spec/")
        || path.contains("/test/")
        || path.contains("/tests/")
        || path.contains("/spec/")
}

fn publish(out: &Path, relative: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    let path = out.join(relative);
    let parent = path.parent().ok_or_else(|| {
        AdapterError::Internal(format!("artifact has no parent directory: {relative}"))
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| AdapterError::Io(format!("create artifact directory: {error}")))?;
    if path.exists() {
        return Err(AdapterError::Io(format!(
            "refusing to overwrite native code evidence artifact: {relative}"
        )));
    }
    fs::write(&path, bytes).map_err(|error| AdapterError::Io(format!("write {relative}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_toolchain_digest_is_the_declared_source_sha256() {
        let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("orchestration/integrations.json");
        let manifest: Value = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
        let integration = manifest["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "evidence.native-code")
            .unwrap();
        assert_eq!(
            integration["toolchainDigestEvidence"],
            json!({
                "algorithm":"sha256",
                "inputs":["crates/code-intel-cli/src/native_code_evidence.rs"]
            })
        );
        let declared = integration["capabilityDeclaration"]["implementation"]["toolchainDigests"]
            [0]
        .as_str()
        .unwrap();
        assert_eq!(
            declared,
            sha256_hex(include_bytes!("native_code_evidence.rs"))
        );
    }
}
