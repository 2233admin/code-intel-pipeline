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

/// FIPS 180-2 vectors around the padding boundaries: the 56-byte message
/// forces the extra length block, and a million bytes fed in odd-sized
/// pieces exercises the partial-block carry across `update` calls.
#[test]
fn streaming_sha256_matches_published_vectors() {
    assert_eq!(
        super::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        super::sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let mut hasher = super::Sha256::new();
    let piece = [b'a'; 997];
    let mut remaining = 1_000_000;
    while remaining > 0 {
        let take = remaining.min(piece.len());
        hasher.update(&piece[..take]);
        remaining -= take;
    }
    assert_eq!(
        hasher.finish(),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn streaming_sha256_ignores_chunk_boundaries() {
    let message = (0..300u32)
        .map(|i| (i * 31 % 256) as u8)
        .collect::<Vec<_>>();
    for len in [0, 1, 55, 56, 63, 64, 65, 119, 120, 128, 300] {
        let whole = super::sha256_hex(&message[..len]);
        for split in 0..=len {
            let mut hasher = super::Sha256::new();
            hasher.update(&message[..split]);
            hasher.update(&message[split..len]);
            assert_eq!(hasher.finish(), whole, "len {len} split {split}");
        }
    }
}
