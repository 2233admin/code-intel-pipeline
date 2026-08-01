#!/usr/bin/env python3
"""Arm A (artifact-guided) and Arm B (naive) search/read logic for eval/harness.py.

Split out of eval/harness.py (issue #93) so no single file in eval/ trips the
sentrux god-file ratchet (`functions > 25 && loc > 400`, or `loc > 800`;
crates/code-intel-cli/src/sentrux_gate.rs:738). Pure extraction -- behavior
is unchanged; see eval/harness.py's module docstring for what Arm A/B
measure and why.

`window` and `file_cap` are plain parameters here, not module constants:
eval/harness.py owns `WINDOW`/`FILE_CAP` as the single mutable source of
truth (eval/test_harness.py pokes `harness.FILE_CAP` directly), and its thin
`arm_a_answer`/`arm_b_answer`/`answer_question` wrappers pass the current
value in on every call. See those wrappers for why this module deliberately
never imports back from harness.py.
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

# --------------------------------------------------------------------------
# Small path helpers (Windows host; keep everything forward-slash on output)
# --------------------------------------------------------------------------


def posix(path: str) -> str:
    return path.replace("\\", "/")


def read_bytes(path: Path) -> bytes:
    return path.read_bytes()


def read_text(path: Path) -> str:
    return path.read_bytes().decode("utf-8", errors="strict")


# --------------------------------------------------------------------------
# Corpus exclusions (issue #93 self-reference pollution fix, #104).
#
# The benchmark must never read its own corpus, its own outputs, or archived
# material. eval/ holds eval/questions.json, which contains every question's
# own golden.keywords as literal JSON text -- an unfiltered naive search
# therefore ranks the benchmark's own question file as a top hit for the
# very question it defines (verified: q06's keywords match eval/questions.json
# 11 times), and eval/BASELINE.md (a rendered per-question summary) does the
# same. Left in, this dilutes Arm B's file cap and pushes the true golden
# file out of the top FILE_CAP results. docs/archive/ holds docs already
# classified non-current (see docs/INVENTORY.md); .out-of-scope/ is a
# non-goal registry, not product content -- neither belongs in an "agent
# reading the repo to answer a question" corpus either.
#
# Applied at both enumeration (git_ls_files, below) and at each arm's point
# of use (arm_b_answer's ranking loop, search_kind's hit collection) so the
# exclusion holds regardless of how file_universe/artifact data was built,
# not only via the one call site that currently constructs it.
# --------------------------------------------------------------------------

EXCLUDED_CORPUS_PREFIXES: Tuple[str, ...] = ("eval/", "docs/archive/", ".out-of-scope/")


def is_excluded_from_corpus(rel_path: str) -> bool:
    """True if `rel_path` (repo-relative) is the benchmark's own corpus,
    its own output, or archived/non-goal material -- never a valid answer
    source for either arm."""
    rel = posix(rel_path)
    return any(rel.startswith(prefix) for prefix in EXCLUDED_CORPUS_PREFIXES)


# --------------------------------------------------------------------------
# Line-range arithmetic shared by both arms' coverage checks
# --------------------------------------------------------------------------


def merge_ranges(ranges: Sequence[Tuple[int, int]]) -> List[Tuple[int, int]]:
    """Merge overlapping/touching inclusive (lo, hi) line ranges, sorted."""
    ordered = sorted(ranges)
    merged: List[List[int]] = []
    for lo, hi in ordered:
        if merged and lo <= merged[-1][1] + 1:
            merged[-1][1] = max(merged[-1][1], hi)
        else:
            merged.append([lo, hi])
    return [(lo, hi) for lo, hi in merged]


def span_fully_covered(start: int, end: int, merged_ranges: Sequence[Tuple[int, int]]) -> bool:
    return any(lo <= start and end <= hi for lo, hi in merged_ranges)


def read_line_window(full_path: Path, start_line: int, end_line: int) -> bytes:
    """Bytes of inclusive 1-indexed lines [start_line, end_line], clamped to the file."""
    text = read_text(full_path)
    lines = text.splitlines(keepends=True)
    n = len(lines)
    lo = max(1, start_line)
    hi = min(n, end_line)
    if lo > hi:
        return b""
    return "".join(lines[lo - 1 : hi]).encode("utf-8")


# --------------------------------------------------------------------------
# Arm A: artifact-guided
# --------------------------------------------------------------------------

# Fixed, deterministic search order for Arm A. Only these three
# native-code-evidence artifact kinds carry a (file, line) pointer at all;
# the rest (files, scorecard, coverage, agent_slice ranking, doctor
# observation, repo.snapshot, inventory.files) are file- or repo-level
# summaries with no line field, so searching them could never change
# whether a pointer is found -- only inflate bytes for no reason. Order is
# precision-first (most granular pointer type tried first), not size-first:
# `code_evidence.chunks` is whole-file-only (startLine is always 1), so
# trying it before `code_evidence.symbols` would let a coarse, low-value
# chunk pointer pre-empt a precise symbol-level one under the "stop at first
# kind with any match" rule below.
ARTIFACT_KIND_ORDER: Tuple[str, ...] = (
    "code_evidence.symbols",
    "code_evidence.imports",
    "code_evidence.chunks",
)


def load_artifact_index(run_dir: Path) -> Dict[str, Path]:
    """Parse run-complete.json -> manifest -> {artifact type: absolute object path}.

    This resolution step (run-complete.json + the manifest object it points
    at) is treated as part of one-time setup, not charged to any question:
    it is fixed plumbing needed to have a queryable corpus at all, not
    evidence a question's search consults.
    """
    run_complete = json.loads(read_bytes(run_dir / "run-complete.json"))
    manifest_rel = run_complete["manifest"]["path"]
    manifest = json.loads(read_bytes(run_dir / manifest_rel))
    index: Dict[str, Path] = {}
    for node in manifest.get("nodes", {}).values():
        for artifact in node.get("artifacts", []):
            atype, apath = artifact.get("type"), artifact.get("path")
            if atype and apath:
                index[atype] = run_dir / apath
    return index


def _searchable_symbol(entry: Dict[str, Any]) -> str:
    return str(entry.get("id", ""))


def _searchable_import(entry: Dict[str, Any]) -> str:
    return f"{entry.get('file', '')}::{entry.get('target', '')}"


def _searchable_chunk(entry: Dict[str, Any]) -> str:
    return str(entry.get("id", ""))


_KIND_SPEC = {
    "code_evidence.symbols": ("symbols", _searchable_symbol, "startLine"),
    "code_evidence.imports": ("imports", _searchable_import, "line"),
    "code_evidence.chunks": ("chunks", _searchable_chunk, "startLine"),
}


def search_kind(kind: str, artifact_json: Dict[str, Any], keywords: Sequence[str]) -> List[Tuple[str, int]]:
    """All (file, line) pointers in one artifact whose searchable text
    contains EVERY keyword (case-insensitive substring, AND semantics).

    AND (not OR) is deliberate: several real symbol/function names in this
    repo collide across files (e.g. `execute` appears in 8+ files), and a
    single generic keyword would resolve to an arbitrary one of them. The
    `id` field folds in the full relative path, so pairing a name with a
    path-fragment keyword disambiguates without inventing a second lookup
    mechanism.
    """
    list_key, text_fn, line_key = _KIND_SPEC[kind]
    entries = artifact_json.get(list_key, [])
    lowered = [k.lower() for k in keywords]
    hits = set()
    for entry in entries:
        text = text_fn(entry).lower()
        if all(k in text for k in lowered):
            line = entry.get(line_key)
            file = entry.get("file")
            if file and isinstance(line, int) and not is_excluded_from_corpus(file):
                hits.add((file, line))
    return sorted(hits)


def arm_a_answer(
    question: Dict[str, Any],
    artifact_index: Dict[str, Path],
    repo_root: Path,
    window: int,
) -> Dict[str, Any]:
    keywords = question["golden"]["keywords"]
    artifact_bytes = 0
    kinds_consulted: List[str] = []
    pointers: List[Tuple[str, int]] = []
    stopped_at: Optional[str] = None

    for kind in ARTIFACT_KIND_ORDER:
        path = artifact_index.get(kind)
        if path is None or not path.is_file():
            continue
        raw = read_bytes(path)
        artifact_bytes += len(raw)
        kinds_consulted.append(kind)
        data = json.loads(raw)
        matches = search_kind(kind, data, keywords)
        if matches:
            pointers = matches
            stopped_at = kind
            break

    if not pointers:
        return {
            "covered": False,
            "reason": "no_pointer_found",
            "bytes_to_answer": artifact_bytes,
            "artifact_bytes": artifact_bytes,
            "span_bytes": 0,
            "artifact_kinds_consulted": kinds_consulted,
            "stopped_at_kind": None,
            "pointers": [],
            "files_touched": list(kinds_consulted),
        }

    # Group resolved pointers by file, merge their +/-window ranges, and
    # read exactly that -- never more, regardless of how interesting the
    # rest of the file might be. This is the "arm separation" invariant
    # eval/test_harness.py checks directly.
    by_file: Dict[str, List[Tuple[int, int]]] = {}
    for file, line in pointers:
        by_file.setdefault(file, []).append((max(1, line - window), line + window))

    span_bytes = 0
    windows_by_file: Dict[str, List[Tuple[int, int]]] = {}
    span_files_touched: List[str] = []
    for file in sorted(by_file):
        merged = merge_ranges(by_file[file])
        windows_by_file[file] = merged
        full_path = repo_root / file
        if not full_path.is_file():
            continue
        file_bytes = 0
        for lo, hi in merged:
            file_bytes += len(read_line_window(full_path, lo, hi))
        span_bytes += file_bytes
        ranges_text = ",".join(f"{lo}-{hi}" for lo, hi in merged)
        span_files_touched.append(f"{posix(file)} (lines {ranges_text})")

    covered = all(
        span["path"] in windows_by_file
        and span_fully_covered(span["start_line"], span["end_line"], windows_by_file[span["path"]])
        for span in question["golden"]["spans"]
    )

    return {
        "covered": covered,
        "reason": None if covered else "pointer_found_window_excludes_golden_span",
        "bytes_to_answer": artifact_bytes + span_bytes,
        "artifact_bytes": artifact_bytes,
        "span_bytes": span_bytes,
        "artifact_kinds_consulted": kinds_consulted,
        "stopped_at_kind": stopped_at,
        "pointers": [{"file": posix(f), "line": l} for f, l in pointers],
        "files_touched": list(kinds_consulted) + span_files_touched,
    }


# --------------------------------------------------------------------------
# Arm B: naive keyword search
# --------------------------------------------------------------------------


def git_ls_files(repo_root: Path) -> List[str]:
    proc = subprocess.run(
        ["git", "ls-files"],
        cwd=str(repo_root),
        capture_output=True,
        text=True,
        check=True,
    )
    files = sorted(
        posix(line)
        for line in proc.stdout.splitlines()
        if line.strip() and not is_excluded_from_corpus(line)
    )
    return files


def _read_text_or_none(path: Path) -> Optional[str]:
    try:
        raw = path.read_bytes()
    except OSError:
        return None
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        return None


def arm_b_answer(
    question: Dict[str, Any],
    repo_root: Path,
    file_universe: Sequence[str],
    file_cap: int,
) -> Dict[str, Any]:
    keywords = question["golden"]["keywords"]
    patterns = [re.compile(re.escape(k), re.IGNORECASE) for k in keywords]

    # Rank phase: a naive agent's first move is a fast grep-style pass (this
    # project's own docs describe `rg` as exactly that: "quick file listing,
    # text search") to see which files are worth opening -- not a context
    # cost in itself. What gets counted below is only the files it goes on
    # to actually open in full.
    scored: List[Tuple[int, str]] = []
    for rel in file_universe:
        if is_excluded_from_corpus(rel):
            continue
        text = _read_text_or_none(repo_root / rel)
        if text is None:
            continue
        count = sum(len(p.findall(text)) for p in patterns)
        if count > 0:
            scored.append((count, rel))
    scored.sort(key=lambda t: (-t[0], t[1]))  # hit count desc, path asc: deterministic
    selected = scored[:file_cap]

    bytes_to_answer = 0
    files_touched: List[str] = []
    covered_paths = set()
    for count, rel in selected:
        raw = read_bytes(repo_root / rel)
        bytes_to_answer += len(raw)
        files_touched.append(f"{rel} ({count} hits)")
        covered_paths.add(rel)

    covered = all(span["path"] in covered_paths for span in question["golden"]["spans"])
    if covered:
        reason = None
    elif not scored:
        reason = "no_keyword_matches"
    else:
        reason = "golden_file_outside_top_cap"

    return {
        "covered": covered,
        "reason": reason,
        "bytes_to_answer": bytes_to_answer,
        "files_matched_total": len(scored),
        "files_touched": files_touched,
    }


# --------------------------------------------------------------------------
# Per-question orchestration
# --------------------------------------------------------------------------


def decide_winner(arm_a: Dict[str, Any], arm_b: Dict[str, Any]) -> str:
    a_ok, b_ok = arm_a["covered"], arm_b["covered"]
    if a_ok and not b_ok:
        return "A"
    if b_ok and not a_ok:
        return "B"
    if not a_ok and not b_ok:
        return "neither"
    a_bytes, b_bytes = arm_a["bytes_to_answer"], arm_b["bytes_to_answer"]
    if a_bytes < b_bytes:
        return "A"
    if b_bytes < a_bytes:
        return "B"
    return "tie"


def answer_question(
    question: Dict[str, Any],
    artifact_index: Dict[str, Path],
    repo_root: Path,
    file_universe: Sequence[str],
    window: int,
    file_cap: int,
) -> Dict[str, Any]:
    arm_a = arm_a_answer(question, artifact_index, repo_root, window)
    arm_b = arm_b_answer(question, repo_root, file_universe, file_cap)
    return {
        "id": question["id"],
        "category": question["category"],
        "question": question["question"],
        "arm_a": arm_a,
        "arm_b": arm_b,
        "winner": decide_winner(arm_a, arm_b),
    }
