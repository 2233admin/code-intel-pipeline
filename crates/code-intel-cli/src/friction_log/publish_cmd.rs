//! `friction publish` — turn a pending entry into a GitHub issue.
//!
//! Defaults to a dry run: prints what would be posted and touches neither
//! GitHub nor the entry file. Only `--yes` actually shells to
//! `gh issue create` and rewrites the entry's `status`/`issue` fields. See
//! `crate::friction_log` module docs and the plan this shipped under for why
//! that gate exists -- this is the first code path in the crate that reaches
//! GitHub on the operator's behalf, and it stays honestly opt-in rather than
//! silently one step short of `docs/follow-up-automation.md`'s stance that
//! the core pipeline never autonomously calls `gh` from the advisory path.

use super::entry::{self, Entry, Status};
use super::{report, take_repo, FrictionError};
use crate::hardened_gh;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    report(run(raw))
}

fn run(raw: &[String]) -> Result<(), FrictionError> {
    let (repo, rest) = take_repo(raw)?;
    let cli = parse_cli(&rest)?;

    let dir = repo.join(entry::ROOT).join(&cli.slug);
    let mut entry = Entry::parse(&dir).map_err(FrictionError::DataErr)?;
    if entry.status != Status::Pending {
        return Err(FrictionError::DataErr(format!(
            "{}: already {}",
            entry.id,
            match entry.status {
                Status::Pending => "pending",
                Status::Published => "published",
            }
        )));
    }

    if !cli.yes {
        println!("friction: dry run, would create an issue titled {:?}", entry.title);
        println!("---");
        println!("{}", entry.body);
        println!("---");
        println!("friction: pass --yes to actually publish");
        return Ok(());
    }

    let body_file = std::env::temp_dir().join(format!(
        "code-intel-friction-publish-{}-{}.md",
        std::process::id(),
        entry.id
    ));
    std::fs::write(&body_file, &entry.body)
        .map_err(|error| FrictionError::HostIo(error.to_string()))?;
    let output = hardened_gh::command(&repo)
        .args([
            "issue",
            "create",
            "--title",
            &entry.title,
            "--body-file",
            &body_file.to_string_lossy(),
        ])
        .output();
    let _ = std::fs::remove_file(&body_file);
    let output = output.map_err(|error| FrictionError::HostIo(format!("spawning gh: {error}")))?;

    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(FrictionError::DataErr(format!(
            "gh issue create exited with {}: {}",
            output.status,
            hardened_gh::redact(text.trim())
        )));
    }

    let issue_url = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| hardened_gh::redact(line.trim()))
        .ok_or_else(|| FrictionError::DataErr("gh issue create produced no issue URL".into()))?;

    entry.status = Status::Published;
    entry.issue = Some(issue_url.clone());
    entry
        .write_atomic(&dir)
        .map_err(|error| FrictionError::HostIo(error.to_string()))?;

    println!("friction: published {} -> {issue_url}", entry.id);
    Ok(())
}

struct Cli {
    slug: String,
    yes: bool,
}

fn parse_cli(raw: &[String]) -> Result<Cli, FrictionError> {
    let mut slug = None;
    let mut yes = false;
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--slug" => {
                let value = raw
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| FrictionError::Usage("--slug requires one value".into()))?;
                if slug.replace(value.clone()).is_some() {
                    return Err(FrictionError::Usage("duplicate --slug".into()));
                }
                index += 2;
            }
            "--yes" => {
                yes = true;
                index += 1;
            }
            other => {
                return Err(FrictionError::Usage(format!(
                    "unknown friction publish argument: {other}"
                )))
            }
        }
    }
    Ok(Cli {
        slug: slug.ok_or_else(|| FrictionError::Usage("--slug is required".into()))?,
        yes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_requires_slug() {
        assert!(parse_cli(&[]).is_err());
        assert!(parse_cli(&["--yes".into()]).is_err());
    }

    #[test]
    fn parse_cli_reads_slug_and_yes() {
        let cli = parse_cli(&["--slug".into(), "abc".into(), "--yes".into()]).unwrap();
        assert_eq!(cli.slug, "abc");
        assert!(cli.yes);
    }
}
