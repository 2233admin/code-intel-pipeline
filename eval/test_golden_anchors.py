#!/usr/bin/env python3
"""Self-tests for eval/golden_anchors.py (issue #93 golden-symbol-anchors).

Split out of eval/test_harness.py so neither file trips the sentrux
god-file ratchet (`functions > 25 && loc > 400`, or `loc > 800`;
crates/code-intel-cli/src/sentrux_gate.rs:738) -- the same reason
eval/arms.py and eval/reporting.py were split out of eval/harness.py itself
(see eval/harness.py's module docstring for that precedent).

Proves:

  * a symbol/snippet anchor resolves to the span its stored offsets
    describe, scoped to the anchor's own file;
  * that resolution follows the code when it moves -- the whole point.
    PR #134 shifted crates/code-intel-cli/src/sentrux_gate.rs's
    evaluate_rules from line 481 to line 537 without changing what it
    does, and that alone dropped the frozen "how" baseline from 5/5 to
    3/5 under the old absolute-line-number spans;
  * a missing or ambiguous anchor fails loudly (AnchorError), and
    resolve_question_spans turns that into a "broken question" rather
    than ever silently scoring it as a miss;
  * every anchor in the real eval/questions.json resolves against the
    live repo tree except the two known, expected exceptions (q01/q02:
    real code PR #134 moved out of main.rs entirely, not line rot).

Stdlib unittest only (no pytest). Run directly:
    python eval/test_golden_anchors.py
"""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

EVAL_DIR = Path(__file__).resolve().parent


def _load_harness():
    spec = importlib.util.spec_from_file_location(
        "eval_harness_under_test_for_anchors", EVAL_DIR / "harness.py"
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


harness = _load_harness()
import golden_anchors  # noqa: E402  (eval/ is on sys.path once harness.py has executed)


class GoldenAnchorTests(unittest.TestCase):
    """eval/golden_anchors.py: golden spans resolve by content, not by a
    pinned absolute line number, so an unrelated refactor that moves code
    cannot silently rot the benchmark."""

    def _write_repo(self, tmp: Path, content: str) -> None:
        (tmp / "target.py").write_text(content, encoding="utf-8")

    def test_symbol_anchor_resolves_to_the_current_declaration_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(root, "x = 1\n\n\ndef add(a, b):\n    return a + b\n")
            span = golden_anchors.resolve_anchor(
                root, "target.py", {"kind": "symbol", "symbol_kind": "function", "name": "add", "after": 1}
            )
            self.assertEqual(span, (4, 5))

    def test_symbol_anchor_survives_a_simulated_line_shift(self):
        # The whole point of this mechanism: inject blank lines above the
        # anchored declaration (simulating an unrelated upstream edit, the
        # exact shape of what PR #134 did to sentrux_gate.rs) and prove the
        # resolved span follows the declaration to its new line rather than
        # staying pinned to the old one.
        before_shift = "x = 1\n\n\ndef add(a, b):\n    return a + b\n"
        anchor = {"kind": "symbol", "symbol_kind": "function", "name": "add", "after": 1}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(root, before_shift)
            original_span = golden_anchors.resolve_anchor(root, "target.py", anchor)
            self.assertEqual(original_span, (4, 5))

            shift = 56  # matches the magnitude of the real evaluate_rules regression
            shifted = ("\n" * shift) + before_shift
            self._write_repo(root, shifted)
            shifted_span = golden_anchors.resolve_anchor(root, "target.py", anchor)

        self.assertEqual(shifted_span, (4 + shift, 5 + shift))
        self.assertNotEqual(shifted_span, original_span)

    def test_static_anchor_resolves_bare_pub_and_mut_declarations(self):
        # Regression: "static " is both a modifier _strip_modifiers strips
        # (Java-style `private static void method()`) and a valid
        # symbol_kind in its own right (Rust's `static NAME: Type = ...`).
        # Stripping it unconditionally before checking for it meant a
        # symbol_kind="static" anchor could never resolve, even a bare one
        # with no other modifier -- exactly the kind of silent broken
        # capability this whole mechanism exists to catch.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(
                root,
                "static BARE: u32 = 1;\n"
                "pub static VISIBLE: u32 = 2;\n"
                "pub(crate) static mut COUNTER: u32 = 3;\n",
            )
            bare = golden_anchors.resolve_anchor(
                root, "target.py", {"kind": "symbol", "symbol_kind": "static", "name": "BARE"}
            )
            self.assertEqual(bare, (1, 1))

            visible = golden_anchors.resolve_anchor(
                root, "target.py", {"kind": "symbol", "symbol_kind": "static", "name": "VISIBLE"}
            )
            self.assertEqual(visible, (2, 2))

            counter = golden_anchors.resolve_anchor(
                root, "target.py", {"kind": "symbol", "symbol_kind": "static", "name": "COUNTER"}
            )
            self.assertEqual(counter, (3, 3))

    def test_snippet_anchor_resolves_by_literal_text_not_position(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(
                root,
                "# filler\n# filler\n# a distinctive rationale phrase here\n# filler\n",
            )
            span = golden_anchors.resolve_anchor(
                root,
                "target.py",
                {"kind": "snippet", "text": "distinctive rationale phrase", "before": 1, "after": 1},
            )
            self.assertEqual(span, (2, 4))

    def test_missing_symbol_fails_loudly_instead_of_returning_a_wrong_span(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(root, "def other(): pass\n")
            with self.assertRaises(golden_anchors.AnchorError):
                golden_anchors.resolve_anchor(
                    root, "target.py", {"kind": "symbol", "symbol_kind": "function", "name": "add"}
                )

    def test_ambiguous_snippet_fails_loudly_instead_of_picking_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(root, "marker\nmarker\n")
            with self.assertRaises(golden_anchors.AnchorError):
                golden_anchors.resolve_anchor(
                    root, "target.py", {"kind": "snippet", "text": "marker"}
                )

    def test_resolve_question_spans_reports_the_question_as_broken_on_failure(self):
        question = {
            "id": "qx",
            "golden": {
                "spans": [
                    {
                        "path": "target.py",
                        "anchor": {"kind": "symbol", "symbol_kind": "function", "name": "missing"},
                    }
                ]
            },
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._write_repo(root, "def present(): pass\n")
            resolved, error = golden_anchors.resolve_question_spans(root, question)
        self.assertIsNone(resolved)
        self.assertIn("qx", error)
        self.assertIn("missing", error)

    def test_resolve_question_spans_passes_through_legacy_absolute_spans_unchanged(self):
        # eval/fixtures/questions.json's tiny synthetic questions still use
        # the pre-migration absolute shape; resolution must be a no-op for
        # those rather than requiring every consumer to migrate at once.
        question = {
            "id": "qy",
            "golden": {"spans": [{"path": "target.py", "start_line": 3, "end_line": 5}]},
        }
        resolved, error = golden_anchors.resolve_question_spans(Path("/nonexistent"), question)
        self.assertIsNone(error)
        self.assertEqual(resolved, [{"path": "target.py", "start_line": 3, "end_line": 5}])


class RealQuestionsAnchorTests(unittest.TestCase):
    """The migrated eval/questions.json itself, graded against the live tree."""

    def test_real_questions_spans_are_all_anchor_based(self):
        questions = harness.load_questions(EVAL_DIR / "questions.json")
        for q in questions:
            for span in q["golden"]["spans"]:
                self.assertIn("anchor", span, f"{q['id']}: span must be anchor-based: {span}")
                self.assertIn(span["anchor"]["kind"], {"symbol", "snippet"})

    def test_real_questions_anchors_all_resolve_against_the_live_tree(self):
        # The whole point: every golden anchor in the real question set must
        # resolve against the actual current repo, or it's a broken question
        # (see GoldenAnchorTests above). q01 and q02 are the one known,
        # expected exception: PR #134 moved is_primary_invocation and the
        # RAW_ROUTES table out of main.rs entirely, which is a real code
        # move this resolver correctly refuses to paper over -- it must
        # report exactly those two broken, and nothing else.
        questions = {q["id"]: q for q in harness.load_questions(EVAL_DIR / "questions.json")}
        repo_root = EVAL_DIR.parent
        broken = {}
        for qid, question in questions.items():
            _, error = golden_anchors.resolve_question_spans(repo_root, question)
            if error:
                broken[qid] = error
        self.assertEqual(
            set(broken), {"q01", "q02"}, f"unexpected broken/resolved anchors: {broken}"
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
