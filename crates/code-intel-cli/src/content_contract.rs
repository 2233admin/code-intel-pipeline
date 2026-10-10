//! Self-contained content-contract primitives shared by the capability
//! framework and Artifact Ref verification. Keeping them in a leaf module
//! removes the artifact_ref <-> capability import cycle: capability
//! re-exports these names, artifact_ref includes this file directly.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

// Was 8 MiB (unrevisited default) until issue #383/#386: fixing #383's
// silent-Null bug (`capability_structured_data` reparsing the 8KB bounded
// preview instead of the full command output, see `sentrux_command.rs`)
// means the capability artifact's `outputs.structuredData` legitimately carries
// the full output (current envelope v2; #383's historical measurement used v1).
// The recorded `sentrux-capability-sentrux-dsm.json` was 9,073,500 bytes
// (~8.65 MiB), which this ceiling
// would previously never have observed because the #383 bug always zeroed
// that field out first. 24 MiB keeps ~1.5x headroom over
// `sentrux_command::MAX_COMMAND_EVIDENCE_BYTES` (16 MiB, #382's own raised
// raw-capture ceiling for the same growth) for this artifact's small JSON
// wrapper overhead, while remaining a blanket safety net (not a per-schema
// budget) for every other, much smaller document this scanner also guards.
pub(crate) const MAX_JSON_BYTES: usize = 24 * 1024 * 1024;
pub(crate) const MAX_JSON_DEPTH: usize = 128;

pub(crate) fn reject_duplicate_json_keys(text: &str) -> Result<(), String> {
    reject_duplicate_json_keys_within(text, MAX_JSON_BYTES)
}

/// Same duplicate-key/size/depth scan as [`reject_duplicate_json_keys`], but
/// bounded by an explicit `max_bytes` ceiling instead of the fixed
/// [`MAX_JSON_BYTES`] default. Callers whose Artifact Ref contract already
/// declares a larger `max_bytes` (enforced upstream by
/// `stable_artifact::read_beneath`) must pass that same ceiling here so this
/// scanner is not a smaller, silent second limit underneath it.
pub(crate) fn reject_duplicate_json_keys_within(
    text: &str,
    max_bytes: usize,
) -> Result<(), String> {
    if text.len() > max_bytes {
        return Err(format!("JSON input exceeds {max_bytes} bytes"));
    }
    JsonKeyScanner {
        bytes: text.as_bytes(),
        pos: 0,
    }
    .scan_document()
}

struct JsonKeyScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonKeyScanner<'_> {
    fn scan_document(&mut self) -> Result<(), String> {
        self.ws();
        self.value(0)?;
        self.ws();
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err("invalid trailing JSON input".to_string())
        }
    }
    fn value(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_JSON_DEPTH {
            return Err(format!("JSON nesting exceeds {MAX_JSON_DEPTH}"));
        }
        self.ws();
        match self.bytes.get(self.pos).copied() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(|_| ()),
            Some(_) => {
                while self.pos < self.bytes.len()
                    && !matches!(
                        self.bytes[self.pos],
                        b',' | b']' | b'}' | b' ' | b'\t' | b'\r' | b'\n'
                    )
                {
                    self.pos += 1;
                }
                Ok(())
            }
            None => Err("unexpected end of JSON".to_string()),
        }
    }
    fn object(&mut self, depth: usize) -> Result<(), String> {
        self.pos += 1;
        self.ws();
        let mut keys = BTreeSet::new();
        if self.take(b'}') {
            return Ok(());
        }
        loop {
            self.ws();
            let key = self.string()?;
            if !keys.insert(key.clone()) {
                return Err(format!("duplicate JSON object key: {key}"));
            }
            self.ws();
            if !self.take(b':') {
                return Err("invalid JSON object separator".to_string());
            }
            self.value(depth)?;
            self.ws();
            if self.take(b'}') {
                return Ok(());
            }
            if !self.take(b',') {
                return Err("invalid JSON object delimiter".to_string());
            }
        }
    }
    fn array(&mut self, depth: usize) -> Result<(), String> {
        self.pos += 1;
        self.ws();
        if self.take(b']') {
            return Ok(());
        }
        loop {
            self.value(depth)?;
            self.ws();
            if self.take(b']') {
                return Ok(());
            }
            if !self.take(b',') {
                return Err("invalid JSON array delimiter".to_string());
            }
        }
    }
    fn string(&mut self) -> Result<String, String> {
        let start = self.pos;
        if !self.take(b'"') {
            return Err("expected JSON string".to_string());
        }
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\\' => {
                    self.pos += 1;
                    if self.pos >= self.bytes.len() {
                        return Err("unterminated JSON escape".to_string());
                    }
                    self.pos += 1;
                }
                b'"' => {
                    self.pos += 1;
                    return serde_json::from_slice(&self.bytes[start..self.pos])
                        .map_err(|e| format!("invalid JSON string: {e}"));
                }
                _ => self.pos += 1,
            }
        }
        Err("unterminated JSON string".to_string())
    }
    fn ws(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.pos += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

pub(crate) fn validate_artifact_ref_shape(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("input Artifact Ref must be an object")?;
    require_exact_keys(
        object,
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
        "input Artifact Ref",
    )?;
    if value["schema"] != "code-intel-artifact-ref.v1" {
        return Err("input Artifact Ref schema is invalid".to_string());
    }
    for key in ["artifactSchema", "type", "path"] {
        if object
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(|v| v.is_empty())
        {
            return Err(format!("input Artifact Ref {key} is invalid"));
        }
    }
    if !value["sha256"].as_str().is_some_and(is_digest) {
        return Err("input Artifact Ref sha256 is invalid".to_string());
    }
    if !value["consumedSnapshotIdentity"].is_null()
        && !value["consumedSnapshotIdentity"]
            .as_str()
            .is_some_and(is_digest)
    {
        return Err("input Artifact Ref consumedSnapshotIdentity is invalid".to_string());
    }
    Ok(())
}
pub(crate) fn require_exact_keys(
    o: &Map<String, Value>,
    keys: &[&str],
    name: &str,
) -> Result<(), String> {
    let a: BTreeSet<&str> = o.keys().map(String::as_str).collect();
    let e: BTreeSet<&str> = keys.iter().copied().collect();
    if a == e {
        Ok(())
    } else {
        Err(format!("{name} fields differ from v1 schema"))
    }
}

pub(crate) fn is_digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn is_run_identity(value: &str) -> bool {
    value.strip_prefix("dag-v1:").is_some_and(|tail| {
        !tail.is_empty()
            && tail.len() % 2 == 0
            && tail
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use super::{is_run_identity, reject_duplicate_json_keys_within};

    #[test]
    fn duplicate_key_scanner_within_honors_explicit_ceiling_over_default() {
        // A payload above the default MAX_JSON_BYTES but within an
        // explicit, larger ceiling must be accepted -- this is the
        // parametrization issue #123 needed: artifact contracts whose
        // declared max_bytes exceeds the default must not be reclamped by
        // this shared scanner's own fixed limit. Sized from the live
        // default so the test keeps discriminating if that default moves.
        let padded = format!(
            r#"{{"key":"{}"}}"#,
            "a".repeat(super::MAX_JSON_BYTES + 1024 * 1024)
        );
        assert!(padded.len() > super::MAX_JSON_BYTES);
        assert!(reject_duplicate_json_keys_within(&padded, 2 * super::MAX_JSON_BYTES).is_ok());
        // The same payload still fails against a ceiling it exceeds.
        let err = reject_duplicate_json_keys_within(&padded, super::MAX_JSON_BYTES).unwrap_err();
        assert!(err.contains("exceeds"));
    }

    #[test]
    fn is_run_identity_requires_dag_v1_prefix() {
        assert!(!is_run_identity("ab"));
        assert!(!is_run_identity("dag-v2:ab"));
    }

    #[test]
    fn is_run_identity_rejects_empty_tail() {
        assert!(!is_run_identity("dag-v1:"));
    }

    #[test]
    fn is_run_identity_requires_even_length_tail() {
        assert!(!is_run_identity("dag-v1:abc"));
        assert!(is_run_identity("dag-v1:abcd"));
    }

    #[test]
    fn is_run_identity_requires_lowercase_hex_tail() {
        assert!(!is_run_identity("dag-v1:AB"));
        assert!(!is_run_identity("dag-v1:gg"));
        assert!(is_run_identity("dag-v1:ab"));
    }
}
