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
    if dir.join("friction.md").exists() {
        return Err(FrictionError::DataErr(format!(
            "{dir_name}: entry already exists (duplicate title logged within the same second)"
        )));
    }
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
                entry::validate_title(value).map_err(FrictionError::Usage)?;
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

    #[test]
    fn run_rejects_a_same_second_duplicate_title_without_touching_the_first_entry() {
        let repo = std::env::temp_dir().join(format!(
            "code-intel-friction-log-collision-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&repo).unwrap();

        // Construct the same-second collision directly rather than relying
        // on two real `run()` calls landing in the same wall-clock second:
        // compute the directory name `run()` will independently derive for
        // this title and pre-populate it, standing in for "an earlier
        // `friction log` call with the same title landed here already".
        let title = "Duplicate friction title";
        let dir_name = entry::dir_name(title);
        let dir = repo.join(entry::ROOT).join(&dir_name);
        let first = Entry::new(dir_name.clone(), title.into(), "first summary".into());
        first.write_atomic(&dir).unwrap();
        let first_contents = std::fs::read_to_string(dir.join("friction.md")).unwrap();

        let raw = vec![
            "--repo".to_string(),
            repo.to_string_lossy().into_owned(),
            "--title".to_string(),
            title.to_string(),
            "--summary".to_string(),
            "second summary".to_string(),
        ];
        let result = run(&raw);
        assert!(
            result.is_err(),
            "same-second duplicate title must fail, not overwrite"
        );
        assert_eq!(result.unwrap_err().exit_code(), 65);

        let after_contents = std::fs::read_to_string(dir.join("friction.md")).unwrap();
        assert_eq!(
            after_contents, first_contents,
            "first entry's friction.md must be untouched by the rejected second call"
        );

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn parse_cli_rejects_title_with_embedded_newline() {
        match parse_cli(&[
            "--title".into(),
            "line one\nline two".into(),
            "--summary".into(),
            "s".into(),
        ]) {
            Ok(_) => panic!("expected an error for a title with an embedded newline"),
            Err(FrictionError::Usage(message)) => assert!(
                message.contains("newline"),
                "expected a newline-related message, got: {message}"
            ),
            Err(other) => panic!("expected FrictionError::Usage, got {other:?}"),
        }
    }
}
