use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

pub(crate) fn resolve(request: &Value) -> Result<Value, String> {
    exact(
        request,
        &[
            "schema",
            "targetFile",
            "expectedSnapshotIdentity",
            "policy",
            "document",
        ],
        "file boundary request",
    )?;
    if request["schema"] != "code-intel-file-boundary-request.v1"
        || !digest(&request["expectedSnapshotIdentity"])
    {
        return Err("file boundary request identity is invalid".to_string());
    }

    let target = normalize_relative_path(&request["targetFile"], "target file")?;
    let policy = &request["policy"];
    exact(
        policy,
        &["evaluatedAt", "maxAgeSeconds"],
        "file boundary freshness policy",
    )?;
    let evaluated_at = policy["evaluatedAt"]
        .as_u64()
        .ok_or_else(|| "file boundary evaluatedAt is invalid".to_string())?;
    let max_age_seconds = policy["maxAgeSeconds"]
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or_else(|| "file boundary maxAgeSeconds must be positive".to_string())?;

    let document = &request["document"];
    validate_document(document)?;
    let expected_snapshot = request["expectedSnapshotIdentity"].as_str().unwrap();
    let source_snapshot = document["snapshotIdentity"].as_str().unwrap();
    if expected_snapshot != source_snapshot {
        return Err("file boundary document snapshot mismatch".to_string());
    }
    let observed_at = document["observedAt"].as_u64().unwrap();
    if observed_at > evaluated_at {
        return Err("file boundary observation is from the future".to_string());
    }
    if evaluated_at - observed_at > max_age_seconds {
        return Err("file boundary observation is stale".to_string());
    }

    let mut by_match_key = BTreeMap::new();
    for entry in document["entries"].as_array().unwrap() {
        let normalized = validate_entry(entry)?;
        let match_key = normalized.to_ascii_lowercase();
        if by_match_key
            .insert(match_key, (normalized, entry))
            .is_some()
        {
            return Err("file boundary document contains duplicate or ambiguous paths".to_string());
        }
    }

    let source = &document["source"];
    let provenance = json!({
        "sourceKind":source["kind"],
        "sourcePath":normalize_relative_path(&source["path"], "boundary source path")?,
        "sourceSha256":source["sha256"],
        "observedAt":observed_at
    });
    let mut diagnostics = Vec::new();
    for construct in document["unsupportedConstructs"].as_array().unwrap() {
        diagnostics.push(json!({
            "code":"unsupported_construct",
            "reference":construct
        }));
    }

    let key = target.to_ascii_lowercase();
    let Some((matched_path, entry)) = by_match_key.get(&key) else {
        diagnostics.push(json!({
            "code":"no_matching_boundary",
            "reference":target
        }));
        return Ok(json!({
            "schema":"code-intel-file-boundary-result.v1",
            "status":"unknown",
            "resolution":"exact_path",
            "expectedSnapshotIdentity":expected_snapshot,
            "sourceSnapshotIdentity":source_snapshot,
            "normalizedTargetFile":target,
            "freshness":"current",
            "completeness":"unknown",
            "boundary":Value::Null,
            "provenance":provenance,
            "diagnostics":diagnostics
        }));
    };

    let completeness = if diagnostics.is_empty() {
        "complete"
    } else {
        "partial"
    };
    Ok(json!({
        "schema":"code-intel-file-boundary-result.v1",
        "status":"resolved",
        "resolution":"exact_path",
        "expectedSnapshotIdentity":expected_snapshot,
        "sourceSnapshotIdentity":source_snapshot,
        "normalizedTargetFile":target,
        "freshness":"current",
        "completeness":completeness,
        "boundary":{
            "path":matched_path,
            "role":entry["role"],
            "forbid":entry["forbid"],
            "gotcha":entry["gotcha"],
            "checks":entry["checks"]
        },
        "provenance":provenance,
        "diagnostics":diagnostics
    }))
}

fn validate_document(document: &Value) -> Result<(), String> {
    exact(
        document,
        &[
            "schema",
            "snapshotIdentity",
            "observedAt",
            "source",
            "entries",
            "unsupportedConstructs",
        ],
        "file boundary document",
    )?;
    if document["schema"] != "code-intel-file-boundary-document.v1"
        || !digest(&document["snapshotIdentity"])
        || document["observedAt"].as_u64().is_none()
        || document["entries"].as_array().is_none()
        || document["unsupportedConstructs"].as_array().is_none()
    {
        return Err("file boundary document identity or collections are invalid".to_string());
    }
    exact(
        &document["source"],
        &["kind", "path", "sha256"],
        "file boundary source",
    )?;
    if document["source"]["kind"] != "local_boundary_document"
        || !digest(&document["source"]["sha256"])
    {
        return Err("file boundary source identity is invalid".to_string());
    }
    normalize_relative_path(&document["source"]["path"], "boundary source path")?;

    let mut unsupported = BTreeSet::new();
    for item in document["unsupportedConstructs"].as_array().unwrap() {
        let value = nonempty_string(item, "unsupported construct")?;
        if !unsupported.insert(value) {
            return Err("file boundary unsupported constructs must be unique".to_string());
        }
    }
    Ok(())
}

fn validate_entry(entry: &Value) -> Result<String, String> {
    exact(
        entry,
        &["path", "role", "forbid", "gotcha", "checks"],
        "file boundary entry",
    )?;
    let path = normalize_relative_path(&entry["path"], "file boundary entry path")?;
    if path.contains('*') || path.contains('?') || path.contains('[') || path.contains(']') {
        return Err("file boundary selectors must be exact paths".to_string());
    }
    if !(entry["role"].is_null()
        || entry["role"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty()))
    {
        return Err("file boundary role is invalid".to_string());
    }

    let mut ids = BTreeSet::new();
    validate_rules(&entry["forbid"], "forbid", &mut ids)?;
    validate_rules(&entry["gotcha"], "gotcha", &mut ids)?;
    validate_checks(&entry["checks"], &mut ids)?;
    Ok(path)
}

fn validate_rules(value: &Value, label: &str, ids: &mut BTreeSet<String>) -> Result<(), String> {
    let rules = value
        .as_array()
        .ok_or_else(|| format!("file boundary {label} rules must be an array"))?;
    for rule in rules {
        exact(
            rule,
            &["id", "summary"],
            &format!("file boundary {label} rule"),
        )?;
        let id = validate_rule_id(&rule["id"])?;
        nonempty_string(&rule["summary"], &format!("file boundary {label} summary"))?;
        if !ids.insert(id) {
            return Err("file boundary rule IDs must be unique per entry".to_string());
        }
    }
    Ok(())
}

fn validate_checks(value: &Value, ids: &mut BTreeSet<String>) -> Result<(), String> {
    let checks = value
        .as_array()
        .ok_or_else(|| "file boundary checks must be an array".to_string())?;
    for check in checks {
        exact(check, &["id", "command"], "file boundary check")?;
        let id = validate_rule_id(&check["id"])?;
        nonempty_string(&check["command"], "file boundary check command")?;
        if !ids.insert(id) {
            return Err("file boundary rule IDs must be unique per entry".to_string());
        }
    }
    Ok(())
}

fn validate_rule_id(value: &Value) -> Result<String, String> {
    let id = nonempty_string(value, "file boundary rule ID")?;
    if id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err("file boundary rule ID is invalid".to_string());
    }
    Ok(id)
}

fn normalize_relative_path(value: &Value, label: &str) -> Result<String, String> {
    let raw = nonempty_string(value, label)?;
    if raw.contains('\0') || raw.starts_with('/') || raw.starts_with('\\') {
        return Err(format!("{label} must be a safe repository-relative path"));
    }
    let replaced = raw.replace('\\', "/");
    if replaced.len() >= 2 && replaced.as_bytes()[1] == b':' {
        return Err(format!("{label} must be a safe repository-relative path"));
    }
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(format!("{label} must not escape the repository")),
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    Ok(parts.join("/"))
}

fn nonempty_string(value: &Value, label: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} is invalid"))
}

fn exact(value: &Value, fields: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}
