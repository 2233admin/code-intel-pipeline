use std::fs;
use std::path::Path;

use serde_json::Value;

/// Validates the JSON document at `document` against the JSON Schema at
/// `schema`, honouring the draft the schema declares in `$schema`. Both files
/// are read from disk so a test exercises the artifact exactly as published.
#[allow(dead_code)]
pub(crate) fn validates_against_schema(document: &Path, schema: &Path) -> bool {
    let schema = read_json(schema);
    let document = read_json(document);
    jsonschema::validator_for(&schema)
        .expect("schema compiles")
        .is_valid(&document)
}

#[allow(dead_code)]
fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let text = String::from_utf8(bytes).expect("JSON file is UTF-8");
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}
