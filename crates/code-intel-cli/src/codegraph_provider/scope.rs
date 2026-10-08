use std::fs;
use std::path::Path;

use serde_json::Value;

use super::{storage, Failure};

pub(super) fn validate_configuration(
    repo: &Path,
    inventory: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
) -> Result<(), Failure> {
    // Upstream reads this configuration even if it is ignored. A snapshot of
    // only the filename/symlink is not a snapshot of its effective contents.
    match fs::symlink_metadata(repo.join("codegraph.json")) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || !inventory.contains_key("codegraph.json")
            {
                return Err(Failure::contract("CodeGraph configuration is outside the hashed regular-file snapshot; configuration preserved unchanged"));
            }
            storage::check_source_path(repo, "codegraph.json")
                .map_err(|error| Failure::contract(error.message))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Failure::io(format!(
            "inspect CodeGraph project configuration: {error}"
        ))),
    }
}

pub(super) fn validate_indexed_files(
    repo: &Path,
    inventory: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
    files: &Value,
) -> Result<(), Failure> {
    let files = files.as_array().ok_or_else(|| {
        Failure::contract("CodeGraph SDK getFiles returned a non-array inventory")
    })?;
    for file in files {
        let path = file["path"]
            .as_str()
            .ok_or_else(|| Failure::contract("CodeGraph file inventory omits path"))?;
        let portable = path.replace('\\', "/");
        let relative = portable.strip_prefix("./").unwrap_or(&portable);
        if !inventory.contains_key(relative) {
            return Err(Failure::contract(format!("CodeGraph indexed input {path} is outside the hashed snapshot (ignored, symlink or embedded repository); no snapshot binding was published")));
        }
        storage::check_source_path(repo, relative)
            .map_err(|error| Failure::contract(error.message))?;
    }
    Ok(())
}
