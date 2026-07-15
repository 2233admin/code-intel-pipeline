use crate::sentrux_analysis;
use crate::Result;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Options<'a> {
    pub operation: Option<&'a str>,
    pub repo: Option<&'a Path>,
}

pub fn run(options: &Options<'_>) -> Result<()> {
    let operation = options.operation.ok_or("sentrux requires an operation")?;
    let repo = options.repo.ok_or("sentrux requires a repo/path")?;
    let repo = repo.canonicalize()?;
    if operation == "dsm" {
        let snapshot = sentrux_analysis::analyze(&repo)?;
        println!("{}", serde_json::to_string(&snapshot)?);
        return Ok(());
    }
    let repo_cli = cli_path(&repo);

    let mut args = Vec::new();
    match operation {
        "scan" | "health" | "check" | "gate" => args.push(operation.to_string()),
        "check_rules" => args.push("check".to_string()),
        "gate_save" | "save_baseline" => {
            args.push("gate".to_string());
            args.push("--save".to_string());
        }
        other => {
            return Err(format!("sentrux operation not yet implemented in Rust: {other}").into())
        }
    }
    args.push(repo_cli.clone());

    let binary = if cfg!(windows) {
        "sentrux.cmd"
    } else {
        "sentrux"
    };

    let output = Command::new(binary)
        .args(&args)
        .current_dir(&repo_cli)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }

    if !output.status.success() {
        return Err(format!(
            "sentrux {operation} failed with exit code {}",
            output.status.code().unwrap_or(-1)
        )
        .into());
    }

    Ok(())
}

fn cli_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        return stripped.to_string();
    }
    text.to_string()
}
