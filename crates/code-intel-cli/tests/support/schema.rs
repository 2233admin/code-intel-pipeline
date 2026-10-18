use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Resolves a `$ref` to a sibling schema file by its file name, so schemas that
/// reference each other as `code-intel-foo.v1.schema.json` validate offline.
struct SiblingSchemas {
    directory: PathBuf,
}

impl jsonschema::Retrieve for SiblingSchemas {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .path()
            .as_str()
            .rsplit('/')
            .next()
            .ok_or("schema reference has no file name")?;
        Ok(read_json(&self.directory.join(name)))
    }
}

/// Validates the JSON document at `document` against the JSON Schema at
/// `schema`, honouring the draft the schema declares in `$schema`. Both files
/// are read from disk so a test exercises the artifact exactly as published.
#[allow(dead_code)]
pub(crate) fn validates_against_schema(document: &Path, schema: &Path) -> bool {
    let directory = schema
        .parent()
        .expect("schema path has a directory")
        .to_path_buf();
    let schema = read_json(schema);
    let document = read_json(document);
    let validator = jsonschema::options()
        .with_base_uri("https://schemas.invalid/")
        .with_retriever(SiblingSchemas { directory })
        .build(&schema)
        .expect("schema compiles");
    let mut valid = true;
    for error in validator.iter_errors(&document) {
        valid = false;
        eprintln!("schema violation: {error}");
    }
    valid
}

#[allow(dead_code)]
fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let text = String::from_utf8(bytes).expect("JSON file is UTF-8");
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}
