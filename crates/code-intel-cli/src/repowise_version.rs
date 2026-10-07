//! Pure Repowise report parsing and installed-version floor decisions.
//! PEP 440 ordering is owned by the native CLI, including compatibility calls.

use std::str::FromStr;

use pep440_rs::Version;
use serde_json::json;

fn reported_version(report: &str) -> Option<Version> {
    let mut found = None;
    for line in report.lines() {
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else { continue };
        let name = name
            .strip_suffix(',')
            .or_else(|| name.strip_suffix(':'))
            .unwrap_or(name);
        if !name.eq_ignore_ascii_case("repowise") {
            continue;
        }
        let first = words.next()?;
        let token = if first.eq_ignore_ascii_case("version") {
            words.next()?
        } else {
            first
        };
        if words.next().is_some() {
            return None;
        }
        let version = Version::from_str(token).ok()?;
        if found.as_ref().is_some_and(|previous| previous != &version) {
            return None;
        }
        found = Some(version);
    }
    found
}

pub fn run_raw(raw: &[String]) -> i32 {
    let mut report = None;
    let mut minimum = None;
    let mut arguments = raw.iter();
    while let Some(argument) = arguments.next() {
        let destination = match argument.as_str() {
            "--reported" => &mut report,
            "--minimum" => &mut minimum,
            _ => {
                eprintln!("unknown repowise-version option: {argument}");
                return 64;
            }
        };
        if destination.is_some() {
            eprintln!("duplicate repowise-version option: {argument}");
            return 64;
        }
        let Some(value) = arguments.next() else {
            eprintln!("missing value for {argument}");
            return 64;
        };
        *destination = Some(value.as_str());
    }
    let Some(report) = report else {
        eprintln!("usage: code-intel repowise-version --reported <stdout> [--minimum <version>]");
        return 64;
    };
    let minimum = match minimum.map(Version::from_str).transpose() {
        Ok(minimum) => minimum,
        Err(error) => {
            eprintln!("invalid Repowise minimum: {error}");
            return 64;
        }
    };
    let version = reported_version(report);
    let ordering = version
        .as_ref()
        .zip(minimum.as_ref())
        .map(|(version, minimum)| version.cmp(minimum));
    let meets_minimum = ordering.map(|ordering| ordering != std::cmp::Ordering::Less);
    let status = match (version.as_ref(), meets_minimum) {
        (None, _) => "unknown",
        (Some(_), None) => "parsed",
        (_, Some(true)) => "accepted",
        (_, Some(false)) => "below_minimum",
    };
    println!(
        "{}",
        json!({
            "schema": "code-intel-repowise-version.v1",
            "version": version.as_ref().map(ToString::to_string),
            "minimum": minimum.as_ref().map(ToString::to_string),
            "meetsMinimum": meets_minimum,
            "ordering": ordering.map(|ordering| match ordering {
                std::cmp::Ordering::Less => "less",
                std::cmp::Ordering::Equal => "equal",
                std::cmp::Ordering::Greater => "greater",
            }),
            "status": status,
        })
    );
    0
}
