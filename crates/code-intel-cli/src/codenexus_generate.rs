//! Compiled CLI shell for the active CodeNexus-lite generator.

use std::fs;
use std::path::PathBuf;

use crate::codenexus_lite::{
    build_active_context, iso_from_unix_seconds, iso_now, normalized_canonical_path,
};

const DEFAULT_MAX_FILES: usize = 8;
const DEFAULT_MAX_REFERENCES_PER_FILE: usize = 12;

#[derive(Debug)]
struct GenerateArgs {
    repo: PathBuf,
    target: Option<PathBuf>,
    out: PathBuf,
    observed_at: Option<i64>,
    max_files: usize,
    max_references_per_file: usize,
}

/// Compiled operator route for the active CodeNexus-lite generator.
///
/// `--observed-at` fixes the generated timestamp for reproducible contract
/// fixtures. Omitting it preserves the compatibility facade's wall-clock
/// timestamp behavior.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let arguments = match parse_generate_args(raw) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("error: {error}");
            return 64;
        }
    };
    let repo = match normalized_canonical_path(&arguments.repo) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            eprintln!(
                "error: CodeNexus repository path is not a directory: {}",
                arguments.repo.display()
            );
            return 65;
        }
        Err(error) => {
            eprintln!(
                "error: resolve CodeNexus repository path {}: {error}",
                arguments.repo.display()
            );
            return 65;
        }
    };
    let target_input = arguments.target.as_deref().unwrap_or(&repo);
    let target = match normalized_canonical_path(target_input) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            eprintln!(
                "error: CodeNexus target path is not a directory: {}",
                target_input.display()
            );
            return 65;
        }
        Err(error) => {
            eprintln!(
                "error: resolve CodeNexus target path {}: {error}",
                target_input.display()
            );
            return 65;
        }
    };

    let generated_at = arguments
        .observed_at
        .map(iso_from_unix_seconds)
        .unwrap_or_else(iso_now);
    let document = build_active_context(
        &repo,
        &target,
        &arguments.out,
        generated_at,
        arguments.max_files,
        arguments.max_references_per_file,
    );
    let rendered = match serde_json::to_string_pretty(&document) {
        Ok(rendered) => format!("{rendered}\n"),
        Err(error) => {
            eprintln!("error: serialize CodeNexus context: {error}");
            return 74;
        }
    };
    if let Some(parent) = arguments.out.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!("error: create CodeNexus output directory: {error}");
                return 74;
            }
        }
    }
    if let Err(error) = fs::write(&arguments.out, rendered.as_bytes()) {
        eprintln!("error: write CodeNexus context: {error}");
        return 74;
    }
    print!("{rendered}");
    0
}

fn parse_generate_args(raw: &[String]) -> Result<GenerateArgs, String> {
    let mut repo = None;
    let mut target = None;
    let mut out = None;
    let mut observed_at = None;
    let mut max_files = DEFAULT_MAX_FILES;
    let mut max_references_per_file = DEFAULT_MAX_REFERENCES_PER_FILE;
    let mut index = 0usize;
    while index < raw.len() {
        let flag = raw[index].as_str();
        let value = || {
            raw.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag {
            "--repo" => repo = Some(PathBuf::from(value()?)),
            "--target" => target = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--observed-at" => {
                let parsed = value()?
                    .parse::<i64>()
                    .map_err(|_| "--observed-at must be a non-negative integer".to_string())?;
                if parsed < 0 {
                    return Err("--observed-at must be a non-negative integer".to_string());
                }
                observed_at = Some(parsed);
            }
            "--max-files" => {
                max_files = value()?
                    .parse::<usize>()
                    .map_err(|_| "--max-files must be a positive integer".to_string())?;
                if max_files == 0 {
                    return Err("--max-files must be a positive integer".to_string());
                }
            }
            "--max-references-per-file" => {
                max_references_per_file = value()?.parse::<usize>().map_err(|_| {
                    "--max-references-per-file must be a non-negative integer".to_string()
                })?;
            }
            unknown => return Err(format!("unknown codenexus generate argument: {unknown}")),
        }
        index += 2;
    }
    Ok(GenerateArgs {
        repo: repo.ok_or_else(|| "codenexus generate requires --repo".to_string())?,
        target,
        out: out.ok_or_else(|| "codenexus generate requires --out".to_string())?,
        observed_at,
        max_files,
        max_references_per_file,
    })
}
