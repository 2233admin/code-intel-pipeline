#!/usr/bin/env python3
"""Self-tests for eval/harness.py.

Proves, against the tiny fixture tree in eval/fixtures/ (no cargo build, no
real pipeline run, no network, no git):

  * determinism -- repeated calls with the same inputs produce byte-
    identical (JSON-serialized) results;
  * arm separation -- Arm A only ever reads bytes inside pointer +/-WINDOW
    windows, never falls back to reading more of a file or to Arm B's
    strategy, and reports the honest reason when a golden span is missed;
  * coverage logic -- covered/not-covered and winner selection match the
    documented rules for both arms, including the artifact-kind stop rule,
    the chunks fallback, and Arm B's file cap.

Stdlib unittest only (no pytest). Run directly:
    python eval/test_harness.py
"""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

EVAL_DIR = Path(__file__).resolve().parent
FIXTURES_DIR = EVAL_DIR / "fixtures"
FIXTURE_REPO = FIXTURES_DIR / "repo"
FIXTURE_RUN = FIXTURES_DIR / "run"
FIXTURE_FILE_UNIVERSE = ["notes.md", "other.py", "sample.py"]  # sorted, matches git-ls-files shape


def _load_harness():
    spec = importlib.util.spec_from_file_location("eval_harness_under_test", EVAL_DIR / "harness.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


harness = _load_harness()


def _fixture_questions():
    return {q["id"]: q for q in harness.load_questions(FIXTURES_DIR / "questions.json")}


def _fixture_artifact_index():
    return harness.load_artifact_index(FIXTURE_RUN)


class ArmAPointerResolutionTests(unittest.TestCase):
    """Arm A: search-for-pointer + fixed-window-read + coverage decision."""

    def setUp(self):
        self.questions = _fixture_questions()
        self.index = _fixture_artifact_index()

    def test_symbols_hit_disambiguated_by_and_keyword(self):
        result = harness.arm_a_answer(self.questions["fq01"], self.index, FIXTURE_REPO)
        self.assertTrue(result["covered"])
        self.assertIsNone(result["reason"])
        self.assertEqual(result["stopped_at_kind"], "code_evidence.symbols")
        # 'add' collides across sample.py and other.py; pairing it with the
        # path-fragment keyword 'sample' must resolve only sample.py's.
        self.assertEqual(result["pointers"], [{"file": "sample.py", "line": 23}])

    def test_artifact_bytes_equal_exactly_the_one_kind_consulted(self):
        result = harness.arm_a_answer(self.questions["fq01"], self.index, FIXTURE_REPO)
        symbols_size = (FIXTURE_RUN / "objects" / "sha256" / "symbols.json").stat().st_size
        self.assertEqual(result["artifact_kinds_consulted"], ["code_evidence.symbols"])
        self.assertEqual(result["artifact_bytes"], symbols_size)

    def test_imports_hit_stops_search_before_chunks(self):
        result = harness.arm_a_answer(self.questions["fq02"], self.index, FIXTURE_REPO)
        self.assertTrue(result["covered"])
        self.assertEqual(result["stopped_at_kind"], "code_evidence.imports")
        self.assertEqual(
            result["artifact_kinds_consulted"], ["code_evidence.symbols", "code_evidence.imports"]
        )
        self.assertNotIn("code_evidence.chunks", result["artifact_kinds_consulted"])
        symbols_size = (FIXTURE_RUN / "objects" / "sha256" / "symbols.json").stat().st_size
        imports_size = (FIXTURE_RUN / "objects" / "sha256" / "imports.json").stat().st_size
        self.assertEqual(result["artifact_bytes"], symbols_size + imports_size)

    def test_chunks_fallback_when_file_has_no_symbols_or_imports(self):
        result = harness.arm_a_answer(self.questions["fq05"], self.index, FIXTURE_REPO)
        self.assertTrue(result["covered"])
        self.assertEqual(result["stopped_at_kind"], "code_evidence.chunks")
        self.assertEqual(result["artifact_kinds_consulted"], list(harness.ARTIFACT_KIND_ORDER))

    def test_prose_only_keyword_matches_no_structured_field_anywhere(self):
        result = harness.arm_a_answer(self.questions["fq03"], self.index, FIXTURE_REPO)
        self.assertFalse(result["covered"])
        self.assertEqual(result["reason"], "no_pointer_found")
        self.assertEqual(result["pointers"], [])
        # Honesty requirement: a failed search must exhaust every kind, not
        # quietly give up early or substitute a different strategy.
        self.assertEqual(result["artifact_kinds_consulted"], list(harness.ARTIFACT_KIND_ORDER))
        self.assertEqual(result["span_bytes"], 0)

    def test_arm_separation_pointer_found_but_window_excludes_golden_span(self):
        result = harness.arm_a_answer(self.questions["fq04"], self.index, FIXTURE_REPO)
        self.assertFalse(result["covered"])
        self.assertEqual(result["reason"], "pointer_found_window_excludes_golden_span")
        self.assertEqual(result["pointers"], [{"file": "sample.py", "line": 28}])

        # Independently recompute the window this pointer implies and prove
        # Arm A read exactly that -- no more, no less -- and that the
        # window genuinely does not overlap the golden span (lines 1-7).
        sample_path = FIXTURE_REPO / "sample.py"
        line_count = len(sample_path.read_text(encoding="utf-8").splitlines())
        lo = max(1, 28 - harness.WINDOW)
        hi = min(line_count, 28 + harness.WINDOW)
        expected_bytes = len(harness.read_line_window(sample_path, lo, hi))
        self.assertEqual(result["span_bytes"], expected_bytes)
        self.assertFalse(harness.span_fully_covered(1, 7, [(lo, hi)]))

        golden_only_bytes = len(harness.read_line_window(sample_path, 1, 7))
        self.assertNotEqual(result["span_bytes"], golden_only_bytes)


class ArmBNaiveSearchTests(unittest.TestCase):
    """Arm B: regex-over-full-text ranking + capped full-file reads."""

    def setUp(self):
        self.questions = _fixture_questions()

    def test_matched_files_are_read_in_full_and_cover_the_golden_file(self):
        # fq01's keywords ('add', 'sample') both hit sample.py and other.py
        # (2 occurrences each -- 'add' the function name plus a docstring
        # mention in each file); with only 2 files matching and a cap of 5,
        # naive search reads both in full, byte-for-byte.
        result = harness.arm_b_answer(self.questions["fq01"], FIXTURE_REPO, FIXTURE_FILE_UNIVERSE)
        self.assertTrue(result["covered"])
        self.assertIsNone(result["reason"])
        self.assertEqual(result["files_touched"], ["other.py (2 hits)", "sample.py (2 hits)"])
        expected_bytes = (FIXTURE_REPO / "other.py").stat().st_size + (
            FIXTURE_REPO / "sample.py"
        ).stat().st_size
        self.assertEqual(result["bytes_to_answer"], expected_bytes)

    def test_full_text_search_reaches_prose_arm_a_cannot(self):
        # fq03's keyword is prose-only; Arm A cannot resolve any pointer for
        # it (see ArmAPointerResolutionTests), but Arm B's regex sees raw
        # file text and finds it directly in sample.py's docstring.
        result = harness.arm_b_answer(self.questions["fq03"], FIXTURE_REPO, FIXTURE_FILE_UNIVERSE)
        self.assertTrue(result["covered"])

    def test_no_keyword_matches_reports_that_reason(self):
        question = dict(self.questions["fq01"])
        question["golden"] = dict(question["golden"], keywords=["nonexistent_token_xyz"])
        result = harness.arm_b_answer(question, FIXTURE_REPO, FIXTURE_FILE_UNIVERSE)
        self.assertFalse(result["covered"])
        self.assertEqual(result["reason"], "no_keyword_matches")
        self.assertEqual(result["files_touched"], [])
        self.assertEqual(result["bytes_to_answer"], 0)

    def test_cap_exhaustion_excludes_a_lower_ranked_golden_file(self):
        # other.py matches 'add' twice (definition + comment), sample.py
        # once, so with the cap forced to 1 only other.py survives.
        original_cap = harness.FILE_CAP
        harness.FILE_CAP = 1
        try:
            result = harness.arm_b_answer(self.questions["fq06"], FIXTURE_REPO, FIXTURE_FILE_UNIVERSE)
        finally:
            harness.FILE_CAP = original_cap
        self.assertFalse(result["covered"])
        self.assertEqual(result["reason"], "golden_file_outside_top_cap")
        self.assertEqual(len(result["files_touched"]), 1)
        self.assertTrue(result["files_touched"][0].startswith("other.py"))


class WinnerSelectionTests(unittest.TestCase):
    def setUp(self):
        self.questions = _fixture_questions()
        self.index = _fixture_artifact_index()

    def test_winner_is_a_when_only_a_covers(self):
        # Starve Arm B by excluding the golden file from its universe
        # entirely; Arm A is independent of file_universe and still covers.
        result = harness.answer_question(self.questions["fq01"], self.index, FIXTURE_REPO, ["other.py"])
        self.assertTrue(result["arm_a"]["covered"])
        self.assertFalse(result["arm_b"]["covered"])
        self.assertEqual(result["winner"], "A")

    def test_winner_is_b_when_only_b_covers(self):
        # The mirror image, and the whole benchmark's thesis in miniature:
        # a prose-only rationale Arm A structurally cannot reach, but naive
        # full-text search finds immediately.
        result = harness.answer_question(
            self.questions["fq03"], self.index, FIXTURE_REPO, FIXTURE_FILE_UNIVERSE
        )
        self.assertFalse(result["arm_a"]["covered"])
        self.assertTrue(result["arm_b"]["covered"])
        self.assertEqual(result["winner"], "B")

    def test_winner_is_neither_when_both_fail(self):
        result = harness.answer_question(self.questions["fq03"], self.index, FIXTURE_REPO, [])
        self.assertFalse(result["arm_a"]["covered"])
        self.assertFalse(result["arm_b"]["covered"])
        self.assertEqual(result["winner"], "neither")

    def test_winner_picks_fewer_bytes_when_both_cover(self):
        result = harness.answer_question(
            self.questions["fq01"], self.index, FIXTURE_REPO, FIXTURE_FILE_UNIVERSE
        )
        self.assertTrue(result["arm_a"]["covered"] and result["arm_b"]["covered"])
        a_bytes, b_bytes = result["arm_a"]["bytes_to_answer"], result["arm_b"]["bytes_to_answer"]
        expected = "A" if a_bytes < b_bytes else ("B" if b_bytes < a_bytes else "tie")
        self.assertEqual(result["winner"], expected)


class DeterminismTests(unittest.TestCase):
    def setUp(self):
        self.questions = list(harness.load_questions(FIXTURES_DIR / "questions.json"))
        self.index = _fixture_artifact_index()

    def _run_all(self):
        return [
            harness.answer_question(q, self.index, FIXTURE_REPO, FIXTURE_FILE_UNIVERSE)
            for q in self.questions
        ]

    def test_two_runs_produce_byte_identical_question_results(self):
        first = json.dumps(self._run_all(), sort_keys=True, ensure_ascii=False)
        second = json.dumps(self._run_all(), sort_keys=True, ensure_ascii=False)
        self.assertEqual(first, second)

    def test_aggregate_is_deterministic_given_the_same_results(self):
        results = self._run_all()
        first = json.dumps(harness.aggregate(results), sort_keys=True, ensure_ascii=False)
        second = json.dumps(harness.aggregate(results), sort_keys=True, ensure_ascii=False)
        self.assertEqual(first, second)

    def test_artifact_index_resolution_is_deterministic(self):
        first = _fixture_artifact_index()
        second = _fixture_artifact_index()
        self.assertEqual(
            {k: str(v) for k, v in first.items()}, {k: str(v) for k, v in second.items()}
        )


class RangeHelperTests(unittest.TestCase):
    def test_merge_ranges_combines_overlapping_and_touching(self):
        self.assertEqual(harness.merge_ranges([(5, 10), (8, 12), (13, 13), (20, 25)]), [(5, 13), (20, 25)])

    def test_merge_ranges_leaves_disjoint_ranges_separate(self):
        self.assertEqual(harness.merge_ranges([(1, 2), (10, 12)]), [(1, 2), (10, 12)])

    def test_span_fully_covered_requires_full_containment(self):
        self.assertTrue(harness.span_fully_covered(6, 9, [(5, 12)]))
        self.assertFalse(harness.span_fully_covered(6, 15, [(5, 12)]))
        self.assertFalse(harness.span_fully_covered(1, 3, [(5, 12)]))


class CorpusExclusionTests(unittest.TestCase):
    """Corpus exclusion (issue #93 self-reference pollution fix, #104): neither
    arm may ever read eval/, docs/archive/, or .out-of-scope/ as an answer
    source, even when a file there outscores the real golden file."""

    def setUp(self):
        self.questions = _fixture_questions()

    def test_is_excluded_from_corpus_matches_eval_archive_and_out_of_scope(self):
        self.assertTrue(harness.is_excluded_from_corpus("eval/questions.json"))
        self.assertTrue(harness.is_excluded_from_corpus("eval/BASELINE.md"))
        self.assertTrue(harness.is_excluded_from_corpus("docs/archive/plans/foo-idea.md"))
        self.assertTrue(harness.is_excluded_from_corpus(".out-of-scope/some-non-goal.md"))
        # arm_b's file_universe entries are posix-normalized, but the
        # predicate itself must not assume that -- a Windows-style path
        # must still match.
        self.assertTrue(harness.is_excluded_from_corpus("eval\\questions.json"))
        self.assertFalse(harness.is_excluded_from_corpus("crates/code-intel-cli/src/main.rs"))
        self.assertFalse(harness.is_excluded_from_corpus("docs/run-commit.md"))

    def test_search_kind_never_returns_a_pointer_into_excluded_corpus(self):
        # Identical keyword, one legitimate hit and one planted inside
        # eval/ -- only the legitimate one may survive.
        artifact_json = {
            "symbols": [
                {"id": "eval::pipeline_root_stub", "file": "eval/questions.json", "startLine": 5},
                {
                    "id": "capability_inventory::pipeline_root",
                    "file": "crates/code-intel-cli/src/capability_inventory.rs",
                    "startLine": 252,
                },
            ]
        }
        hits = harness.search_kind("code_evidence.symbols", artifact_json, ["pipeline_root"])
        self.assertEqual(hits, [("crates/code-intel-cli/src/capability_inventory.rs", 252)])

    def test_arm_b_never_touches_excluded_corpus_even_when_it_outscores_the_golden_file(self):
        # eval/fixtures/repo/eval/questions.json is a fixture file that
        # sits under an excluded dir (eval/, relative to the fixture repo
        # root) and deliberately repeats fq01's own keywords far more than
        # the real golden files do, so this proves exclusion, not a vacuous
        # "it never matched anyway" case.
        excluded_rel = "eval/questions.json"
        excluded_path = FIXTURE_REPO / excluded_rel
        self.assertTrue(excluded_path.is_file(), "fixture file must exist under an excluded dir")
        keywords = self.questions["fq01"]["golden"]["keywords"]
        excluded_text = excluded_path.read_text(encoding="utf-8")
        outscore = sum(excluded_text.lower().count(k.lower()) for k in keywords)
        self.assertGreaterEqual(outscore, 4, "fixture must genuinely outscore the real 2-hit files")

        result = harness.arm_b_answer(
            self.questions["fq01"], FIXTURE_REPO, FIXTURE_FILE_UNIVERSE + [excluded_rel]
        )
        self.assertTrue(all(not touched.startswith(excluded_rel) for touched in result["files_touched"]))
        # Byte-for-byte the same answer as without the excluded file present
        # (see ArmBNaiveSearchTests.test_matched_files_are_read_in_full_and_cover_the_golden_file):
        # planting a higher-scoring excluded file must change nothing.
        self.assertTrue(result["covered"])
        self.assertEqual(result["files_touched"], ["other.py (2 hits)", "sample.py (2 hits)"])


class QuestionSchemaTests(unittest.TestCase):
    """Basic golden.* schema shape. Anchor-specific shape and live-tree
    resolution of the real question set are eval/test_golden_anchors.py's
    job (RealQuestionsAnchorTests) -- split out for the same god-file-ratchet
    reason eval/golden_anchors.py's own resolver tests live there."""

    def test_real_questions_file_has_twelve_valid_entries(self):
        questions = harness.load_questions(EVAL_DIR / "questions.json")
        self.assertEqual(len(questions), 12)
        ids = [q["id"] for q in questions]
        self.assertEqual(len(set(ids)), 12, "question ids must be unique")
        for q in questions:
            self.assertIn(q["category"], {"how", "where", "why"})
            self.assertTrue(q["golden"]["keywords"])
            self.assertTrue(q["golden"]["spans"])

    def test_duplicate_question_id_is_rejected(self):
        fixture_questions = json.loads((FIXTURES_DIR / "questions.json").read_text(encoding="utf-8"))
        fixture_questions.append(dict(fixture_questions[0]))
        with tempfile.TemporaryDirectory() as tmp:
            bad_path = Path(tmp) / "questions_with_dup.json"
            bad_path.write_text(json.dumps(fixture_questions), encoding="utf-8")
            with self.assertRaises(ValueError):
                harness.load_questions(bad_path)

    def test_missing_keywords_is_rejected(self):
        fixture_questions = json.loads((FIXTURES_DIR / "questions.json").read_text(encoding="utf-8"))
        fixture_questions[0]["golden"]["keywords"] = []
        with tempfile.TemporaryDirectory() as tmp:
            bad_path = Path(tmp) / "questions_no_keywords.json"
            bad_path.write_text(json.dumps(fixture_questions), encoding="utf-8")
            with self.assertRaises(ValueError):
                harness.load_questions(bad_path)


if __name__ == "__main__":
    unittest.main(verbosity=2)
