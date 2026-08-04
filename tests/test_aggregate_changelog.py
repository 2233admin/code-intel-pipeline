"""Self-check for tools/aggregate_changelog.py (no network, temp dirs only)."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "aggregate_changelog.py"


def load_module():
    spec = importlib.util.spec_from_file_location("aggregate_changelog", SCRIPT)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    # dataclasses reads sys.modules[cls.__module__] during class body setup.
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


AGG = load_module()


SAMPLE_CHANGELOG = """# Changelog

All notable changes.

## [Unreleased]

### Fixed

- **stock fix** (#1): already in Unreleased.

## [0.1.0] — 2026-01-01

### Added

- first release.
"""


class ParseFragmentTests(unittest.TestCase):
    def test_type_line_and_bullet(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "174-feat.md"
            path.write_text(
                "type: feat\n\n- **title** (#174): body.\n",
                encoding="utf-8",
            )
            frag = AGG.parse_fragment(path)
            self.assertEqual(frag.section, "Added")
            self.assertIn("**title**", frag.body)

    def test_yaml_frontmatter(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "99.md"
            path.write_text(
                "---\ntype: fix\n---\n\n- **bug** (#99): done.\n",
                encoding="utf-8",
            )
            frag = AGG.parse_fragment(path)
            self.assertEqual(frag.section, "Fixed")

    def test_filename_type_suffix(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "12.docs.md"
            path.write_text("- docs only note.\n", encoding="utf-8")
            frag = AGG.parse_fragment(path)
            self.assertEqual(frag.section, "Changed")

    def test_bare_line_promoted_to_bullet(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.md"
            path.write_text("type: notes\n\nbare paragraph\n", encoding="utf-8")
            frag = AGG.parse_fragment(path)
            self.assertEqual(frag.body, "- bare paragraph")

    def test_unknown_type_raises(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.md"
            path.write_text("type: spaceship\n\n- no\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                AGG.parse_fragment(path)

    def test_skips_readme(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "README.md").write_text("# hi\n", encoding="utf-8")
            (d / "1.md").write_text("type: feat\n\n- a\n", encoding="utf-8")
            frags = AGG.load_fragments(d)
            self.assertEqual(len(frags), 1)
            self.assertEqual(frags[0].path.name, "1.md")


class MergeTests(unittest.TestCase):
    def test_merge_into_unreleased_existing_section(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            frag_dir = d / "changelog.d"
            frag_dir.mkdir()
            (frag_dir / "2.md").write_text(
                "type: fix\n\n- **new fix** (#2): from fragment.\n",
                encoding="utf-8",
            )
            cl = d / "CHANGELOG.md"
            cl.write_text(SAMPLE_CHANGELOG, encoding="utf-8")
            frags = AGG.load_fragments(frag_dir)
            grouped = AGG.group_by_section(frags)
            updated = AGG.apply_fragments_to_changelog(
                cl.read_text(encoding="utf-8"),
                grouped,
                version=None,
                release_date=None,
            )
            self.assertIn("**stock fix**", updated)
            self.assertIn("**new fix**", updated)
            # Still under Unreleased, before 0.1.0
            unreleased_pos = updated.index("## [Unreleased]")
            new_fix_pos = updated.index("**new fix**")
            v010 = updated.index("## [0.1.0]")
            self.assertLess(unreleased_pos, new_fix_pos)
            self.assertLess(new_fix_pos, v010)

    def test_new_version_section_inserted_after_unreleased(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            frag_dir = d / "changelog.d"
            frag_dir.mkdir()
            (frag_dir / "3.md").write_text(
                "type: feat\n\n- **ship it** (#3): feature.\n",
                encoding="utf-8",
            )
            cl = d / "CHANGELOG.md"
            cl.write_text(SAMPLE_CHANGELOG, encoding="utf-8")
            frags = AGG.load_fragments(frag_dir)
            grouped = AGG.group_by_section(frags)
            updated = AGG.apply_fragments_to_changelog(
                cl.read_text(encoding="utf-8"),
                grouped,
                version="0.2.0",
                release_date="2026-08-04",
            )
            self.assertIn("## [0.2.0] — 2026-08-04", updated)
            self.assertIn("**ship it**", updated)
            # Order: Unreleased, then 0.2.0, then 0.1.0
            i_u = updated.index("## [Unreleased]")
            i_new = updated.index("## [0.2.0]")
            i_old = updated.index("## [0.1.0]")
            self.assertLess(i_u, i_new)
            self.assertLess(i_new, i_old)
            # Stock Unreleased entry preserved and not moved into 0.2.0
            stock_pos = updated.index("**stock fix**")
            self.assertLess(stock_pos, i_new)

    def test_creates_missing_subsection(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            frag_dir = d / "changelog.d"
            frag_dir.mkdir()
            (frag_dir / "4.md").write_text(
                "type: security\n\n- **patched** (#4).\n",
                encoding="utf-8",
            )
            cl = d / "CHANGELOG.md"
            cl.write_text(SAMPLE_CHANGELOG, encoding="utf-8")
            frags = AGG.load_fragments(frag_dir)
            grouped = AGG.group_by_section(frags)
            updated = AGG.apply_fragments_to_changelog(
                cl.read_text(encoding="utf-8"),
                grouped,
                version=None,
                release_date=None,
            )
            self.assertIn("### Security", updated)
            self.assertIn("**patched**", updated)


class CliDryRunTests(unittest.TestCase):
    def test_dry_run_does_not_modify(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            frag_dir = d / "changelog.d"
            frag_dir.mkdir()
            frag = frag_dir / "5.md"
            frag.write_text(
                "type: feat\n\n- **dry** (#5): keep me.\n",
                encoding="utf-8",
            )
            cl = d / "CHANGELOG.md"
            original = SAMPLE_CHANGELOG
            cl.write_text(original, encoding="utf-8")

            proc = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--repo",
                    str(d),
                    "--version",
                    "9.9.9",
                    "--date",
                    "2026-08-04",
                    "--dry-run",
                ],
                capture_output=True,
                text=True,
                encoding="utf-8",
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("## [9.9.9]", proc.stdout)
            self.assertIn("**dry**", proc.stdout)
            self.assertTrue(frag.is_file(), "dry-run must not delete fragments")
            self.assertEqual(cl.read_text(encoding="utf-8"), original)

    def test_apply_writes_and_deletes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            frag_dir = d / "changelog.d"
            frag_dir.mkdir()
            frag = frag_dir / "6.md"
            frag.write_text(
                "type: fix\n\n- **applied** (#6): gone from fragments.\n",
                encoding="utf-8",
            )
            cl = d / "CHANGELOG.md"
            cl.write_text(SAMPLE_CHANGELOG, encoding="utf-8")

            proc = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--repo",
                    str(d),
                    "--version",
                    "0.3.0",
                    "--date",
                    "2026-08-04",
                ],
                capture_output=True,
                text=True,
                encoding="utf-8",
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            self.assertFalse(frag.exists())
            text = cl.read_text(encoding="utf-8")
            self.assertIn("## [0.3.0] — 2026-08-04", text)
            self.assertIn("**applied**", text)


class CheckPrTests(unittest.TestCase):
    def test_check_pr_advisory_exits_zero(self) -> None:
        # Against this real repo: our own branch should either have a fragment
        # (pass quietly) or emit advisory — either way exit 0.
        proc = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--check-pr",
                "--repo",
                str(ROOT),
                "--base-ref",
                "origin/main",
                "--head-ref",
                "HEAD",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)


if __name__ == "__main__":
    unittest.main()
