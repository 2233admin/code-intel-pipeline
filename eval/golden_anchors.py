#!/usr/bin/env python3
"""Content-addressed golden span resolution for eval/harness.py (issue #93).

`eval/questions.json` used to pin a golden span as an absolute (start_line,
end_line) pair. That rots silently: PR #134 shrank
crates/code-intel-cli/src/main.rs from ~700 lines to 73 and moved
`is_primary_invocation` out of it entirely, and an unrelated commit added
~56 lines earlier in sentrux_gate.rs and pushed `evaluate_rules` from line
481 to line 537 -- neither the question, the code's meaning, nor the code
itself doing the wrong thing changed, only the line count above it. The
"how" category's coverage dropped from 5/5 to 3/5 purely from that.

This module resolves a golden span from a *content* anchor instead:

  * `"kind": "symbol"` -- a declaration this repo's own heuristics would
    recognize (`fn`/`def`/`function` for symbol_kind "function", or a
    `const `/`static `/`struct `-style keyword for others), identified by
    `name`. The span floats with wherever that declaration line is *now*.
  * `"kind": "snippet"` -- a short, literal substring that should appear on
    exactly one line of the file (a phrase from a doc comment, a match-arm
    string, a JSON-field comparison -- anything that isn't itself a
    parseable declaration). The span floats with wherever that text is now.

Either way the span is `[anchor_line - before, anchor_line + after]`,
where `before`/`after` are the same fixed line counts the span was
originally authored with relative to its anchor -- migrating to anchors
does not redefine what a span covers, only what makes it move with the
code.

Resolution must fail loudly, never silently: if the anchor text/symbol is
missing (renamed, deleted, moved to a different file) or ambiguous
(matches more than once), `resolve_question_spans` reports that question as
broken rather than quietly returning a wrong or empty span. A broken
question is graded by neither arm -- see eval/harness.py's `build_baseline`
and eval/reporting.py's `aggregate` for how that's kept out of (and visible
alongside) the coverage numbers.

Stdlib only, no network, matching the rest of eval/.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

_FUNCTION_KEYWORDS: Tuple[str, ...] = ("fn ", "def ", "function ")

# Visibility/modifier prefixes stripped (possibly repeatedly) before matching
# a declaration keyword. Deliberately the same small, honest set
# crates/code-intel-cli/src/native_code_evidence.rs's own line-heuristic
# extractor understands -- this resolver is intentionally independent of
# that Rust code (it must also work for `snippet` anchors, which aren't
# declarations at all), but there's no reason to be less capable at
# stripping a bare `pub(crate)`.
_MODIFIER_PREFIXES: Tuple[str, ...] = (
    "pub(crate) ",
    "pub(super) ",
    "pub ",
    "public ",
    "private ",
    "protected ",
    "static ",
    "async ",
    "export ",
)


class AnchorError(ValueError):
    """A golden anchor could not be resolved against the current tree."""


def _strip_modifiers(line: str) -> str:
    changed = True
    while changed:
        changed = False
        for prefix in _MODIFIER_PREFIXES:
            if line.startswith(prefix):
                line = line[len(prefix) :]
                changed = True
    return line


def _declares_symbol(line: str, symbol_kind: str, name: str) -> bool:
    stripped = _strip_modifiers(line.lstrip())
    keywords = _FUNCTION_KEYWORDS if symbol_kind == "function" else (f"{symbol_kind} ",)
    for keyword in keywords:
        if not stripped.startswith(keyword):
            continue
        rest = stripped[len(keyword) :]
        if not rest.startswith(name):
            continue
        boundary = rest[len(name) : len(name) + 1]
        if boundary == "" or not (boundary.isalnum() or boundary == "_"):
            return True
    return False


def _read_lines(repo_root: Path, path: str) -> List[str]:
    full_path = repo_root / path
    if not full_path.is_file():
        raise AnchorError(f"{path}: file does not exist")
    text = full_path.read_bytes().decode("utf-8", errors="strict")
    return text.splitlines()


def _find_symbol_line(lines: List[str], symbol_kind: str, name: str) -> int:
    matches = [
        index + 1 for index, line in enumerate(lines) if _declares_symbol(line, symbol_kind, name)
    ]
    if not matches:
        raise AnchorError(f"no {symbol_kind} declaration named {name!r} found")
    if len(matches) > 1:
        raise AnchorError(
            f"{symbol_kind} {name!r} is ambiguous: {len(matches)} declarations at lines {matches}"
        )
    return matches[0]


def _find_snippet_line(lines: List[str], text: str) -> int:
    matches = [index + 1 for index, line in enumerate(lines) if text in line]
    if not matches:
        raise AnchorError(f"snippet {text!r} not found")
    if len(matches) > 1:
        raise AnchorError(f"snippet {text!r} is ambiguous: appears at lines {matches}")
    return matches[0]


def resolve_anchor(repo_root: Path, path: str, anchor: Dict[str, Any]) -> Tuple[int, int]:
    """Resolve one anchor to a concrete (start_line, end_line), or raise
    AnchorError with a message identifying exactly what failed to resolve
    and why."""
    lines = _read_lines(repo_root, path)
    kind = anchor.get("kind")
    before = int(anchor.get("before", 0))
    after = int(anchor.get("after", 0))

    if kind == "symbol":
        symbol_kind = anchor.get("symbol_kind")
        name = anchor.get("name")
        if not symbol_kind or not name:
            raise AnchorError("symbol anchor requires 'symbol_kind' and 'name'")
        anchor_line = _find_symbol_line(lines, symbol_kind, name)
    elif kind == "snippet":
        text = anchor.get("text")
        if not text:
            raise AnchorError("snippet anchor requires 'text'")
        anchor_line = _find_snippet_line(lines, text)
    else:
        raise AnchorError(f"unknown anchor kind {kind!r}")

    start_line = max(1, anchor_line - before)
    end_line = min(len(lines), anchor_line + after)
    if start_line > end_line:
        raise AnchorError(
            f"resolved span [{start_line}, {end_line}] around anchor line {anchor_line} is empty"
        )
    return start_line, end_line


def resolve_question_spans(
    repo_root: Path, question: Dict[str, Any]
) -> Tuple[Optional[List[Dict[str, Any]]], Optional[str]]:
    """Resolve every span in `question["golden"]["spans"]`.

    A span that already carries a concrete `start_line`/`end_line` (the
    legacy shape, still used by eval/fixtures/questions.json's tiny
    synthetic questions) passes through unresolved -- there is nothing to
    anchor. A span carrying an `anchor` is resolved against the live repo
    tree.

    Returns `(resolved_spans, None)` on success, or `(None, message)` if any
    one span failed to resolve -- one broken anchor breaks the whole
    question, since a partially-graded golden target is exactly the silent
    failure mode this exists to prevent.
    """
    resolved: List[Dict[str, Any]] = []
    for span in question["golden"]["spans"]:
        if "anchor" not in span:
            resolved.append(span)
            continue
        try:
            start_line, end_line = resolve_anchor(repo_root, span["path"], span["anchor"])
        except AnchorError as error:
            return None, f"{question['id']} span ({span['path']}): {error}"
        resolved.append({"path": span["path"], "start_line": start_line, "end_line": end_line})
    return resolved, None
