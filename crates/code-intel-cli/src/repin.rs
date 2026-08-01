//! Scans every tracked file for a stale sha256 pin — a 64-hex-digit token
//! that matches a source file's digest at HEAD but no longer matches that
//! file's current worktree content — and reports or rewrites them in one
//! pass.
//!
//! Pins recur in several shapes across this repository (a bare `"sha256"`
//! field, a hex run embedded inside a semicolon-joined `revision` string, an
//! `evidenceId` string that quotes the digest as a suffix), but every shape
//! reduces to the same ground truth: a `[0-9a-f]{64}` token somewhere in
//! tracked text. Scanning for that token directly, rather than modeling each
//! JSON shape, is what lets one pass cover all of them.
//!
//! Pin chains are not one level deep: a file whose own digest is itself
//! pinned elsewhere (for example a shared measurements ledger) must be
//! re-hashed and propagated again after it is patched, so the scan repeats
//! to a fixpoint rather than running once. See `orchestration/retirements/`
//! for the one case that must never be patched this way: those packets
//! freeze a working-tree overlay taken at generation time, not a commit, so
//! there is no "current" digest to resync them to — only their own
//! `New-*RetirementPacket.ps1` generator can produce a valid one.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::snapshot;

const DEFAULT_EXCLUDES: [&str; 1] = ["orchestration/retirements/"];
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PASSES: usize = 25;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let report = match scan(&cli.repo, &cli.excludes) {
        Ok(report) => report,
        Err(message) => {
            eprintln!("error: {message}");
            return 65;
        }
    };
    if cli.write {
        if let Err(error) = flush(&cli.repo, &report) {
            eprintln!("error: {error}");
            return 74;
        }
        match scan(&cli.repo, &cli.excludes) {
            Ok(verify) if !verify.findings.is_empty() => {
                eprintln!(
                    "error: repin left {} stale pin site(s) after write \u{2014} internal inconsistency, please report",
                    verify.findings.len()
                );
                return 65;
            }
            Err(message) => {
                eprintln!("error: verifying after write: {message}");
                return 65;
            }
            Ok(_) => {}
        }
    }
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report.to_json(cli.write))
                .expect("repin report serializes")
        );
    } else {
        print_human(&report, cli.write);
    }
    let orphaned = !report.orphaned.is_empty();
    if cli.write {
        i32::from(orphaned)
    } else if report.is_clean() {
        0
    } else {
        1
    }
}

struct Cli {
    repo: PathBuf,
    write: bool,
    json: bool,
    excludes: Vec<String>,
}

fn parse_cli(raw: &[String]) -> Result<Cli, String> {
    let mut repo = None;
    let mut write = false;
    let mut json = false;
    let mut excludes = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--repo" => {
                let value = raw
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or("--repo requires one value")?;
                if repo.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --repo".into());
                }
                index += 2;
            }
            "--exclude" => {
                let value = raw
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or("--exclude requires one value")?;
                excludes.push(value.replace('\\', "/"));
                index += 2;
            }
            "--write" => {
                write = true;
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other => return Err(format!("unknown repin argument: {other}")),
        }
    }
    let repo = match repo {
        Some(repo) => repo,
        None => std::env::current_dir().map_err(|error| error.to_string())?,
    };
    if !repo.is_dir() {
        return Err(format!(
            "repository path is not a directory: {}",
            repo.display()
        ));
    }
    let repo = fs::canonicalize(&repo).map_err(|error| error.to_string())?;
    Ok(Cli {
        repo,
        write,
        json,
        excludes,
    })
}

struct StaleSite {
    old: String,
    new: String,
    source_path: String,
    count: usize,
}

struct FileFinding {
    path: String,
    sites: Vec<StaleSite>,
}

struct OrphanedFinding {
    path: String,
    old: String,
    deleted_source_path: String,
    count: usize,
}

pub(crate) struct RepinReport {
    passes: usize,
    findings: Vec<FileFinding>,
    orphaned: Vec<OrphanedFinding>,
    rewritten: BTreeMap<String, Vec<u8>>,
}

impl RepinReport {
    fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.orphaned.is_empty()
    }

    fn total_substitutions(&self) -> usize {
        self.findings
            .iter()
            .map(|finding| finding.sites.iter().map(|site| site.count).sum::<usize>())
            .sum()
    }

    fn to_json(&self, write: bool) -> Value {
        json!({
            "schema": "code-intel-repin-report.v1",
            "mode": if write { "write" } else { "report" },
            "passes": self.passes,
            "clean": self.is_clean(),
            "filesChanged": self.findings.len(),
            "totalSubstitutions": self.total_substitutions(),
            "stalePins": self.findings.iter().map(|finding| json!({
                "file": finding.path,
                "sites": finding.sites.iter().map(|site| json!({
                    "old": site.old,
                    "new": site.new,
                    "sourcePath": site.source_path,
                    "count": site.count,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "orphanedPins": self.orphaned.iter().map(|orphan| json!({
                "file": orphan.path,
                "old": orphan.old,
                "deletedSourcePath": orphan.deleted_source_path,
                "count": orphan.count,
            })).collect::<Vec<_>>(),
        })
    }
}

fn print_human(report: &RepinReport, write: bool) {
    for finding in &report.findings {
        println!("{}", finding.path);
        for site in &finding.sites {
            println!(
                "  {}...{} -> {}...{}  ({}x, source: {})",
                &site.old[..12],
                &site.old[56..],
                &site.new[..12],
                &site.new[56..],
                site.count,
                site.source_path
            );
        }
    }
    for orphan in &report.orphaned {
        println!(
            "{}: orphaned pin {}...{} (deleted source: {})",
            orphan.path,
            &orphan.old[..12],
            &orphan.old[56..],
            orphan.deleted_source_path
        );
    }
    if report.is_clean() {
        println!(
            "repin: clean \u{2014} no stale pins ({} pass{})",
            report.passes,
            if report.passes == 1 { "" } else { "es" }
        );
        return;
    }
    let action = if write { "rewrote" } else { "found" };
    println!(
        "repin: {action} {} substitution(s) across {} file(s) in {} pass(es); {} orphaned pin(s)",
        report.total_substitutions(),
        report.findings.len(),
        report.passes,
        report.orphaned.len()
    );
}

fn flush(repo: &Path, report: &RepinReport) -> Result<(), String> {
    for (path, bytes) in &report.rewritten {
        fs::write(repo.join(path), bytes).map_err(|error| format!("write {path}: {error}"))?;
    }
    Ok(())
}

fn scan(repo: &Path, extra_excludes: &[String]) -> Result<RepinReport, String> {
    let mut excludes: Vec<String> = DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
    excludes.extend(extra_excludes.iter().cloned());
    let is_excluded = |path: &str| {
        excludes
            .iter()
            .any(|prefix| path.starts_with(prefix.as_str()))
    };

    let (head_map, worktree_map) = snapshot::repin_digests(repo)?;

    // Reverse index: a HEAD digest resolves to the one source path it came
    // from, so a finding can report *where* a stale pin should now point
    // without threading that path through every pass of the loop below.
    let source_of: BTreeMap<String, String> = head_map
        .iter()
        .filter(|(path, _)| !is_excluded(path))
        .map(|(path, digest)| (digest.clone(), path.clone()))
        .collect();

    let scan_targets: Vec<String> = worktree_map
        .keys()
        .filter(|path| !is_excluded(path))
        .cloned()
        .collect();

    let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut current_digest: BTreeMap<String, String> = worktree_map.clone();
    // path -> old_digest -> occurrences, accumulated across passes.
    let mut findings: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();

    let mut passes = 0usize;
    loop {
        passes += 1;
        if passes > MAX_PASSES {
            return Err(format!(
                "repin did not converge after {MAX_PASSES} passes \u{2014} a pin chain may be cyclic or unbounded"
            ));
        }
        let mut rewrite_map: BTreeMap<String, String> = BTreeMap::new();
        for (path, head_digest) in &head_map {
            if is_excluded(path) {
                continue;
            }
            if let Some(current) = current_digest.get(path) {
                if current != head_digest {
                    rewrite_map.insert(head_digest.clone(), current.clone());
                }
            }
        }
        if rewrite_map.is_empty() {
            break;
        }

        let mut substitutions_this_pass = 0usize;
        for path in &scan_targets {
            if !contents.contains_key(path) {
                match load_text(repo, path) {
                    Some(bytes) => {
                        contents.insert(path.clone(), bytes);
                    }
                    None => continue,
                }
            }
            let bytes = contents.get(path).expect("just loaded or already cached");
            let (rewritten, hits) = rewrite_tokens(bytes, &rewrite_map);
            if hits.is_empty() {
                continue;
            }
            substitutions_this_pass += hits.values().sum::<usize>();
            let entry = findings.entry(path.clone()).or_default();
            for (old, count) in &hits {
                *entry.entry(old.clone()).or_insert(0) += count;
            }
            current_digest.insert(path.clone(), crate::capability::sha256_hex(&rewritten));
            contents.insert(path.clone(), rewritten);
        }
        if substitutions_this_pass == 0 {
            break;
        }
    }

    // A HEAD path that vanished from the worktree pins a digest that can
    // never be resynced: there is no current content to hash. Report it
    // rather than silently ignoring it.
    let deleted_map: BTreeMap<String, String> = head_map
        .iter()
        .filter(|(path, _)| !is_excluded(path) && !worktree_map.contains_key(*path))
        .map(|(path, digest)| (digest.clone(), path.clone()))
        .collect();
    let mut orphaned = Vec::new();
    if !deleted_map.is_empty() {
        for path in &scan_targets {
            let bytes = match contents.get(path) {
                Some(bytes) => bytes.clone(),
                None => match load_text(repo, path) {
                    Some(bytes) => bytes,
                    None => continue,
                },
            };
            let identity: BTreeMap<String, String> = deleted_map
                .keys()
                .map(|old| (old.clone(), old.clone()))
                .collect();
            let (_, hits) = rewrite_tokens(&bytes, &identity);
            for (old, count) in hits {
                orphaned.push(OrphanedFinding {
                    path: path.clone(),
                    deleted_source_path: deleted_map[&old].clone(),
                    old,
                    count,
                });
            }
        }
    }

    let rewritten: BTreeMap<String, Vec<u8>> = findings
        .keys()
        .filter_map(|path| {
            contents
                .get(path)
                .map(|bytes| (path.clone(), bytes.clone()))
        })
        .collect();
    let file_findings = findings
        .into_iter()
        .map(|(path, olds)| {
            let sites = olds
                .into_iter()
                .map(|(old, count)| {
                    let source_path = source_of.get(&old).cloned().unwrap_or_default();
                    let new = current_digest
                        .get(&source_path)
                        .cloned()
                        .unwrap_or_default();
                    StaleSite {
                        old,
                        new,
                        source_path,
                        count,
                    }
                })
                .collect();
            FileFinding { path, sites }
        })
        .collect();

    Ok(RepinReport {
        passes,
        findings: file_findings,
        orphaned,
        rewritten,
    })
}

fn load_text(repo: &Path, path: &str) -> Option<Vec<u8>> {
    let full = repo.join(path);
    let metadata = fs::metadata(&full).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(&full).ok()?;
    std::str::from_utf8(&bytes).ok()?;
    Some(bytes)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Replaces every maximal run of word bytes that is *exactly* 64 bytes long
/// and a key of `rewrite_map`, mirroring `\b[0-9a-f]{64}\b`: a 64-hex run
/// embedded in a longer alphanumeric run (case that would need a word
/// boundary regex would reject) is left untouched, so a digest that happens
/// to sit next to another word character is never partially matched.
fn rewrite_tokens(
    bytes: &[u8],
    rewrite_map: &BTreeMap<String, String>,
) -> (Vec<u8>, BTreeMap<String, usize>) {
    let mut out = Vec::with_capacity(bytes.len());
    let mut hits: BTreeMap<String, usize> = BTreeMap::new();
    let mut index = 0;
    while index < bytes.len() {
        if is_word_byte(bytes[index]) {
            let start = index;
            while index < bytes.len() && is_word_byte(bytes[index]) {
                index += 1;
            }
            let run = &bytes[start..index];
            if run.len() == 64 {
                if let Ok(token) = std::str::from_utf8(run) {
                    if let Some(new) = rewrite_map.get(token) {
                        *hits.entry(token.to_string()).or_insert(0) += 1;
                        out.extend_from_slice(new.as_bytes());
                        continue;
                    }
                }
            }
            out.extend_from_slice(run);
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    (out, hits)
}
