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
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

# --------------------------------------------------------------------------
# eval/arms.py holds both arms' search/read logic (pointer resolution,
# windowed reads, naive keyword search); eval/reporting.py holds
# aggregate()/render_markdown(). Split out under issue #93 so no single file
# in eval/ trips the sentrux god-file ratchet (`functions > 25 && loc > 400`,
# or `loc > 800`; crates/code-intel-cli/src/sentrux_gate.rs:738). This file
# stays the CLI entry point + orchestration only.
#
# The sys.path insert is needed because this file is loaded two different
# ways that never register it in sys.modules under the literal name
# "harness": as `__main__` when run directly, and under a synthetic name
# when eval/test_harness.py loads it via importlib.util.spec_from_file_location.
# Either way, "eval/" is on sys.path (script dir for the former, test
# runner's script dir for the latter), so a plain `import arms` resolves --
# but only after this insert makes that true regardless of invocation.
# eval/arms.py and eval/reporting.py must never import back from this
# module: doing so would trigger a *second*, independent import of this file
# under the literal name "harness" (fresh module object, fresh globals),
# instead of finding the one already running -- silently breaking the
# `harness.FILE_CAP = 1` mutation eval/test_harness.py relies on below.
# --------------------------------------------------------------------------
_EVAL_DIR = Path(__file__).resolve().parent
if str(_EVAL_DIR) not in sys.path:
    sys.path.insert(0, str(_EVAL_DIR))

from arms import (  # noqa: E402
    ARTIFACT_KIND_ORDER,
    answer_question as _answer_question_impl,
    arm_a_answer as _arm_a_answer_impl,
    arm_b_answer as _arm_b_answer_impl,
    git_ls_files,
    load_artifact_index,
    merge_ranges,
    posix,
    read_bytes,
    read_line_window,
    span_fully_covered,
)
from reporting import aggregate, render_markdown  # noqa: E402

# --------------------------------------------------------------------------
# Constants
# --------------------------------------------------------------------------

SCHEMA = "code-intel-eval-baseline.v1"
WINDOW = 10  # Arm A: fixed +/-N line window around a resolved pointer.
FILE_CAP = 5  # Arm B: read at most this many ranked, matched files in full.

DEFAULT_ARTIFACT_DIRNAME = "artifacts"


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
# Arm wrappers -- eval/test_harness.py mutates `FILE_CAP` on this module
# directly (`harness.FILE_CAP = 1`) to test cap exhaustion. These wrappers
# re-read WINDOW/FILE_CAP from this module's own namespace on every call and
# forward to eval/arms.py's parameterized implementations, so the mutation
# takes effect immediately -- see the sys.path/import comment above for why
# arms.py can't just read these constants back off this module itself.
# --------------------------------------------------------------------------


def arm_a_answer(question: Dict[str, Any], artifact_index: Dict[str, Path], repo_root: Path) -> Dict[str, Any]:
    return _arm_a_answer_impl(question, artifact_index, repo_root, WINDOW)


def arm_b_answer(question: Dict[str, Any], repo_root: Path, file_universe: Sequence[str]) -> Dict[str, Any]:
    return _arm_b_answer_impl(question, repo_root, file_universe, FILE_CAP)


def answer_question(
    question: Dict[str, Any],
    artifact_index: Dict[str, Path],
    repo_root: Path,
    file_universe: Sequence[str],
) -> Dict[str, Any]:
    return _answer_question_impl(question, artifact_index, repo_root, file_universe, WINDOW, FILE_CAP)


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
