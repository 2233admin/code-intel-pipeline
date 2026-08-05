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

use crate::declared_pins;
use crate::evidence_outcome::{EvidenceOutcome, EvidenceScope, PartialReason};
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
    // Digests an internalization record declares about itself are invisible to
    // the scan above: it only sees a pin as stale while HEAD and the worktree
    // disagree, which stops being true the moment the edit is committed. The
    // record carries the pinned path next to the digest, so those are resolved
    // from that declaration instead. See `declared_pins`.
    let declared = match declared_pins::audit(&cli.repo) {
        Ok(findings) => findings,
        Err(message) => {
            eprintln!("error: auditing declared pins: {message}");
            return 65;
        }
    };

    if cli.write {
        if let Err(error) = flush(&cli.repo, &report) {
            eprintln!("error: {error}");
            return 74;
        }
        match declared_pins::resync(&cli.repo, &declared) {
            Ok(records) if !records.is_empty() => {
                for record in &records {
                    println!("repin: resynced declared pins in {record}");
                }
            }
            Err(message) => {
                eprintln!("error: resyncing declared pins: {message}");
                return 74;
            }
            Ok(_) => {}
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
        match declared_pins::audit(&cli.repo) {
            Ok(verify) => {
                let unresolved: Vec<_> = verify
                    .iter()
                    .filter(|finding| {
                        matches!(finding.state, declared_pins::PinState::Stale { .. })
                    })
                    .collect();
                if !unresolved.is_empty() {
                    eprintln!(
                        "error: repin left {} declared pin(s) stale after write \u{2014} internal inconsistency, please report",
                        unresolved.len()
                    );
                    return 65;
                }
            }
            Err(message) => {
                eprintln!("error: verifying declared pins after write: {message}");
                return 65;
            }
        }
    }
    declared_pins::print_findings(&declared, cli.write);
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report.to_json(cli.write))
                .expect("repin report serializes")
        );
    } else {
        print_human(&report, cli.write);
    }
    // A declared pin that `--write` refused to touch (ambiguous, or its source
    // is gone) is unresolved in exactly the sense `has_unresolved` already
    // means: the tree is not consistent and no further `repin` run will make
    // it so.
    let declared_unresolved = declared.iter().any(|finding| {
        matches!(
            finding.state,
            declared_pins::PinState::Ambiguous | declared_pins::PinState::SourceMissing
        )
    });
    let declared_dirty = declared
        .iter()
        .any(declared_pins::PinFinding::needs_attention);

    if cli.write {
        i32::from(report.has_unresolved() || declared_unresolved)
    } else if report.is_clean() && !declared_dirty {
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

/// A HEAD digest shared by more than one non-excluded path: repin cannot
/// tell which of `paths` a pin referencing `digest` meant, so it resolves
/// none of them and reports the ambiguity instead of guessing.
struct AmbiguousSource {
    digest: String,
    paths: Vec<String>,
}

/// A scan target repin could not read at all. `gates_clean` is false only
/// for binary files and symlinks, which structurally cannot hold a pin in
/// this repo's convention; anything else (too large, unreadable) means real
/// text went uninspected and must not be reported as "clean".
struct SkippedFile {
    path: String,
    reason: String,
    gates_clean: bool,
}

pub(crate) struct RepinReport {
    passes: usize,
    findings: Vec<FileFinding>,
    orphaned: Vec<OrphanedFinding>,
    ambiguous: Vec<AmbiguousSource>,
    skipped: Vec<SkippedFile>,
    rewritten: BTreeMap<String, Vec<u8>>,
    /// Gate G1 (issue #139 / #138): whether the pin-staleness scan itself
    /// covered its whole surface, computed once here from the same
    /// `scan_targets` / `skipped` data every other field on this struct is
    /// built from. `is_clean()` and the JSON `scanCoverage` field both read
    /// this single value, so they cannot drift apart the way the charter
    /// warns against (a pass/fail decision reading different data than the
    /// interval it is supposed to be judging).
    outcome: EvidenceOutcome,
}

impl RepinReport {
    /// Orphaned pins and ambiguous digests are never touched by `--write`
    /// — they need a human, not a resync — so they stay "unresolved" even
    /// after a successful write. An incomplete scan (`outcome` is not
    /// `Complete`) is unresolved for the same reason a stale pin is: the
    /// report cannot back up a "clean" claim over ground it never covered.
    fn has_unresolved(&self) -> bool {
        !self.orphaned.is_empty() || !self.ambiguous.is_empty() || !self.outcome.is_complete()
    }

    fn is_clean(&self) -> bool {
        self.findings.is_empty() && !self.has_unresolved()
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
            "scanCoverage": self.outcome.to_json(),
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
            "ambiguousSources": self.ambiguous.iter().map(|ambiguous| json!({
                "digest": ambiguous.digest,
                "paths": ambiguous.paths,
            })).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|skip| json!({
                "file": skip.path,
                "reason": skip.reason,
                "gatesClean": skip.gates_clean,
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
    for ambiguous in &report.ambiguous {
        println!(
            "ambiguous: {}...{} is shared by {} \u{2014} not resolved, resync by hand",
            &ambiguous.digest[..12],
            &ambiguous.digest[56..],
            ambiguous.paths.join(", ")
        );
    }
    for skip in &report.skipped {
        println!(
            "skipped: {} ({}{})",
            skip.path,
            skip.reason,
            if skip.gates_clean {
                ""
            } else {
                ", informational"
            }
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
    if !report.outcome.is_complete() {
        println!("scan coverage: {}", report.outcome.describe());
    }
    let action = if write { "rewrote" } else { "found" };
    println!(
        "repin: {action} {} substitution(s) across {} file(s) in {} pass(es); {} orphaned pin(s), {} ambiguous digest(s), {} skipped file(s)",
        report.total_substitutions(),
        report.findings.len(),
        report.passes,
        report.orphaned.len(),
        report.ambiguous.len(),
        report.skipped.len()
    );
}

fn flush(repo: &Path, report: &RepinReport) -> Result<(), String> {
    for (path, bytes) in &report.rewritten {
        let target = repo.join(path);
        // Append rather than replace the extension (`Path::with_extension`
        // would turn `a.json` and `a.txt` into the same `a.repin-tmp`), and
        // write-then-rename so a failure mid-write never leaves `target`
        // itself truncated: either the rename lands the full new content or
        // the original file is untouched.
        let mut temp_name = target
            .file_name()
            .ok_or_else(|| format!("{path}: has no file name"))?
            .to_os_string();
        temp_name.push(".repin-tmp");
        let temporary = target.with_file_name(temp_name);
        if let Err(error) = fs::write(&temporary, bytes) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("write {path}: {error}"));
        }
        // Best-effort: `fs::write` creates `temporary` with default
        // permissions, which would silently drop the original file's mode
        // (e.g. an executable bit) across the rename below. A failure to
        // read or set permissions is not fatal to the rewrite itself.
        if let Ok(metadata) = fs::metadata(&target) {
            let _ = fs::set_permissions(&temporary, metadata.permissions());
        }
        if let Err(error) = fs::rename(&temporary, &target) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("replace {path}: {error}"));
        }
    }
    Ok(())
}

fn scan(repo: &Path, extra_excludes: &[String]) -> Result<RepinReport, String> {
    let mut excludes: Vec<String> = DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
    excludes.extend(extra_excludes.iter().cloned());
    let is_excluded = |path: &str| {
        excludes.iter().any(|prefix| {
            let prefix = prefix.trim_end_matches('/');
            path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    };

    let digests = snapshot::repin_digests(repo)?;
    let head_map = digests.head;
    let worktree_map = digests.worktree;

    // Two (or more) tracked files can share one HEAD digest (empty files,
    // copied fixtures, license copies). A digest-keyed map cannot then say
    // which of them a given pin meant, so any digest shared by more than one
    // non-excluded HEAD path is resolved nowhere below — never rewritten,
    // never reported as orphaned, only surfaced as ambiguous.
    let mut digest_paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, digest) in &head_map {
        if is_excluded(path) {
            continue;
        }
        digest_paths
            .entry(digest.clone())
            .or_default()
            .push(path.clone());
    }
    let ambiguous_digests: std::collections::BTreeSet<String> = digest_paths
        .iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(digest, _)| digest.clone())
        .collect();

    // Reverse index: an unambiguous HEAD digest resolves to the one source
    // path it came from, so a finding can report *where* a stale pin should
    // now point without threading that path through every pass below.
    let source_of: BTreeMap<String, String> = digest_paths
        .iter()
        .filter(|(digest, _)| !ambiguous_digests.contains(*digest))
        .map(|(digest, paths)| (digest.clone(), paths[0].clone()))
        .collect();

    let scan_targets: Vec<String> = worktree_map
        .keys()
        .filter(|path| !is_excluded(path))
        .cloned()
        .collect();

    let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    // Paths whose content couldn't be read at all: sticky across passes,
    // since a file that's too large or not UTF-8 now will still be too large
    // or not UTF-8 on the next pass.
    let mut skipped: BTreeMap<String, LoadError> = BTreeMap::new();
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
            if is_excluded(path) || ambiguous_digests.contains(head_digest) {
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
            if skipped.contains_key(path) {
                continue;
            }
            if !contents.contains_key(path) {
                match load_text(repo, path) {
                    Ok(bytes) => {
                        contents.insert(path.clone(), bytes);
                    }
                    Err(error) => {
                        skipped.insert(path.clone(), error);
                        continue;
                    }
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
    // rather than silently ignoring it. Ambiguous digests are excluded here
    // too: if a surviving twin still carries the same digest, the pin isn't
    // actually orphaned, and if every twin is gone, we still can't say which
    // deleted path a given pin meant.
    let deleted_map: BTreeMap<String, String> = digest_paths
        .iter()
        .filter(|(digest, paths)| {
            !ambiguous_digests.contains(*digest) && !worktree_map.contains_key(&paths[0])
        })
        .map(|(digest, paths)| (digest.clone(), paths[0].clone()))
        .collect();
    let mut orphaned = Vec::new();
    if !deleted_map.is_empty() {
        let identity: BTreeMap<String, String> = deleted_map
            .keys()
            .map(|old| (old.clone(), old.clone()))
            .collect();
        for path in &scan_targets {
            if skipped.contains_key(path) {
                continue;
            }
            let loaded;
            let bytes = match contents.get(path) {
                Some(bytes) => bytes,
                None => match load_text(repo, path) {
                    Ok(bytes) => {
                        loaded = bytes;
                        &loaded
                    }
                    Err(error) => {
                        skipped.insert(path.clone(), error);
                        continue;
                    }
                },
            };
            let (_, hits) = rewrite_tokens(bytes, &identity);
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

    let ambiguous: Vec<AmbiguousSource> = ambiguous_digests
        .into_iter()
        .map(|digest| AmbiguousSource {
            paths: digest_paths[&digest].clone(),
            digest,
        })
        .collect();

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
    let skipped: Vec<SkippedFile> = skipped
        .into_iter()
        .map(|(path, error)| SkippedFile {
            gates_clean: error.gates_clean(),
            reason: error.message(),
            path,
        })
        .collect();

    let outcome = scan_coverage(&scan_targets, &skipped, &excludes);

    Ok(RepinReport {
        passes,
        findings: file_findings,
        orphaned,
        ambiguous,
        skipped,
        rewritten,
        outcome,
    })
}

/// Gate G1 (#139 / #138): compute the honesty bit for this scan from
/// exactly the data every other part of the report is built from --
/// `scan_targets` (what a full scan would have covered after exclusions)
/// and `skipped` (what actually failed to load). No separate tally is
/// kept anywhere else, so this can never drift from the report it
/// describes.
///
/// An empty `scan_targets` is deliberately treated as `NotComputed`, not
/// vacuously `Complete`: `--exclude`-ing away every tracked file (or
/// pointing `--repo` at a tree with none) previously made `is_clean()`
/// return `true` on zero files inspected, printing the same
/// `clean: true, filesChanged: 0` shape as the #133 bug this type exists
/// to prevent -- a scan that covered nothing must not read the same as a
/// scan that covered everything and found it clean.
fn scan_coverage(
    scan_targets: &[String],
    skipped: &[SkippedFile],
    excludes: &[String],
) -> EvidenceOutcome {
    if scan_targets.is_empty() {
        return EvidenceOutcome::NotComputed {
            reason: "scan surface empty: no tracked file remained after exclusions".to_string(),
        };
    }
    let scanned_paths: Vec<String> = scan_targets
        .iter()
        .filter(|path| !skipped.iter().any(|skip| &skip.path == *path))
        .cloned()
        .collect();
    let scope = EvidenceScope {
        artifact_types: vec!["tracked-text-file".to_string()],
        scanned_paths,
        excluded_prefixes: excludes.to_vec(),
    };
    // Binary/symlink skips are outside the "tracked-text-file" artifact
    // type this scan declares, so their absence from `scanned_paths` is
    // not a gap. Only skips that gate `clean` today (too large, unreadable)
    // represent text this scan should have covered but could not.
    let unreadable: Vec<String> = skipped
        .iter()
        .filter(|skip| skip.gates_clean)
        .map(|skip| skip.path.clone())
        .collect();
    if unreadable.is_empty() {
        EvidenceOutcome::Complete(scope)
    } else {
        EvidenceOutcome::Partial {
            reason: PartialReason::UnreadableInput(unreadable),
            scope,
        }
    }
}

/// Why a scan target's content wasn't inspected. `Binary` is routine and
/// never gates "clean" — this repository's pins only ever live in text — but
/// `TooLarge` and `Unreadable` mean a file that *could* hold a pin went
/// uninspected, which the report must never fold into a silent "clean".
enum LoadError {
    Binary,
    Symlink,
    TooLarge,
    Unreadable(String),
}

impl LoadError {
    fn message(&self) -> String {
        match self {
            LoadError::Binary => "not valid UTF-8 (binary file)".to_string(),
            LoadError::Symlink => "symbolic link (target is not scanned for pins)".to_string(),
            LoadError::TooLarge => format!("exceeds {MAX_FILE_BYTES} byte scan limit"),
            LoadError::Unreadable(detail) => format!("unreadable: {detail}"),
        }
    }

    fn gates_clean(&self) -> bool {
        !matches!(self, LoadError::Binary | LoadError::Symlink)
    }
}

fn load_text(repo: &Path, path: &str) -> Result<Vec<u8>, LoadError> {
    let full = repo.join(path);
    // `symlink_metadata` (unlike `metadata`) does not follow a symlink, so a
    // tracked symlink is caught here instead of falling through to `is_file`
    // on its *target*: if it were loaded and later rewritten, `flush` would
    // rename a regular file on top of the link path, destroying the symlink.
    let metadata =
        fs::symlink_metadata(&full).map_err(|error| LoadError::Unreadable(error.to_string()))?;
    if metadata.is_symlink() {
        return Err(LoadError::Symlink);
    }
    if !metadata.is_file() {
        return Err(LoadError::Unreadable("not a regular file".to_string()));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge);
    }
    let bytes = fs::read(&full).map_err(|error| LoadError::Unreadable(error.to_string()))?;
    std::str::from_utf8(&bytes).map_err(|_| LoadError::Binary)?;
    Ok(bytes)
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
