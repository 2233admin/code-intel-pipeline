use std::collections::HashMap;

use serde_json::{json, Value};

use super::CommittedEvidence;

const MAX_ITEMS: usize = 20;
type Records = HashMap<String, Option<Value>>;
type SymbolRecords = HashMap<String, HashMap<String, HashMap<u64, Option<Value>>>>;
type Sources = HashMap<String, HashMap<String, HashMap<String, Option<Claims>>>>;

enum Claims {
    Files(Records),
    Symbols(SymbolRecords),
}

/// Projects only authenticated, same-run publication evidence. Neither repository
/// freshness nor the report's aggregate counts can establish an anchor's state.
pub(super) struct NavigationEvidence<'a> {
    report_ref: Option<&'a Value>,
    sources: Sources,
    unavailable_reason: Option<&'static str>,
}

impl<'a> NavigationEvidence<'a> {
    pub(super) fn new(evidence: &'a CommittedEvidence) -> Self {
        let mut reports = evidence
            .refs
            .iter()
            .zip(&evidence.verified)
            .filter(|(artifact, _)| {
                artifact["artifactSchema"] == "code-intel-anchor-verification.v1"
                    && artifact["type"] == "verification.anchors"
            });
        let Some((report_ref, verified)) = reports.next() else {
            return Self {
                report_ref: None,
                sources: HashMap::new(),
                unavailable_reason: Some(
                    "this run has no publication-time anchor verification report",
                ),
            };
        };
        if reports.next().is_some() {
            return Self {
                report_ref: None,
                sources: HashMap::new(),
                unavailable_reason: Some("this run has conflicting anchor verification reports"),
            };
        }
        let mut report: Value = serde_json::from_slice(verified.bytes())
            .expect("admitted anchor verification report is JSON");
        let mut sources: Sources = HashMap::new();
        // Move parsed strings/rows into lookup tables; never copy verified bytes
        // or repeatedly scan the report for each returned claim.
        for mut source in take_array(&mut report["sources"]) {
            let artifact_type = take_string(&mut source["artifactType"]);
            let artifact_path = take_string(&mut source["artifactPath"]);
            let anchor_kind = take_string(&mut source["anchorKind"]);
            let anchors = take_array(&mut source["anchors"]);
            let claims = if anchor_kind == "file" {
                let mut rows = HashMap::new();
                for mut anchor in anchors {
                    let path = take_string(&mut anchor["path"]);
                    anchor
                        .as_object_mut()
                        .expect("admitted anchor object")
                        .remove("path");
                    insert_unique(&mut rows, path, anchor);
                }
                Claims::Files(rows)
            } else {
                let mut rows: SymbolRecords = HashMap::new();
                for mut anchor in anchors {
                    let file = take_string(&mut anchor["file"]);
                    let name = take_string(&mut anchor["name"]);
                    let line = anchor["claimedLine"]
                        .as_u64()
                        .expect("admitted claimed line");
                    let object = anchor.as_object_mut().expect("admitted anchor object");
                    for key in ["file", "name", "claimedLine"] {
                        object.remove(key);
                    }
                    insert_unique(
                        rows.entry(file).or_default().entry(name).or_default(),
                        line,
                        anchor,
                    );
                }
                Claims::Symbols(rows)
            };
            insert_unique(
                sources
                    .entry(artifact_type)
                    .or_default()
                    .entry(artifact_path)
                    .or_default(),
                anchor_kind,
                claims,
            );
        }
        Self {
            report_ref: Some(report_ref),
            sources,
            unavailable_reason: None,
        }
    }

    pub(super) fn for_artifact(&self, artifact: &Value, bytes: &[u8]) -> Value {
        let kind = artifact["type"].as_str().expect("admitted artifact type");
        let schema = artifact["artifactSchema"]
            .as_str()
            .expect("admitted artifact schema");
        let anchor_kind =
            match (schema, kind) {
                ("agent-code-slice-ranking.v1", "code_evidence.agent_slice")
                | ("code-intel-surgery-plan.v1", "diagnosis.surgery-plan") => "file",
                ("code-evidence-symbols.v1", "code_evidence.symbols") => "symbol",
                _ => return self.projection(
                    "not_applicable",
                    Vec::new(),
                    false,
                    Some("this artifact contract has no supported publication-time anchor claims"),
                ),
            };
        let source: Value =
            serde_json::from_slice(bytes).expect("admitted source artifact is JSON");
        let path = artifact["path"].as_str().expect("admitted artifact path");
        let association = self
            .sources
            .get(kind)
            .and_then(|paths| paths.get(path))
            .and_then(|kinds| kinds.get(anchor_kind));
        let reason = self.unavailable_reason.or(match association {
            None => Some(
                "the report has no source with this exact artifact type, path, and anchor kind",
            ),
            Some(None) => Some("the report has conflicting duplicate source associations"),
            Some(Some(_)) => None,
        });
        let claims = association.and_then(Option::as_ref);
        let capacity = match kind {
            "code_evidence.agent_slice" => source["files"].as_array().map_or(0, Vec::len),
            "code_evidence.symbols" => source["symbols"].as_array().map_or(0, Vec::len),
            _ => usize::from(source["primary_target"]["file"].is_string()),
        };
        let mut items = Vec::with_capacity(capacity.min(MAX_ITEMS));
        let mut truncated = false;
        let mut add = |path: &str, name: Option<&str>, line: Option<u64>| {
            if items.len() == MAX_ITEMS {
                truncated = true;
                return false;
            }
            let row = match claims {
                Some(Claims::Files(rows)) => rows.get(path),
                Some(Claims::Symbols(rows)) => name
                    .and_then(|name| rows.get(path)?.get(name))
                    .and_then(|lines| lines.get(&line?)),
                None => None,
            };
            let row_reason = reason.or(match row {
                None => Some("the report does not assess this exact source claim"),
                Some(None) => Some("the report has conflicting duplicate records for this claim"),
                Some(Some(_)) => None,
            });
            items.push(project_item(
                path,
                name,
                line,
                row.and_then(Option::as_ref),
                row_reason,
            ));
            true
        };
        match kind {
            "code_evidence.agent_slice" => {
                for file in source["files"].as_array().expect("admitted ranking files") {
                    if let Some(path) = file["path"].as_str() {
                        if !add(path, None, None) {
                            break;
                        }
                    }
                }
            }
            "code_evidence.symbols" => {
                for symbol in source["symbols"]
                    .as_array()
                    .expect("admitted source symbols")
                {
                    if let Some(path) = symbol["file"].as_str() {
                        if !add(
                            path,
                            symbol["name"].as_str(),
                            symbol["startLine"].as_u64().filter(|line| *line > 0),
                        ) {
                            break;
                        }
                    }
                }
            }
            _ => {
                if let Some(path) = source["primary_target"]["file"].as_str() {
                    add(path, None, None);
                }
            }
        }
        if items.is_empty() {
            return self.projection(
                "not_applicable",
                items,
                false,
                Some("this artifact makes no supported navigation claims"),
            );
        }
        self.projection(
            if reason.is_some() {
                "unavailable"
            } else {
                "available"
            },
            items,
            truncated,
            reason,
        )
    }

    fn projection(
        &self,
        status: &str,
        items: Vec<Value>,
        truncated: bool,
        reason: Option<&str>,
    ) -> Value {
        json!({
            "status":status,"basis":"publication_time","currentValidity":"not_assessed",
            "reportRef":self.report_ref,"items":items,"itemsTruncated":truncated,"reason":reason,
        })
    }
}

fn insert_unique<K: std::hash::Hash + Eq, V>(rows: &mut HashMap<K, Option<V>>, key: K, value: V) {
    use std::collections::hash_map::Entry;
    match rows.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(Some(value));
        }
        Entry::Occupied(mut entry) => {
            entry.insert(None);
        }
    }
}

fn take_string(value: &mut Value) -> String {
    match value.take() {
        Value::String(value) => value,
        _ => unreachable!("admitted report string"),
    }
}

fn take_array(value: &mut Value) -> Vec<Value> {
    match value.take() {
        Value::Array(value) => value,
        _ => unreachable!("admitted report array"),
    }
}

fn project_item(
    path: &str,
    name: Option<&str>,
    line: Option<u64>,
    row: Option<&Value>,
    unknown: Option<&str>,
) -> Value {
    let mut state = "not_assessed";
    let mut resolved_line = None;
    let mut reason = unknown;
    let mut target = Value::Null;
    if unknown.is_none() {
        if let Some(row) = row {
            let fields = row.as_object().expect("admitted anchor record");
            match row["state"].as_str() {
                Some("verified") if fields.len() == 1 => {
                    state = "verified";
                    target = json!({"path":path,"line":line});
                }
                Some("approximate") if name.is_some() && fields.len() == 2 => {
                    if let Some(moved) = row["resolvedLine"].as_u64().filter(|line| *line > 0) {
                        state = "approximate";
                        resolved_line = Some(moved);
                        target = json!({"path":path,"line":moved});
                    }
                }
                Some("dropped") if fields.len() == 2 => {
                    state = "dropped";
                    reason = row["reason"].as_str();
                }
                _ => {}
            }
            if state == "not_assessed" {
                reason = Some("the recorded state cannot establish a supported navigation target");
            }
        }
    }
    json!({"path":path,"name":name,"claimedLine":line,"state":state,
        "resolvedLine":resolved_line,"navigationTarget":target,"reason":reason})
}
