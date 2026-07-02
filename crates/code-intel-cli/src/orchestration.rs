use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) struct Options<'a> {
    pub(crate) action: &'a str,
    pub(crate) mode: &'a str,
    pub(crate) manifest: Option<&'a Path>,
    pub(crate) capability: Option<&'a str>,
    pub(crate) repo: Option<&'a Path>,
    pub(crate) json: bool,
}

pub(crate) fn run(options: &Options<'_>) -> Result<()> {
    let action = normalize_action(options.action)?;
    let mode = normalize_mode(options.mode)?;
    let manifest_path = resolve_manifest_path(options.manifest)?;
    let root = root_for_manifest(&manifest_path)?;
    let manifest = read_json(&manifest_path)?;
    let stages = array_values(&manifest, "stages");
    let integrations = array_values(&manifest, "integrations");

    let mut errors = Vec::new();
    let mut stage_ids = HashSet::new();
    let mut stage_order = HashMap::new();

    for stage in &stages {
        let id = string_field(stage, "id").unwrap_or_default();
        if id.trim().is_empty() {
            errors.push("stage id is empty".to_string());
            continue;
        }
        if !stage_ids.insert(id.clone()) {
            errors.push(format!("duplicate stage id: {id}"));
        }
        stage_order.insert(id, int_field(stage, "order"));
    }

    let mut integration_ids = HashSet::new();
    for integration in &integrations {
        let id = string_field(integration, "id").unwrap_or_default();
        let stage = string_field(integration, "stage").unwrap_or_default();
        let entrypoint = string_field(integration, "entrypoint").unwrap_or_default();
        let capabilities = string_array_field(integration, "capabilities");

        if id.trim().is_empty() {
            errors.push("integration id is empty".to_string());
            continue;
        }
        if !integration_ids.insert(id.clone()) {
            errors.push(format!("duplicate integration id: {id}"));
        }
        if !stage_ids.contains(&stage) {
            errors.push(format!(
                "integration {id} references unknown stage: {stage}"
            ));
        }
        if entrypoint.trim().is_empty() {
            errors.push(format!("integration {id} has no entrypoint"));
        } else if should_validate_entrypoint(&entrypoint) {
            let candidate = root.join(&entrypoint);
            if !candidate.is_file() {
                errors.push(format!("integration {id} entrypoint missing: {entrypoint}"));
            }
        }
        if capabilities.is_empty() {
            errors.push(format!("integration {id} exposes no capabilities"));
        }
    }

    let mut selected = integrations
        .iter()
        .filter(|integration| integration_matches_capability(integration, options.capability))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        let left_stage = string_field(left, "stage").unwrap_or_default();
        let right_stage = string_field(right, "stage").unwrap_or_default();
        let left_order = stage_order.get(&left_stage).copied().unwrap_or(i64::MAX);
        let right_order = stage_order.get(&right_stage).copied().unwrap_or(i64::MAX);
        left_order
            .cmp(&right_order)
            .then_with(|| string_field(left, "id").cmp(&string_field(right, "id")))
    });

    let plan = selected
        .iter()
        .map(|integration| plan_item(integration, options.repo, &mode))
        .collect::<Vec<_>>();

    let mut sorted_stages = stages.clone();
    sorted_stages.sort_by_key(|stage| int_field(stage, "order"));

    let ok = errors.is_empty();
    let out = serde_json::json!({
        "ok": ok,
        "action": action,
        "manifest": manifest_path,
        "policy": manifest.get("policy").cloned().unwrap_or(Value::Null),
        "errors": errors,
        "stages": sorted_stages,
        "integrations": if action == "Validate" { Vec::<Value>::new() } else { plan.clone() },
        "plan": if action == "Plan" { plan.clone() } else { Vec::<Value>::new() }
    });

    if options.json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_text(&action, &manifest_path, &out);
    }

    if ok {
        Ok(())
    } else {
        Err("orchestration validation failed".into())
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

fn resolve_manifest_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return absolute_file_path(path);
    }
    let relative = Path::new("orchestration").join("integrations.json");
    for start in manifest_search_starts()? {
        for ancestor in start.ancestors() {
            let candidate = ancestor.join(&relative);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("orchestration manifest missing: orchestration/integrations.json".into())
}

fn manifest_search_starts() -> Result<Vec<PathBuf>> {
    let mut starts = Vec::new();
    starts.push(env::current_dir()?);
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            starts.push(parent.to_path_buf());
        }
    }
    Ok(starts)
}

fn absolute_file_path(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()?.join(path)
        });
    }
    Err(format!("file does not exist: {}", path.display()).into())
}

fn root_for_manifest(manifest_path: &Path) -> Result<PathBuf> {
    let parent = manifest_path
        .parent()
        .ok_or("manifest path has no parent directory")?;
    if parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("orchestration"))
    {
        return parent
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "orchestration manifest has no repository root".into());
    }
    Ok(parent.to_path_buf())
}

fn normalize_action(value: &str) -> Result<String> {
    match value.to_ascii_lowercase().as_str() {
        "validate" => Ok("Validate".to_string()),
        "list" => Ok("List".to_string()),
        "plan" => Ok("Plan".to_string()),
        other => Err(format!("unsupported orchestration action: {other}").into()),
    }
}

fn normalize_mode(value: &str) -> Result<String> {
    match value.to_ascii_lowercase().as_str() {
        "lite" => Ok("lite".to_string()),
        "normal" => Ok("normal".to_string()),
        "full" => Ok("full".to_string()),
        other => Err(format!("unsupported orchestration mode: {other}").into()),
    }
}

fn should_validate_entrypoint(entrypoint: &str) -> bool {
    let lower = entrypoint.to_ascii_lowercase();
    [".ps1", ".py", ".toml", ".rs"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn array_values(value: &Value, key: &str) -> Vec<Value> {
    match value.get(key).and_then(Value::as_array) {
        Some(items) => items.clone(),
        None => Vec::new(),
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn integration_matches_capability(integration: &Value, capability: Option<&str>) -> bool {
    let Some(capability) = capability.filter(|value| !value.trim().is_empty()) else {
        return true;
    };
    string_field(integration, "id").as_deref() == Some(capability)
        || string_field(integration, "stage").as_deref() == Some(capability)
        || string_array_field(integration, "capabilities")
            .iter()
            .any(|item| item == capability)
}

fn plan_item(integration: &Value, repo: Option<&Path>, mode: &str) -> Value {
    let commands = integration
        .get("commands")
        .and_then(Value::as_object)
        .map(|items| {
            let mut expanded = serde_json::Map::new();
            for (name, value) in items {
                if let Some(template) = value.as_str() {
                    expanded.insert(
                        name.clone(),
                        Value::String(expand_command_template(template, repo, mode)),
                    );
                }
            }
            Value::Object(expanded)
        })
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    serde_json::json!({
        "id": string_field(integration, "id").unwrap_or_default(),
        "stage": string_field(integration, "stage").unwrap_or_default(),
        "kind": string_field(integration, "kind").unwrap_or_default(),
        "required": bool_field(integration, "required"),
        "entrypoint": string_field(integration, "entrypoint").unwrap_or_default(),
        "capabilities": string_array_field(integration, "capabilities"),
        "commands": commands,
        "artifactContract": string_array_field(integration, "artifactContract"),
        "extensionPoint": string_field(integration, "extensionPoint").unwrap_or_default()
    })
}

fn expand_command_template(template: &str, repo: Option<&Path>, mode: &str) -> String {
    let mut expanded = template.replace("<mode>", mode);
    if let Some(repo) = repo {
        expanded = expanded.replace("<repo-path>", &repo.display().to_string());
    }
    expanded
}

fn print_text(action: &str, manifest: &Path, result: &Value) {
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        println!("Code Intel orchestration: FAILED");
        if let Some(errors) = result.get("errors").and_then(Value::as_array) {
            for error in errors.iter().filter_map(Value::as_str) {
                println!("- {error}");
            }
        }
        return;
    }

    if action == "Validate" {
        let stages = result
            .get("stages")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let integrations = read_manifest_integration_count(result).unwrap_or(0);
        println!("Code Intel orchestration: OK");
        println!("Manifest: {}", manifest.display());
        println!("Stages: {stages}");
        println!("Integrations: {integrations}");
        return;
    }

    println!("Code Intel orchestration: {action}");
    if let Some(items) = result.get("integrations").and_then(Value::as_array) {
        for item in items {
            println!(
                "{}: {} [{}] entry={}",
                string_field(item, "stage").unwrap_or_default(),
                string_field(item, "id").unwrap_or_default(),
                string_field(item, "kind").unwrap_or_default(),
                string_field(item, "entrypoint").unwrap_or_default()
            );
            if action == "Plan" {
                if let Some(commands) = item.get("commands").and_then(Value::as_object) {
                    for (name, value) in commands {
                        if let Some(command) = value.as_str() {
                            println!("  {name}: {command}");
                        }
                    }
                }
            }
        }
    }
}

fn read_manifest_integration_count(result: &Value) -> Option<usize> {
    let manifest = result.get("manifest")?.as_str()?;
    let data = read_json(Path::new(manifest)).ok()?;
    data.get("integrations")
        .and_then(Value::as_array)
        .map(Vec::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        env::temp_dir().join(format!("code-intel-{name}-{stamp}"))
    }

    fn touch(path: &Path, text: &str) {
        fs::write(path, text).expect("fixture file should be writable");
    }

    fn orchestration_fixture_dir(name: &str) -> PathBuf {
        let dir = unique_temp_dir(name);
        fs::create_dir_all(dir.join("orchestration")).expect("fixture orchestration dir");
        fs::create_dir_all(dir.join("crates/code-intel-cli")).expect("fixture crate dir");
        touch(&dir.join("crates/code-intel-cli/Cargo.toml"), "[package]\n");
        dir
    }

    #[test]
    fn validates_manifest_and_rust_entrypoint() {
        let dir = orchestration_fixture_dir("orchestration-valid");
        let manifest = dir.join("orchestration/integrations.json");
        touch(
            &manifest,
            &json!({
                "schemaVersion": 1,
                "policy": {"name": "test"},
                "stages": [
                    {"id": "rust_runtime", "order": 10, "required": true}
                ],
                "integrations": [
                    {
                        "id": "runtime.code-intel",
                        "stage": "rust_runtime",
                        "kind": "internal-rust-binary",
                        "required": true,
                        "entrypoint": "crates/code-intel-cli/Cargo.toml",
                        "capabilities": ["orchestration"],
                        "commands": {
                            "validate": "target/debug/code-intel.exe orchestrate --action Validate --json",
                            "plan": "target/debug/code-intel.exe orchestrate --action Plan --repo <repo-path> --mode <mode> --json"
                        },
                        "artifactContract": ["integrations.json"]
                    }
                ]
            })
            .to_string(),
        );

        let options = Options {
            manifest: Some(&manifest),
            repo: Some(Path::new("D:/work/demo")),
            action: "Plan",
            mode: "normal",
            capability: Some("orchestration"),
            json: true,
        };

        run(&options).expect("valid orchestration manifest should pass");
    }

    #[test]
    fn fails_when_registered_entrypoint_is_missing() {
        let dir = orchestration_fixture_dir("orchestration-missing");
        let manifest = dir.join("orchestration/integrations.json");
        touch(
            &manifest,
            &json!({
                "schemaVersion": 1,
                "policy": {"name": "test"},
                "stages": [
                    {"id": "rust_runtime", "order": 10, "required": true}
                ],
                "integrations": [
                    {
                        "id": "runtime.missing",
                        "stage": "rust_runtime",
                        "kind": "internal-rust-binary",
                        "required": true,
                        "entrypoint": "crates/missing/Cargo.toml",
                        "capabilities": ["orchestration"],
                        "commands": {},
                        "artifactContract": []
                    }
                ]
            })
            .to_string(),
        );

        let options = Options {
            manifest: Some(&manifest),
            action: "Validate",
            mode: "normal",
            repo: None,
            capability: None,
            json: true,
        };

        let err = run(&options).expect_err("missing entrypoint should fail");
        assert!(err.to_string().contains("orchestration validation failed"));
    }
}
