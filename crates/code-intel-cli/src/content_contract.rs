//! Self-contained content-contract primitives shared by the capability
//! framework and Artifact Ref verification. Keeping them in a leaf module
//! removes the artifact_ref <-> capability import cycle: capability
//! re-exports these names, artifact_ref includes this file directly.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

pub(crate) const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
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
        if !object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
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
        data.push(0)
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
            w[i] = u32::from_be_bytes(word.try_into().unwrap())
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1)
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2)
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value)
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::{is_run_identity, reject_duplicate_json_keys_within};

    #[test]
    fn duplicate_key_scanner_within_honors_explicit_ceiling_over_default() {
        // A payload above the default MAX_JSON_BYTES but within an
        // explicit, larger ceiling must be accepted -- this is the
        // parametrization issue #123 needed: artifact contracts whose
        // declared max_bytes exceeds the default 8 MiB must not be
        // reclamped by this shared scanner's own fixed limit.
        let padded = format!(r#"{{"key":"{}"}}"#, "a".repeat(9 * 1024 * 1024));
        assert!(padded.len() > super::MAX_JSON_BYTES);
        assert!(reject_duplicate_json_keys_within(&padded, 16 * 1024 * 1024).is_ok());
        // The same payload still fails against a ceiling it exceeds.
        let err = reject_duplicate_json_keys_within(&padded, 8 * 1024 * 1024).unwrap_err();
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
