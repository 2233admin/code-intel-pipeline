//! `friction list` — every unresolved entry, and a validity check: this
//! doubles as a CI gate (exit 65 the moment any entry fails to parse), the
//! same role `frog list` plays upstream.

use serde_json::json;

use super::entry::{self, Entry};
use super::{take_repo, FrictionError};

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match run(raw) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("friction: {}", error.message());
            error.exit_code()
        }
    }
}

fn run(raw: &[String]) -> Result<i32, FrictionError> {
    let (repo, rest) = take_repo(raw)?;
    let as_json = parse_cli(&rest)?;

    let dirs = entry::list_dirs(&repo).map_err(|error| FrictionError::HostIo(error.to_string()))?;
    let results: Vec<Result<Entry, String>> = dirs.iter().map(|dir| Entry::parse(dir)).collect();
    let malformed = results.iter().filter(|result| result.is_err()).count();

    if as_json {
        let entries: Vec<_> = results
            .iter()
            .map(|result| match result {
                Ok(entry) => json!({
                    "ok": true,
                    "id": entry.id,
                    "title": entry.title,
                    "status": status_str(entry),
                    "issue": entry.issue,
                }),
                Err(message) => json!({ "ok": false, "error": message }),
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string(&json!({ "ok": malformed == 0, "entries": entries }))
                .expect("friction list json serializes")
        );
    } else if dirs.is_empty() {
        println!("friction: no entries");
    } else {
        for result in &results {
            match result {
                Ok(entry) => println!(
                    "{} [{}] {}{}",
                    entry.id,
                    status_str(entry),
                    entry.title,
                    entry
                        .issue
                        .as_deref()
                        .map(|issue| format!(" -> {issue}"))
                        .unwrap_or_default()
                ),
                Err(message) => eprintln!("friction: {message}"),
            }
        }
    }

    Ok(if malformed > 0 { 65 } else { 0 })
}

fn status_str(entry: &Entry) -> &'static str {
    match entry.status {
        entry::Status::Pending => "pending",
        entry::Status::Published => "published",
    }
}

fn parse_cli(raw: &[String]) -> Result<bool, FrictionError> {
    let mut as_json = false;
    for argument in raw {
        match argument.as_str() {
            "--json" => as_json = true,
            other => return Err(FrictionError::Usage(format!("unknown friction list argument: {other}"))),
        }
    }
    Ok(as_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_rejects_unknown_flags() {
        assert!(parse_cli(&["--bogus".into()]).is_err());
    }

    #[test]
    fn parse_cli_recognizes_json() {
        assert!(parse_cli(&["--json".into()]).unwrap());
        assert!(!parse_cli(&[]).unwrap());
    }
}
