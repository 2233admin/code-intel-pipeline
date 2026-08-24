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
        println!(
            "friction: dry run, would create an issue titled {:?}",
            entry.title
        );
        println!("---");
        println!("{}", entry.body);
        println!("---");
        println!("friction: pass --yes to actually publish");
        return Ok(());
    }

    // Write the issue body inside the entry's own directory, not the
    // shared system temp dir: `std::env::temp_dir()` is world-writable on
    // multi-user hosts, so a predictable path there
    // (`code-intel-friction-publish-<pid>-<id>.md`) could be pre-created by
    // another local user as a symlink, and `std::fs::write` would follow it
    // and clobber whatever it points at. `dir` is this command's own
    // caller-owned entry directory; the tmp-name convention mirrors
    // `Entry::write_atomic`'s `<name>.tmp-<pid>` scratch file below it.
    let mut body_name = std::ffi::OsString::from("body.md");
    body_name.push(format!(".tmp-{}", std::process::id()));
    let body_file = dir.join(body_name);
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
                validate_slug(value)?;
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

/// Rejects any `--slug` value that isn't the `<timestamp>-<slug>` shape
/// `entry::dir_name` produces: ASCII letters, digits, and interior dashes
/// only, non-empty, no leading/trailing/doubled dash. That charset has no
/// room for `/`, `\`, or a `.` (so no `..`), which is what keeps
/// `repo.join(entry::ROOT).join(&cli.slug)` in `run`, above, from ever
/// resolving outside `.agents/friction-log`.
fn validate_slug(slug: &str) -> Result<(), FrictionError> {
    let shape_ok = !slug.is_empty()
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
    if shape_ok {
        Ok(())
    } else {
        Err(FrictionError::Usage(format!(
            "invalid --slug {slug:?}: expected the <timestamp>-<slug> shape `friction log` prints, not a path"
        )))
    }
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

    #[test]
    fn parse_cli_accepts_a_dir_name_shaped_slug() {
        let cli = parse_cli(&["--slug".into(), "20260825T000000Z-example".into()]).unwrap();
        assert_eq!(cli.slug, "20260825T000000Z-example");
    }

    #[test]
    fn parse_cli_rejects_slugs_that_could_escape_the_friction_log_root() {
        for slug in [
            "../evil",
            "..\\evil",
            "a/b",
            "a\\b",
            "..",
            ".",
            "",
            "-leading-dash",
            "trailing-dash-",
            "double--dash",
        ] {
            assert!(
                parse_cli(&["--slug".into(), slug.into()]).is_err(),
                "expected --slug {slug:?} to be rejected"
            );
        }
    }
}
