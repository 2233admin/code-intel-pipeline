//! One friction-log entry: a directory under `.agents/friction-log/` holding
//! a `friction.md` with a small hand-rolled frontmatter block (this crate has
//! no YAML/markdown dependency, and the format here is deliberately smaller
//! than either) followed by the free-text write-up.
//!
//! ```text
//! title: <text>
//! created: <YYYYMMDDTHHMMSSZ>
//! status: pending|published
//! issue: <url, blank until published>
//! ---
//!
//! <body>
//! ```

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const ROOT: &str = ".agents/friction-log";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    Pending,
    Published,
}

impl Status {
    fn as_str(&self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Published => "published",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Status::Pending),
            "published" => Ok(Status::Published),
            other => Err(format!("unknown status: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    /// The entry directory's name (`<timestamp>-<slug>`), unique within
    /// `.agents/friction-log/` and how `--slug` on `publish`/`sync` addresses
    /// one entry.
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) created: String,
    pub(crate) status: Status,
    pub(crate) issue: Option<String>,
    pub(crate) body: String,
}

impl Entry {
    pub(crate) fn new(id: String, title: String, body: String) -> Self {
        Entry {
            id,
            title,
            created: timestamp_prefix(),
            status: Status::Pending,
            issue: None,
            body,
        }
    }

    fn render(&self) -> String {
        format!(
            "title: {}\ncreated: {}\nstatus: {}\nissue: {}\n---\n\n{}\n",
            self.title,
            self.created,
            self.status.as_str(),
            self.issue.as_deref().unwrap_or(""),
            self.body.trim_end()
        )
    }

    /// Writes `friction.md` inside `dir` via temp-file-then-rename, the
    /// atomic pattern `git_remote_registry::registry::save` uses: a reader
    /// (`list`) must never observe a half-written file.
    pub(crate) fn write_atomic(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let target = dir.join("friction.md");
        let mut tmp_name = std::ffi::OsString::from("friction.md");
        tmp_name.push(format!(".tmp-{}", std::process::id()));
        let tmp_path = dir.join(tmp_name);
        std::fs::write(&tmp_path, self.render())?;
        std::fs::rename(&tmp_path, &target)
    }

    pub(crate) fn parse(dir: &Path) -> Result<Entry, String> {
        let id = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("entry directory has no name: {}", dir.display()))?;
        let path = dir.join("friction.md");
        let content = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        parse_content(&id, &content).map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn parse_content(id: &str, content: &str) -> Result<Entry, String> {
    let mut lines = content.lines();
    let mut title = None;
    let mut created = None;
    let mut status = None;
    let mut issue = None;
    for line in &mut lines {
        if line == "---" {
            break;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("malformed frontmatter line: {line:?}"))?;
        let value = value.trim().to_string();
        match key.trim() {
            "title" => title = Some(value),
            "created" => created = Some(value),
            "status" => status = Some(Status::parse(&value)?),
            "issue" => issue = (!value.is_empty()).then_some(value),
            other => return Err(format!("unknown frontmatter key: {other}")),
        }
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.strip_prefix('\n').unwrap_or(&body).trim().to_string();
    Ok(Entry {
        id: id.to_string(),
        title: title.ok_or("missing title")?,
        created: created.ok_or("missing created")?,
        status: status.ok_or("missing status")?,
        issue,
        body,
    })
}

/// `dir_name(&title)` for a freshly logged entry: `<timestamp>-<slug>`.
pub(crate) fn dir_name(title: &str) -> String {
    format!("{}-{}", timestamp_prefix(), slugify(title))
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = true; // suppress a leading dash
    for ch in title.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(48);
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "entry".to_string()
    } else {
        slug
    }
}

/// `YYYYMMDDTHHMMSSZ` for the current moment. No timezone crate: everything
/// here is UTC, computed straight from the Unix epoch second count with the
/// standard civil-calendar-from-day-count algorithm (Howard Hinnant's
/// `civil_from_days`), since this crate carries no date/time dependency.
fn timestamp_prefix() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_seconds(now.as_secs())
}

fn format_unix_seconds(total_seconds: u64) -> String {
    let days = (total_seconds / 86_400) as i64;
    let time_of_day = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// Days-since-epoch to `(year, month, day)`, valid for the proleptic
/// Gregorian calendar. Reference:
/// <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

/// Every entry directory under `.agents/friction-log`, sorted by name (which
/// sorts by timestamp since the prefix is fixed-width and zero-padded).
pub(crate) fn list_dirs(repo: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root = repo.join(ROOT);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_render_and_parse() {
        let entry = Entry::new(
            "20260825T000000Z-example".into(),
            "Example friction".into(),
            "Body text.\nSecond line.".into(),
        );
        let dir = std::env::temp_dir().join(format!(
            "code-intel-friction-entry-roundtrip-{}",
            std::process::id()
        ));
        entry.write_atomic(&dir).unwrap();
        let parsed = Entry::parse(&dir).unwrap();
        assert_eq!(parsed.title, "Example friction");
        assert_eq!(parsed.status, Status::Pending);
        assert_eq!(parsed.issue, None);
        assert_eq!(parsed.body, "Body text.\nSecond line.");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_rejects_missing_frontmatter_field() {
        let dir = std::env::temp_dir().join(format!(
            "code-intel-friction-entry-malformed-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("friction.md"), "title: only a title\n---\nbody\n").unwrap();
        assert!(Entry::parse(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn slugify_strips_punctuation_and_collapses_separators() {
        assert_eq!(slugify("Config loader turns `undefined` into a string!"), "config-loader-turns-undefined-into-a-string");
    }

    #[test]
    fn civil_from_days_round_trips_through_its_own_inverse() {
        // `days_from_civil` is the textbook inverse of `civil_from_days`
        // (Hinnant's `date_algorithms.html#days_from_civil`); round-tripping
        // through it checks the forward direction without hand-computing an
        // epoch day count, which is exactly the kind of arithmetic this
        // function exists to avoid doing by hand elsewhere.
        fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
            let y = if m <= 2 { y - 1 } else { y };
            let era = if y >= 0 { y } else { y - 399 } / 400;
            let yoe = (y - era * 400) as u64;
            let mp = if m > 2 { m - 3 } else { m + 9 };
            let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            era * 146_097 + doe as i64 - 719_468
        }

        for (year, month, day) in [
            (1970, 1, 1),
            (2000, 1, 1),
            (2000, 2, 29),
            (2026, 8, 25),
            (2100, 3, 1),
        ] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }
    }

    #[test]
    fn civil_from_days_of_the_epoch_is_the_epoch_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
