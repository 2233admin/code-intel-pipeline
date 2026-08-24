//! `friction log` — record one friction entry.

use std::path::PathBuf;

use super::entry::{self, Entry};
use super::{report, take_repo, FrictionError};

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    report(run(raw))
}

fn run(raw: &[String]) -> Result<(), FrictionError> {
    let (repo, rest) = take_repo(raw)?;
    let cli = parse_cli(&rest)?;

    let dir_name = entry::dir_name(&cli.title);
    let entry = Entry::new(dir_name.clone(), cli.title, cli.summary);
    let dir = repo.join(entry::ROOT).join(&dir_name);
    entry
        .write_atomic(&dir)
        .map_err(|error| FrictionError::HostIo(error.to_string()))?;

    if !cli.artifacts.is_empty() {
        let artifacts_dir = dir.join("artifacts");
        std::fs::create_dir_all(&artifacts_dir)
            .map_err(|error| FrictionError::HostIo(error.to_string()))?;
        for artifact in &cli.artifacts {
            let file_name = artifact.file_name().ok_or_else(|| {
                FrictionError::Usage(format!(
                    "--artifact has no file name: {}",
                    artifact.display()
                ))
            })?;
            std::fs::copy(artifact, artifacts_dir.join(file_name)).map_err(|error| {
                FrictionError::HostIo(format!("copying {}: {error}", artifact.display()))
            })?;
        }
    }

    println!("friction: logged {dir_name}");
    Ok(())
}

struct Cli {
    title: String,
    summary: String,
    artifacts: Vec<PathBuf>,
}

fn parse_cli(raw: &[String]) -> Result<Cli, FrictionError> {
    let mut title = None;
    let mut summary = None;
    let mut artifacts = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--title" => {
                let value = value_of(raw, index, "--title")?;
                if title.replace(value.clone()).is_some() {
                    return Err(FrictionError::Usage("duplicate --title".into()));
                }
                index += 2;
            }
            "--summary" => {
                let value = value_of(raw, index, "--summary")?;
                if summary.replace(value.clone()).is_some() {
                    return Err(FrictionError::Usage("duplicate --summary".into()));
                }
                index += 2;
            }
            "--artifact" => {
                let value = value_of(raw, index, "--artifact")?;
                artifacts.push(PathBuf::from(value));
                index += 2;
            }
            other => {
                return Err(FrictionError::Usage(format!(
                    "unknown friction log argument: {other}"
                )))
            }
        }
    }
    Ok(Cli {
        title: title.ok_or_else(|| FrictionError::Usage("--title is required".into()))?,
        summary: summary.ok_or_else(|| FrictionError::Usage("--summary is required".into()))?,
        artifacts,
    })
}

fn value_of<'a>(raw: &'a [String], index: usize, flag: &str) -> Result<&'a String, FrictionError> {
    raw.get(index + 1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| FrictionError::Usage(format!("{flag} requires one value")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_requires_title_and_summary() {
        assert!(parse_cli(&[]).is_err());
        assert!(parse_cli(&["--title".into(), "x".into()]).is_err());
    }

    #[test]
    fn parse_cli_collects_repeated_artifacts() {
        let cli = parse_cli(&[
            "--title".into(),
            "t".into(),
            "--summary".into(),
            "s".into(),
            "--artifact".into(),
            "a.txt".into(),
            "--artifact".into(),
            "b.txt".into(),
        ])
        .unwrap();
        assert_eq!(cli.title, "t");
        assert_eq!(cli.summary, "s");
        assert_eq!(
            cli.artifacts,
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]
        );
    }
}
