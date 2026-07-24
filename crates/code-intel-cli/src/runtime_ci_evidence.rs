use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};

use serde_json::{json, Value};

const REQUEST_SCHEMA: &str = "code-intel-runtime-ci-ingest-request.v1";
const SOURCE_SCHEMA: &str = "code-intel-runtime-ci-observation.v1";
const SUMMARY_SCHEMA: &str = "code-intel-runtime-ci-summary.v1";
const MAX_ARTIFACT_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) fn parse_request_bytes(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "runtime/CI request must be UTF-8 JSON".to_string())?;
    reject_duplicate_json_keys(text)?;
    serde_json::from_str(text)
        .map_err(|error| format!("runtime/CI request is invalid JSON: {error}"))
}

pub(crate) fn ingest_request(artifact_root: &Path, request: &Value) -> Result<Value, String> {
    require_object_keys(
        request,
        &["schema", "expectedSnapshotIdentity", "artifact", "policy"],
        "request",
    )?;
    require_const(request, "schema", REQUEST_SCHEMA, "request")?;
    let expected_snapshot = digest_field(request, "expectedSnapshotIdentity", "request")?;
    let artifact = object_field(request, "artifact", "request")?;
    require_object_keys(artifact, &["path", "sha256"], "request.artifact")?;
    let relative = string_field(artifact, "path", "request.artifact")?;
    let expected_digest = digest_field(artifact, "sha256", "request.artifact")?;
    let policy = object_field(request, "policy", "request")?;
    require_object_keys(policy, &["evaluatedAt", "maxAgeSeconds"], "request.policy")?;
    let evaluated_at = integer_field(policy, "evaluatedAt", "request.policy")?;
    let max_age = positive_integer_field(policy, "maxAgeSeconds", "request.policy")?;
    let path = safe_join(artifact_root, relative)?;

    if !path.is_file() {
        return Ok(unknown_summary(
            expected_snapshot,
            "missing",
            "missing",
            "artifact_missing",
        ));
    }
    let canonical_root = fs::canonicalize(artifact_root)
        .map_err(|error| format!("resolve runtime/CI artifact root: {error}"))?;
    let canonical_path = fs::canonicalize(&path)
        .map_err(|error| format!("resolve runtime/CI artifact path: {error}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("runtime/CI artifact resolves outside the artifact root".into());
    }
    let length = fs::metadata(&canonical_path)
        .map_err(|error| format!("inspect runtime/CI artifact: {error}"))?
        .len();
    if length > MAX_ARTIFACT_BYTES {
        return Err(format!(
            "runtime/CI artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
        ));
    }
    let bytes =
        fs::read(&canonical_path).map_err(|error| format!("read runtime/CI artifact: {error}"))?;
    if sha256_hex(&bytes) != expected_digest {
        return Err("runtime/CI artifact digest mismatch".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "runtime/CI artifact must be UTF-8 JSON".to_string())?;
    reject_duplicate_json_keys(text)?;
    let source: Value = serde_json::from_str(text)
        .map_err(|error| format!("runtime/CI artifact is invalid JSON: {error}"))?;
    normalize(&source, expected_snapshot, evaluated_at, max_age)
}

pub(crate) fn normalize(
    source: &Value,
    expected_snapshot: &str,
    evaluated_at: u64,
    max_age_seconds: u64,
) -> Result<Value, String> {
    validate_source(source)?;
    if !is_digest(expected_snapshot) {
        return Err("expected snapshot identity must be a lowercase SHA-256 digest".into());
    }
    let source_snapshot = source["snapshotIdentity"].as_str().expect("validated");
    let completeness = source["completeness"].as_str().expect("validated");
    let observed_at = source["observedAt"].as_u64().expect("validated");
    let provider = source["provider"].clone();
    let provenance = source["provenance"].clone();
    let signals = source["signals"].clone();

    let (freshness, failure_kind) = if source_snapshot != expected_snapshot {
        ("snapshot_mismatch", "snapshot_mismatch")
    } else if observed_at > evaluated_at || evaluated_at - observed_at > max_age_seconds {
        ("stale", "stale")
    } else {
        ("current", "none")
    };
    if freshness != "current" {
        return Ok(json!({
            "schema":SUMMARY_SCHEMA,
            "admission":"rejected",
            "health":"unknown",
            "freshness":freshness,
            "completeness":completeness,
            "expectedSnapshotIdentity":expected_snapshot,
            "sourceSnapshotIdentity":source_snapshot,
            "provider":provider,
            "provenance":provenance,
            "observedAt":observed_at,
            "signals":signals,
            "facts":[],
            "failureKind":failure_kind
        }));
    }

    let statuses = [
        signals["tests"]["status"].as_str().expect("validated"),
        signals["build"]["status"].as_str().expect("validated"),
        signals["runtime"]["status"].as_str().expect("validated"),
    ];
    let observed_failure = statuses[0] == "failed"
        || statuses[1] == "failed"
        || matches!(statuses[2], "failed" | "degraded");
    let fully_positive = completeness == "complete"
        && statuses[0] == "passed"
        && statuses[1] == "passed"
        && statuses[2] == "healthy";
    let health = if observed_failure {
        "red"
    } else if fully_positive {
        "green"
    } else {
        "unknown"
    };
    let failure_kind = if observed_failure {
        "observed_failure"
    } else if fully_positive {
        "none"
    } else {
        "partial_coverage"
    };
    let facts = if health == "green" {
        vec![
            "tests_observed_passed",
            "build_observed_passed",
            "runtime_observed_healthy",
        ]
    } else if health == "red" {
        vec!["runtime_ci_observed_failure"]
    } else {
        Vec::new()
    };
    Ok(json!({
        "schema":SUMMARY_SCHEMA,
        "admission":"accepted",
        "health":health,
        "freshness":"current",
        "completeness":completeness,
        "expectedSnapshotIdentity":expected_snapshot,
        "sourceSnapshotIdentity":source_snapshot,
        "provider":provider,
        "provenance":provenance,
        "observedAt":observed_at,
        "signals":signals,
        "facts":facts,
        "failureKind":failure_kind
    }))
}

fn validate_source(source: &Value) -> Result<(), String> {
    require_object_keys(
        source,
        &[
            "schema",
            "provider",
            "provenance",
            "snapshotIdentity",
            "observedAt",
            "completeness",
            "signals",
        ],
        "observation",
    )?;
    require_const(source, "schema", SOURCE_SCHEMA, "observation")?;
    digest_field(source, "snapshotIdentity", "observation")?;
    integer_field(source, "observedAt", "observation")?;
    enum_field(
        source,
        "completeness",
        &["complete", "partial"],
        "observation",
    )?;

    let provider = object_field(source, "provider", "observation")?;
    require_object_keys(
        provider,
        &["id", "runId", "sourceRevision"],
        "observation.provider",
    )?;
    string_field(provider, "id", "observation.provider")?;
    string_field(provider, "runId", "observation.provider")?;
    string_field(provider, "sourceRevision", "observation.provider")?;

    let provenance = object_field(source, "provenance", "observation")?;
    require_object_keys(
        provenance,
        &[
            "collectorId",
            "collectorVersion",
            "collectionId",
            "collectedAt",
        ],
        "observation.provenance",
    )?;
    string_field(provenance, "collectorId", "observation.provenance")?;
    string_field(provenance, "collectorVersion", "observation.provenance")?;
    string_field(provenance, "collectionId", "observation.provenance")?;
    integer_field(provenance, "collectedAt", "observation.provenance")?;

    let signals = object_field(source, "signals", "observation")?;
    require_object_keys(
        signals,
        &["tests", "build", "runtime"],
        "observation.signals",
    )?;
    validate_signal(
        object_field(signals, "tests", "observation.signals")?,
        "observation.signals.tests",
        &["passed", "failed", "cancelled", "unknown"],
    )?;
    validate_signal(
        object_field(signals, "build", "observation.signals")?,
        "observation.signals.build",
        &["passed", "failed", "cancelled", "unknown"],
    )?;
    validate_signal(
        object_field(signals, "runtime", "observation.signals")?,
        "observation.signals.runtime",
        &["healthy", "degraded", "failed", "unknown"],
    )?;
    Ok(())
}

fn validate_signal(value: &Value, context: &str, statuses: &[&str]) -> Result<(), String> {
    require_object_keys(value, &["status", "observed", "summary"], context)?;
    enum_field(value, "status", statuses, context)?;
    let observed = value["observed"]
        .as_bool()
        .ok_or_else(|| format!("{context}.observed must be a boolean"))?;
    let status = value["status"].as_str().expect("validated");
    if !observed && status != "unknown" {
        return Err(format!(
            "{context} cannot claim {status} when observed is false"
        ));
    }
    string_field(value, "summary", context)?;
    Ok(())
}

fn unknown_summary(
    expected_snapshot: &str,
    freshness: &str,
    completeness: &str,
    failure: &str,
) -> Value {
    json!({
        "schema":SUMMARY_SCHEMA,
        "admission":"rejected",
        "health":"unknown",
        "freshness":freshness,
        "completeness":completeness,
        "expectedSnapshotIdentity":expected_snapshot,
        "sourceSnapshotIdentity":Value::Null,
        "provider":Value::Null,
        "provenance":Value::Null,
        "observedAt":Value::Null,
        "signals":{
            "tests":{"status":"unknown","observed":false,"summary":"not available"},
            "build":{"status":"unknown","observed":false,"summary":"not available"},
            "runtime":{"status":"unknown","observed":false,"summary":"not available"}
        },
        "facts":[],
        "failureKind":failure
    })
}

fn safe_join(root: &Path, relative: &str) -> Result<std::path::PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\0')
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("runtime/CI artifact path must be repository-relative without '..'".into());
    }
    Ok(root.join(path))
}

fn require_object_keys(value: &Value, expected: &[&str], context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{context} fields are invalid"));
    }
    Ok(())
}

fn require_const(value: &Value, field: &str, expected: &str, context: &str) -> Result<(), String> {
    if value[field].as_str() != Some(expected) {
        return Err(format!("{context}.{field} must be {expected}"));
    }
    Ok(())
}

fn object_field<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a Value, String> {
    value[field]
        .as_object()
        .map(|_| &value[field])
        .ok_or_else(|| format!("{context}.{field} must be an object"))
}

fn string_field<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{context}.{field} must be a non-empty string"))
}

fn digest_field<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a str, String> {
    let digest = string_field(value, field, context)?;
    if !is_digest(digest) {
        return Err(format!(
            "{context}.{field} must be a lowercase SHA-256 digest"
        ));
    }
    Ok(digest)
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn integer_field(value: &Value, field: &str, context: &str) -> Result<u64, String> {
    value[field]
        .as_u64()
        .ok_or_else(|| format!("{context}.{field} must be a non-negative integer"))
}

fn positive_integer_field(value: &Value, field: &str, context: &str) -> Result<u64, String> {
    integer_field(value, field, context).and_then(|number| {
        if number == 0 {
            Err(format!("{context}.{field} must be positive"))
        } else {
            Ok(number)
        }
    })
}

fn enum_field<'a>(
    value: &'a Value,
    field: &str,
    allowed: &[&str],
    context: &str,
) -> Result<&'a str, String> {
    let actual = string_field(value, field, context)?;
    if !allowed.contains(&actual) {
        return Err(format!("{context}.{field} has an unsupported value"));
    }
    Ok(actual)
}

#[cfg(not(test))]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    crate::capability::sha256_hex(bytes)
}

#[cfg(not(test))]
fn reject_duplicate_json_keys(text: &str) -> Result<(), String> {
    crate::capability::reject_duplicate_json_keys(text)
        .map_err(|error| format!("runtime/CI artifact {error}"))
}

#[cfg(test)]
fn reject_duplicate_json_keys(_text: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = bytes.to_vec();
    let bits = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
                .wrapping_add((a & b) ^ (a & c) ^ (b & c));
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value);
        }
    }
    h.iter().map(|value| format!("{value:08x}")).collect()
}
