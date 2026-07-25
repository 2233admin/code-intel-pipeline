use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let (request_path, artifact_root) = match parse_cli(raw) {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let request = match read_request(&request_path) {
        Ok(value) => value,
        Err(message) => return reject(&message),
    };
    match scan(&request, &artifact_root) {
        Ok(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            0
        }
        Err(message) => reject(&message),
    }
}

fn reject(message: &str) -> i32 {
    eprintln!("{message}");
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schema":"code-intel-repository-survival-scan-rejection.v1",
            "status":"rejected",
            "diagnostics":[message]
        }))
        .unwrap()
    );
    65
}

fn parse_cli(raw: &[String]) -> Result<(String, PathBuf), String> {
    let mut request = None;
    let mut artifact_root = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(flag, "--request" | "--artifact-root") {
            return Err(format!("unknown survival scan argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        match flag {
            "--request" if request.replace(value.clone()).is_some() => {
                return Err("duplicate survival scan argument: --request".into())
            }
            "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                return Err("duplicate survival scan argument: --artifact-root".into())
            }
            _ => {}
        }
        index += 2;
    }
    Ok((
        request.ok_or("survival scan requires --request")?,
        artifact_root.ok_or("survival scan requires --artifact-root")?,
    ))
}

fn read_request(path: &str) -> Result<Value, String> {
    let mut bytes = Vec::new();
    if path == "-" {
        io::stdin()
            .take(MAX_REQUEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read survival scan request: {error}"))?;
    } else {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("read survival scan request metadata: {error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_REQUEST_BYTES {
            return Err("survival scan request must be a bounded regular file".into());
        }
        bytes = fs::read(path).map_err(|error| format!("read survival scan request: {error}"))?;
    }
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("survival scan request exceeds size limit".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("survival scan request is not UTF-8: {error}"))?;
    crate::capability::reject_duplicate_json_keys(text)?;
    serde_json::from_str(text).map_err(|_| "invalid survival scan request JSON".into())
}

pub(crate) fn scan(request: &Value, artifact_root: &Path) -> Result<Value, String> {
    exact(
        request,
        &["schema", "snapshotIdentity", "inputs", "codenexusAdapter"],
        "survival scan request",
    )?;
    if request["schema"] != "code-intel-repository-survival-scan-request.v1" {
        return Err("survival scan request schema is invalid".into());
    }
    let snapshot_identity = request["snapshotIdentity"]
        .as_str()
        .filter(|value| digest(value))
        .ok_or("survival scan snapshot identity is invalid")?;
    let verified = crate::artifact_ref::verify_inputs(
        &request["inputs"],
        Some(artifact_root),
        snapshot_identity,
    )
    .map_err(|error| error.message().to_string())?;
    if verified.len() != 2 {
        return Err("survival scan requires exactly snapshot and inventory Artifact Refs".into());
    }
    let snapshot = verified
        .iter()
        .find(|item| item.artifact_type() == "repository.snapshot")
        .ok_or("survival scan repository snapshot input is missing")?;
    let inventory = verified
        .iter()
        .find(|item| item.artifact_type() == "inventory.files")
        .ok_or("survival scan inventory input is missing")?;
    let snapshot_value: Value = serde_json::from_slice(snapshot.bytes())
        .map_err(|_| "verified repository snapshot is invalid JSON")?;
    if snapshot_value["snapshot"]["identity"] != snapshot_identity {
        return Err("repository snapshot payload identity mismatch".into());
    }

    let adapter = &request["codenexusAdapter"];
    crate::codenexus_adapter::validate_adapter_result(adapter)?;
    if adapter["schema"] != "code-intel-codenexus-adapter-result.v1"
        || adapter["port"]["status"] != "unavailable"
        || adapter["port"]["failureKind"] != "provider_unavailable"
        || adapter["port"]["perceptionUsable"] != false
        || adapter["factPromotion"]["engineeringFacts"] != json!([])
        || adapter["evidence"]["request"]["observation"]["failure"]["kind"]
            != "provider_unavailable"
    {
        return Err("survival scan is only valid for unavailable CodeNexus evidence".into());
    }
    let admitted = crate::admissibility::validate_for_consumer(
        &adapter["evidence"]["request"],
        artifact_root,
    )?;
    crate::codenexus_adapter::validate_admitted_payload(admitted.payload(), adapter)?;
    if admitted.result()["domainVerdict"] != "unknown"
        || admitted.result()["engineeringFacts"] != json!([])
    {
        return Err("unavailable CodeNexus evidence must remain unknown without facts".into());
    }

    let paths = inventory_paths(inventory.bytes())?;
    let mut extensions = BTreeMap::<String, u64>::new();
    for path in &paths {
        if let Some(extension) = Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .filter(|value| !value.is_empty())
        {
            *extensions.entry(extension).or_default() += 1;
        }
    }
    let repository_kind = snapshot_value["repository"]["kind"].clone();
    let repo_identity = snapshot_value["snapshot"]["repoIdentity"].clone();
    let revision = snapshot_value["snapshot"]["head"].clone();
    let inventory_count = paths.len() as u64;
    Ok(json!({
        "schema":"code-intel-repository-survival-scan-result.v1",
        "status":"completed",
        "snapshotIdentity":snapshot_identity,
        "repository":{
            "kind":repository_kind,
            "identity":repo_identity,
            "revision":revision,
            "dirty":snapshot_value["dirtyOverlay"]["present"],
            "sourceSha256":snapshot.sha256()
        },
        "inventory":{
            "fileCount":inventory_count,
            "extensions":extensions,
            "sourceSha256":inventory.sha256()
        },
        "providerDiagnosis":{
            "providerId":adapter["port"]["provider"]["providerId"],
            "status":"provider_unavailable",
            "domainVerdict":"unknown"
        },
        "completeness":"reduced",
        "structuralVerdict":"unknown",
        "limitations":[
            "only repository identity and basic file inventory are available",
            "deeper structural perception requires an admitted provider result"
        ],
        "engineeringFacts":[
            {"kind":"repository_identity","value":repo_identity,"sourceSha256":snapshot.sha256()},
            {"kind":"repository_revision","value":revision,"sourceSha256":snapshot.sha256()},
            {"kind":"inventory_file_count","value":inventory_count,"sourceSha256":inventory.sha256()}
        ]
    }))
}

fn inventory_paths(bytes: &[u8]) -> Result<Vec<String>, String> {
    let delimiter = if bytes.contains(&0) { 0 } else { b'\n' };
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for raw in bytes.split(|byte| *byte == delimiter) {
        if raw.is_empty() {
            continue;
        }
        let path = std::str::from_utf8(raw)
            .map_err(|_| "verified inventory contains non-UTF-8 path")?
            .trim_end_matches('\r');
        if path.is_empty() || path.starts_with('/') || path.contains("..") || path.contains('\\') {
            return Err("verified inventory contains a non-portable path".into());
        }
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err("verified inventory contains duplicate paths".into());
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn exact(value: &Value, keys: &[&str], label: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{label} fields are invalid"));
    }
    Ok(())
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
