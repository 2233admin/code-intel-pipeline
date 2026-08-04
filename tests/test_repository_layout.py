from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
POWERSHELL_TESTS = ROOT / "legacy" / "scripts" / "tests"

# A line-start git conflict marker: `<<<<<<< [label]`, a bare `=======`
# divider, `>>>>>>> [label]`, or the diff3 `||||||| [label]` ancestor marker.
# Requiring a following space/tab (or end-of-line for the bare `=======`
# divider) avoids false positives on unrelated runs of the same character.
CONFLICT_MARKER_PATTERN = re.compile(
    r"^(?:<{7}(?=[ \t]|$)|={7}$|>{7}(?=[ \t]|$)|\|{7}(?=[ \t]|$))"
)
PUBLIC_ENTRY_POINTS = {
    "legacy/bootstrap-new-machine.ps1",
    "legacy/check-code-intel-tools.ps1",
    "legacy/Find-CodeIntelProjects.ps1",
    "legacy/install-code-intel-pipeline.ps1",
    "legacy/code-intel.ps1",
    "legacy/invoke-code-intel.ps1",
    "legacy/Invoke-SentruxAgentTool.ps1",
    "legacy/run-code-intel.ps1",
}


class RepositoryLayoutTests(unittest.TestCase):
    def test_public_entry_points_remain_stable(self) -> None:
        for name in PUBLIC_ENTRY_POINTS:
            with self.subTest(name=name):
                self.assertTrue((ROOT / name).is_file())

    def test_powershell_contract_tests_are_not_stored_at_root(self) -> None:
        root_tests = sorted(
            path.name
            for path in ROOT.glob("*.ps1")
            if path.name.lower().startswith("test-")
        )
        self.assertEqual(root_tests, [])
        self.assertTrue(POWERSHELL_TESTS.is_dir())
        self.assertGreater(len(list(POWERSHELL_TESTS.glob("*.ps1"))), 20)

    def test_completed_planning_records_are_archived(self) -> None:
        archive = ROOT / "docs" / "archive" / "2026-07"
        self.assertFalse((ROOT / "PLAN.md").exists())
        self.assertFalse((ROOT / "PLAN-REVIEW-LOG.md").exists())
        self.assertTrue((archive / "sentrux-failure-normalization-plan.md").is_file())
        self.assertTrue(
            (archive / "sentrux-failure-normalization-review-log.md").is_file()
        )

    def test_pipeline_workflows_checkout_complete_git_history(self) -> None:
        for name in ("ci.yml", "release.yml"):
            with self.subTest(name=name):
                text = (ROOT / ".github" / "workflows" / name).read_text(
                    encoding="utf-8"
                )
                checkout_count = text.count("uses: actions/checkout@")
                self.assertGreater(checkout_count, 0)
                self.assertEqual(text.count("fetch-depth: 0"), checkout_count)

    def test_no_committed_merge_conflict_markers(self) -> None:
        # A merge/rebase conflict resolved by committing the raw markers
        # instead of resolving them ships both sides of the conflict at
        # once and silently corrupts whatever text surrounds it. This has
        # happened before (CHANGELOG.md, see the fix that added this test).
        # `git ls-files` scopes the sweep to tracked files exactly like a
        # human `grep` triage would — no need to walk the working tree by
        # hand or worry about build output / .gitignore'd paths.
        result = subprocess.run(
            ["git", "ls-files", "-z"],
            cwd=ROOT,
            capture_output=True,
            check=True,
        )
        tracked = [p for p in result.stdout.split(b"\0") if p]

        offenders: list[str] = []
        for raw_path in tracked:
            rel_path = raw_path.decode("utf-8", errors="surrogateescape")
            path = ROOT / rel_path
            try:
                data = path.read_bytes()
            except OSError:
                continue
            # Cheap binary sniff so image/zip/exe fixtures aren't decoded.
            if b"\0" in data[:8000]:
                continue
            text = data.decode("utf-8", errors="ignore")
            for lineno, line in enumerate(text.splitlines(), start=1):
                if CONFLICT_MARKER_PATTERN.match(line):
                    offenders.append(f"{rel_path}:{lineno}: {line.strip()}")

        self.assertEqual(
            offenders,
            [],
            "committed git conflict markers found — an unresolved merge/"
            "rebase was committed as-is, shipping both sides at once:\n"
            + "\n".join(offenders),
        )


if __name__ == "__main__":
    unittest.main()
