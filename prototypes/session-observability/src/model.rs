use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub harness: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub event_count: usize,
    pub edited: u64,
    pub error_rate: f64,
    pub churn_files: u64,
    pub edits_after_last_verify: u64,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub path: String,
    pub touch: String,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub tool: String,
    pub action: String,
    pub targets: Vec<Target>,
    pub is_error: bool,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct Hotspot {
    pub path: String,
    pub max_complexity: u64,
    pub avg_complexity: f64,
    pub loc: u64,
    pub churn: u64,
    pub dirty: bool,
}

#[derive(Debug)]
pub struct Timeline {
    pub session: Session,
    pub stats: Stats,
    pub events: Vec<Event>,
    hotspots: HashMap<String, Hotspot>,
}

impl Timeline {
    pub fn load(trace_path: &Path, hotspots_path: &Path) -> Result<Self, String> {
        let trace = read_json(trace_path)?;
        if trace.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("unsupported trace schema: expected version 1".to_string());
        }

        let session_value = trace
            .get("session")
            .ok_or_else(|| "trace is missing session".to_string())?;
        let session = Session {
            id: string_field(session_value, "id"),
            harness: string_field(session_value, "harness"),
            cwd: string_field(session_value, "cwd"),
        };

        let events = trace
            .get("events")
            .and_then(Value::as_array)
            .ok_or_else(|| "trace is missing events".to_string())?
            .iter()
            .map(parse_event)
            .collect::<Vec<_>>();

        let stats_value = trace.get("stats").unwrap_or(&Value::Null);
        let stats = Stats {
            event_count: events.len(),
            edited: u64_field(stats_value, "edited"),
            error_rate: f64_field(stats_value, "errorRate"),
            churn_files: u64_field(stats_value, "churnFiles"),
            edits_after_last_verify: u64_field(stats_value, "editsAfterLastVerify"),
        };

        let hotspot_json = read_json(hotspots_path)?;
        let hotspot_files = hotspot_json
            .get("files")
            .or_else(|| hotspot_json.get("file_details"))
            .and_then(Value::as_array)
            .ok_or_else(|| "Sentrux artifact is missing files or file_details".to_string())?;
        let hotspots = hotspot_files
            .iter()
            .map(parse_hotspot)
            .filter(|item| !item.path.is_empty())
            .map(|item| (normalize_path(&item.path), item))
            .collect();

        Ok(Self {
            session,
            stats,
            events,
            hotspots,
        })
    }

    pub fn hotspot_for(&self, target_path: &str) -> Option<&Hotspot> {
        let normalized = normalize_path(target_path);
        if let Some(exact) = self.hotspots.get(&normalized) {
            return Some(exact);
        }

        self.hotspots
            .iter()
            .filter(|(candidate, _)| {
                normalized.ends_with(candidate.as_str()) || candidate.ends_with(&normalized)
            })
            .max_by_key(|(candidate, _)| candidate.len())
            .map(|(_, hotspot)| hotspot)
    }

    pub fn matched_target_count(&self) -> usize {
        self.events
            .iter()
            .flat_map(|event| &event.targets)
            .filter(|target| self.hotspot_for(&target.path).is_some())
            .count()
    }

    pub fn first_interesting_index(&self) -> usize {
        self.events
            .iter()
            .position(|event| {
                event
                    .targets
                    .iter()
                    .any(|target| self.hotspot_for(&target.path).is_some())
            })
            .or_else(|| {
                self.events
                    .iter()
                    .position(|event| event.is_error || event.action == "edit")
            })
            .unwrap_or(0)
    }

    pub fn redacted_summary(&self, summary: &str) -> String {
        let mut redacted = summary.replace('\n', " ").replace('\r', " ");
        if !self.session.cwd.is_empty() {
            redacted = replace_case_insensitive(&redacted, &self.session.cwd, "<repo>");
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            redacted = replace_case_insensitive(&redacted, &home.to_string_lossy(), "<home>");
        }
        truncate(&redacted, 180)
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn parse_event(value: &Value) -> Event {
    let targets = value
        .get("targets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|target| Target {
                    path: string_field(target, "path"),
                    touch: string_field(target, "touch"),
                })
                .collect()
        })
        .unwrap_or_default();

    Event {
        seq: u64_field(value, "seq"),
        tool: string_field(value, "tool"),
        action: string_field(value, "action"),
        targets,
        is_error: value
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        summary: string_field(value, "summary"),
    }
}

fn parse_hotspot(value: &Value) -> Hotspot {
    let git = value.get("git").unwrap_or(&Value::Null);
    Hotspot {
        path: string_field(value, "path"),
        max_complexity: u64_field_any(value, &["maxComplexity", "max_complexity"]),
        avg_complexity: f64_field_any(value, &["avgComplexity", "avg_complexity"]),
        loc: u64_field(value, "loc"),
        churn: u64_field(git, "churn"),
        dirty: git.get("dirty").and_then(Value::as_bool).unwrap_or(false),
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn u64_field(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn f64_field(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn u64_field_any(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn f64_field_any(value: &Value, keys: &[&str]) -> f64 {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_f64))
        .unwrap_or(0.0)
}

fn normalize_path(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .to_lowercase()
}

fn replace_case_insensitive(source: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return source.to_string();
    }
    let source_lower = source.to_lowercase();
    let needle_lower = needle.to_lowercase();
    if let Some(start) = source_lower.find(&needle_lower) {
        let end = start + needle.len();
        format!("{}{}{}", &source[..start], replacement, &source[end..])
    } else {
        source.to_string()
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}
