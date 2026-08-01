#!/usr/bin/env python3
"""North-star eval v1 -- artifact-guided (Arm A) vs naive (Arm B), issue #93.

Measures how many bytes an agent must consult to answer 12 fixed questions
about this repository two ways:

  Arm A "artifact-guided": run this repo's own `code-intel` pipeline once
  (`code-intel . --mode lite`) to produce its native-code-evidence artifacts,
  then per question search ONLY those artifacts for a pointer (file, line),
  then read ONLY a fixed +/-10 line window around that pointer from the
  source file. If no pointer is found, or the window it finds does not reach
  every golden span, Arm A is recorded covered:false -- it never falls back
  to reading more of the file, and never falls back to Arm B's strategy.

  Arm B "naive": `git ls-files`-enumerate the repo, regex-search (stdlib
  `re`) each file's full text for the question's keywords, rank matched
  files deterministically (hit count desc, path asc), and read up to 5 of
  them in FULL.

Both arms consume the identical `golden.keywords` per question; they differ
in what corpus they search (a small structured index vs. raw repo text) and
at what granularity they read (a bounded window vs. whole files).

Determinism: two consecutive runs on an unchanged working tree must produce
byte-identical `eval/baseline-<shortsha>.json` content, except the top-level
`meta` object (timestamps, wall-clock durations, and the nonce-bearing
pipeline run directory live there and only there). Everything under
`setup` (excluding `meta`), `questions`, and `aggregate` is derived only
from static repo content plus this file's fixed algorithm.

Stdlib only. No network. No LLM calls: the one-time pipeline run uses
`--mode lite`, which maps to `execution_policy::RunProfile::Offline` and
structurally disables the repowise/understand/graph/sentrux providers
(verified against crates/code-intel-cli/src/execution_policy.rs) -- there is
no code path in a lite run that reaches the network.

Usage:
    python eval/harness.py                    # full run: build, scan, answer, write JSON + MD
    python eval/harness.py --no-build          # skip `cargo build` (assume target/release exists)
    python eval/harness.py --render            # only regenerate BASELINE.md from the latest baseline JSON
    python eval/harness.py --artifact-root DIR # override the pipeline's --artifact-root (default: <repo>/artifacts)
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

# --------------------------------------------------------------------------
# Constants
# --------------------------------------------------------------------------

SCHEMA = "code-intel-eval-baseline.v1"
WINDOW = 10  # Arm A: fixed +/-N line window around a resolved pointer.
FILE_CAP = 5  # Arm B: read at most this many ranked, matched files in full.

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

DEFAULT_ARTIFACT_DIRNAME = "artifacts"


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
# Questions
# --------------------------------------------------------------------------


def load_questions(path: Path) -> List[Dict[str, Any]]:
    data = json.loads(read_bytes(path))
    if not isinstance(data, list) or not data:
        raise ValueError(f"{path}: expected a non-empty JSON array of questions")
    seen_ids = set()
    for q in data:
        for key in ("id", "question", "category", "golden"):
            if key not in q:
                raise ValueError(f"question missing required key {key!r}: {q}")
        golden = q["golden"]
        for key in ("paths", "spans", "keywords"):
            if key not in golden:
                raise ValueError(f"question {q['id']}: golden missing {key!r}")
        if not golden["keywords"]:
            raise ValueError(f"question {q['id']}: golden.keywords must be non-empty")
        if not golden["spans"]:
            raise ValueError(f"question {q['id']}: golden.spans must be non-empty")
        if q["id"] in seen_ids:
            raise ValueError(f"duplicate question id {q['id']!r}")
        seen_ids.add(q["id"])
    return data


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
            if file and isinstance(line, int):
                hits.add((file, line))
    return sorted(hits)


def arm_a_answer(
    question: Dict[str, Any],
    artifact_index: Dict[str, Path],
    repo_root: Path,
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

    # Group resolved pointers by file, merge their +/-WINDOW ranges, and
    # read exactly that -- never more, regardless of how interesting the
    # rest of the file might be. This is the "arm separation" invariant
    # eval/test_harness.py checks directly.
    by_file: Dict[str, List[Tuple[int, int]]] = {}
    for file, line in pointers:
        by_file.setdefault(file, []).append((max(1, line - WINDOW), line + WINDOW))

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
    files = sorted(posix(line) for line in proc.stdout.splitlines() if line.strip())
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
        text = _read_text_or_none(repo_root / rel)
        if text is None:
            continue
        count = sum(len(p.findall(text)) for p in patterns)
        if count > 0:
            scored.append((count, rel))
    scored.sort(key=lambda t: (-t[0], t[1]))  # hit count desc, path asc: deterministic
    selected = scored[:FILE_CAP]

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
# Per-question orchestration + aggregation
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
) -> Dict[str, Any]:
    arm_a = arm_a_answer(question, artifact_index, repo_root)
    arm_b = arm_b_answer(question, repo_root, file_universe)
    return {
        "id": question["id"],
        "category": question["category"],
        "question": question["question"],
        "arm_a": arm_a,
        "arm_b": arm_b,
        "winner": decide_winner(arm_a, arm_b),
    }


def aggregate(results: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    total = len(results)

    def arm_summary(key: str) -> Dict[str, Any]:
        covered = [r for r in results if r[key]["covered"]]
        total_bytes = sum(r[key]["bytes_to_answer"] for r in results)
        covered_bytes = sum(r[key]["bytes_to_answer"] for r in covered)
        return {
            "covered_count": len(covered),
            "coverage_rate": round(len(covered) / total, 4) if total else 0.0,
            "total_bytes_all_questions": total_bytes,
            "total_bytes_when_covered": covered_bytes,
            "mean_bytes_when_covered": (
                round(covered_bytes / len(covered), 2) if covered else None
            ),
        }

    categories = sorted({r["category"] for r in results})
    by_category = {}
    for cat in categories:
        rows = [r for r in results if r["category"] == cat]
        by_category[cat] = {
            "count": len(rows),
            "arm_a_covered": sum(1 for r in rows if r["arm_a"]["covered"]),
            "arm_b_covered": sum(1 for r in rows if r["arm_b"]["covered"]),
        }

    reasons_a: Dict[str, int] = {}
    for r in results:
        reason = r["arm_a"]["reason"]
        if reason:
            reasons_a[reason] = reasons_a.get(reason, 0) + 1
    reasons_b: Dict[str, int] = {}
    for r in results:
        reason = r["arm_b"]["reason"]
        if reason:
            reasons_b[reason] = reasons_b.get(reason, 0) + 1

    win_loss = {
        "a_wins": sum(1 for r in results if r["winner"] == "A"),
        "b_wins": sum(1 for r in results if r["winner"] == "B"),
        "ties": sum(1 for r in results if r["winner"] == "tie"),
        "neither_covered": sum(1 for r in results if r["winner"] == "neither"),
    }

    return {
        "total_questions": total,
        "arm_a": arm_summary("arm_a"),
        "arm_b": arm_summary("arm_b"),
        "by_category": by_category,
        "arm_a_failure_reasons": reasons_a,
        "arm_b_failure_reasons": reasons_b,
        "win_loss": win_loss,
        "win_loss_table": [
            {
                "id": r["id"],
                "category": r["category"],
                "arm_a_covered": r["arm_a"]["covered"],
                "arm_a_bytes": r["arm_a"]["bytes_to_answer"],
                "arm_b_covered": r["arm_b"]["covered"],
                "arm_b_bytes": r["arm_b"]["bytes_to_answer"],
                "winner": r["winner"],
            }
            for r in results
        ],
    }


# --------------------------------------------------------------------------
# Pipeline setup (build + one run) -- the amortized, non-deterministic-timed part
# --------------------------------------------------------------------------


def binary_path(repo_root: Path) -> Path:
    name = "code-intel.exe" if os.name == "nt" else "code-intel"
    return repo_root / "target" / "release" / name


def build_pipeline(repo_root: Path, env: Dict[str, str], skip: bool) -> Dict[str, Any]:
    if skip:
        if not binary_path(repo_root).is_file():
            raise SystemExit(
                f"--no-build given but {binary_path(repo_root)} does not exist; "
                "build it first with `cargo build -p code-intel --release --locked`."
            )
        return {"skipped": True, "seconds": 0.0}
    t0 = time.time()
    proc = subprocess.run(
        ["cargo", "build", "-p", "code-intel", "--release", "--locked"],
        cwd=str(repo_root),
        env=env,
        capture_output=True,
        text=True,
    )
    elapsed = time.time() - t0
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"cargo build failed (exit {proc.returncode})")
    return {"skipped": False, "seconds": round(elapsed, 3)}


def run_pipeline(
    repo_root: Path, artifact_root: Path, env: Dict[str, str]
) -> Tuple[Dict[str, Any], Path, float]:
    if artifact_root.exists():
        shutil.rmtree(artifact_root)
    artifact_root.mkdir(parents=True, exist_ok=True)

    # Defense in depth: --mode lite already disables every optional
    # provider at the ExecutionPolicy level (RunProfile::Offline), but this
    # removes any chance of an accidental network call from this process's
    # environment regardless.
    clean_env = dict(env)
    clean_env.pop("CODE_INTEL_ANTHROPIC_API_KEY", None)
    clean_env.pop("CODE_INTEL_ANTHROPIC_BASE_URL", None)

    t0 = time.time()
    proc = subprocess.run(
        [
            str(binary_path(repo_root)),
            str(repo_root),
            "--mode",
            "lite",
            "--artifact-root",
            str(artifact_root),
            "--json",
        ],
        cwd=str(repo_root),
        env=clean_env,
        capture_output=True,
        text=True,
    )
    elapsed = time.time() - t0
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"pipeline run failed (exit {proc.returncode})")
    result = json.loads(proc.stdout)
    if result.get("outcome") != "completed":
        raise SystemExit(f"pipeline outcome was not 'completed': {result}")
    run_dir = Path(result["publication"]["path"])
    if not run_dir.is_absolute():
        run_dir = repo_root / run_dir
    return result, run_dir, elapsed


def compute_dir_bytes(run_dir: Path) -> int:
    total = 0
    for p in run_dir.rglob("*"):
        if p.is_file():
            total += p.stat().st_size
    return total


# --------------------------------------------------------------------------
# git helpers
# --------------------------------------------------------------------------


def git_short_head(repo_root: Path) -> str:
    proc = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        cwd=str(repo_root),
        capture_output=True,
        text=True,
        check=True,
    )
    return proc.stdout.strip()


# --------------------------------------------------------------------------
# Rendering (deterministic, derived from a baseline JSON dict -- no timestamps)
# --------------------------------------------------------------------------


def render_markdown(baseline: Dict[str, Any]) -> str:
    agg = baseline["aggregate"]
    a, b = agg["arm_a"], agg["arm_b"]
    total = agg["total_questions"]

    lines: List[str] = []
    lines.append("# Eval v1 baseline -- artifact-guided (A) vs naive (B)")
    lines.append("")
    lines.append(
        f"Generated by `python eval/harness.py --render` from "
        f"`{baseline['meta']['baseline_file']}`. Do not hand-edit; re-run harness.py instead."
    )
    lines.append("")
    lines.append(f"HEAD: `{baseline['meta']['repo_head_sha']}` | questions: {total}")
    lines.append("")
    lines.append("| id | category | A covered | A bytes | B covered | B bytes | winner |")
    lines.append("|---|---|---|---|---|---|---|")
    for row in agg["win_loss_table"]:
        lines.append(
            f"| {row['id']} | {row['category']} | "
            f"{'yes' if row['arm_a_covered'] else 'no'} | {row['arm_a_bytes']:,} | "
            f"{'yes' if row['arm_b_covered'] else 'no'} | {row['arm_b_bytes']:,} | "
            f"{row['winner']} |"
        )
    lines.append("")
    lines.append("## Findings")
    lines.append("")

    wl = agg["win_loss"]
    a_reasons = ", ".join(f"{k}={v}" for k, v in sorted(agg["arm_a_failure_reasons"].items())) or "none"
    b_reasons = ", ".join(f"{k}={v}" for k, v in sorted(agg["arm_b_failure_reasons"].items())) or "none"
    cat_bits = "; ".join(
        f"{cat}: A {row['arm_a_covered']}/{row['count']}, B {row['arm_b_covered']}/{row['count']}"
        for cat, row in sorted(agg["by_category"].items())
    )

    lines.append(
        f"1. Arm A covered {a['covered_count']}/{total} questions "
        f"({a['total_bytes_when_covered']:,} bytes over those, "
        f"{a['total_bytes_all_questions']:,} bytes total including failed searches); "
        f"Arm B covered {b['covered_count']}/{total} "
        f"({b['total_bytes_when_covered']:,} bytes over those, "
        f"{b['total_bytes_all_questions']:,} bytes total)."
    )
    lines.append(
        f"2. Head-to-head: A wins {wl['a_wins']}, B wins {wl['b_wins']}, "
        f"ties {wl['ties']}, neither covered {wl['neither_covered']} (out of {total})."
    )
    lines.append(f"3. Arm A failure reasons: {a_reasons}. Arm B failure reasons: {b_reasons}.")
    lines.append(f"4. By category (covered/total): {cat_bits}.")
    setup = baseline["setup"]
    meta_timing = baseline["meta"]["timing"]
    lines.append(
        f"5. One-time setup: `cargo build --release` + `code-intel . --mode lite` produced a "
        f"{setup['setup_bytes']:,}-byte artifact corpus in "
        f"{meta_timing['build_seconds'] + meta_timing['pipeline_run_seconds']:.1f}s "
        "(amortized across all 12 questions, not charged per-question)."
    )
    lines.append("")
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------
# Top-level run
# --------------------------------------------------------------------------


def _clean_stale_eval_outputs(eval_dir: Path) -> None:
    """Remove this script's own previous outputs before the pipeline scans
    the working tree.

    Self-hosting gotcha: `code-intel . --mode lite` scans the whole working
    tree, including untracked-but-not-gitignored files -- which is exactly
    what an as-yet-uncommitted `eval/baseline-<sha>.json` and
    `eval/BASELINE.md` are. Leaving a previous run's output on disk means
    the *next* run's evidence.native-code artifacts (file/chunk counts,
    hence their byte sizes) differ from the first run's purely because the
    corpus grew by two files that did not exist yet -- a real, measured
    determinism break (see TASK_REPORT.md). Deleting them here, before
    `run_pipeline`, and writing the new ones only after every question has
    already been answered, makes each run's scan see the same input every
    time regardless of what a previous invocation left behind.
    """
    for stale in eval_dir.glob("baseline-*.json"):
        stale.unlink()
    baseline_md = eval_dir / "BASELINE.md"
    if baseline_md.is_file():
        baseline_md.unlink()


def build_baseline(
    repo_root: Path,
    eval_dir: Path,
    questions_path: Path,
    artifact_root: Path,
    no_build: bool,
) -> Dict[str, Any]:
    env = os.environ.copy()
    env["CODE_INTEL_HOME"] = str(repo_root)

    questions = load_questions(questions_path)

    build_info = build_pipeline(repo_root, env, skip=no_build)
    _clean_stale_eval_outputs(eval_dir)
    pipeline_result, run_dir, run_seconds = run_pipeline(repo_root, artifact_root, env)
    artifact_index = load_artifact_index(run_dir)
    setup_bytes = compute_dir_bytes(run_dir)

    file_universe = git_ls_files(repo_root)

    results = [
        answer_question(q, artifact_index, repo_root, file_universe) for q in questions
    ]
    agg = aggregate(results)

    head_sha = git_short_head(repo_root)
    baseline_filename = f"baseline-{head_sha}.json"

    baseline = {
        "schema": SCHEMA,
        "setup": {
            "pipeline_mode": "lite",
            "pipeline_outcome": pipeline_result.get("outcome"),
            "artifact_kind_order": list(ARTIFACT_KIND_ORDER),
            "window_lines": WINDOW,
            "file_cap": FILE_CAP,
            "setup_bytes": setup_bytes,
            "repo_file_count": len(file_universe),
        },
        "questions": results,
        "aggregate": agg,
        "meta": {
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "python_version": sys.version.split()[0],
            "platform": sys.platform,
            "repo_head_sha": head_sha,
            "baseline_file": f"eval/{baseline_filename}",
            "artifact_run_dir": posix(str(run_dir.relative_to(repo_root)))
            if run_dir.is_relative_to(repo_root)
            else posix(str(run_dir)),
            "timing": {
                "build_seconds": build_info["seconds"],
                "build_skipped": build_info["skipped"],
                "pipeline_run_seconds": round(run_seconds, 3),
            },
        },
    }
    return baseline


def write_baseline(baseline: Dict[str, Any], eval_dir: Path) -> Path:
    out_path = eval_dir / Path(baseline["meta"]["baseline_file"]).name
    out_path.write_text(
        json.dumps(baseline, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    return out_path


def find_latest_baseline(eval_dir: Path, head_sha: Optional[str]) -> Path:
    if head_sha:
        candidate = eval_dir / f"baseline-{head_sha}.json"
        if candidate.is_file():
            return candidate
    candidates = sorted(eval_dir.glob("baseline-*.json"), key=lambda p: p.stat().st_mtime)
    if not candidates:
        raise SystemExit(f"no eval/baseline-*.json found under {eval_dir}")
    return candidates[-1]


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", default=None, help="repo root (default: parent of eval/)")
    parser.add_argument("--questions", default=None, help="path to questions.json")
    parser.add_argument(
        "--artifact-root", default=None, help="pipeline --artifact-root (default: <repo>/artifacts)"
    )
    parser.add_argument("--no-build", action="store_true", help="skip `cargo build`")
    parser.add_argument(
        "--render", action="store_true", help="only regenerate BASELINE.md from an existing baseline JSON"
    )
    parser.add_argument("--baseline-file", default=None, help="baseline JSON to render (with --render)")
    args = parser.parse_args(argv)

    eval_dir = Path(__file__).resolve().parent
    repo_root = Path(args.repo_root).resolve() if args.repo_root else eval_dir.parent
    questions_path = Path(args.questions).resolve() if args.questions else eval_dir / "questions.json"

    if args.render:
        baseline_path = (
            Path(args.baseline_file).resolve()
            if args.baseline_file
            else find_latest_baseline(eval_dir, git_short_head(repo_root))
        )
        baseline = json.loads(read_bytes(baseline_path))
        md = render_markdown(baseline)
        (eval_dir / "BASELINE.md").write_text(md, encoding="utf-8")
        print(f"wrote {eval_dir / 'BASELINE.md'} from {baseline_path}")
        return 0

    artifact_root = (
        Path(args.artifact_root).resolve() if args.artifact_root else repo_root / DEFAULT_ARTIFACT_DIRNAME
    )

    baseline = build_baseline(repo_root, eval_dir, questions_path, artifact_root, args.no_build)
    out_path = write_baseline(baseline, eval_dir)
    md = render_markdown(baseline)
    (eval_dir / "BASELINE.md").write_text(md, encoding="utf-8")

    agg = baseline["aggregate"]
    print(
        f"wrote {out_path} and {eval_dir / 'BASELINE.md'}\n"
        f"Arm A: {agg['arm_a']['covered_count']}/{agg['total_questions']} covered, "
        f"{agg['arm_a']['total_bytes_all_questions']:,} bytes total\n"
        f"Arm B: {agg['arm_b']['covered_count']}/{agg['total_questions']} covered, "
        f"{agg['arm_b']['total_bytes_all_questions']:,} bytes total\n"
        f"win/loss: A={agg['win_loss']['a_wins']} B={agg['win_loss']['b_wins']} "
        f"tie={agg['win_loss']['ties']} neither={agg['win_loss']['neither_covered']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
