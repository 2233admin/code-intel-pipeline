from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
POWERSHELL_TESTS = ROOT / "scripts" / "tests"
PUBLIC_ROOT_ENTRY_POINTS = {
    "bootstrap-new-machine.ps1",
    "check-code-intel-tools.ps1",
    "Find-CodeIntelProjects.ps1",
    "install-code-intel-pipeline.ps1",
    "invoke-code-intel.ps1",
    "Invoke-SentruxAgentTool.ps1",
    "run-code-intel.ps1",
}


class RepositoryLayoutTests(unittest.TestCase):
    def test_public_root_entry_points_remain_stable(self) -> None:
        for name in PUBLIC_ROOT_ENTRY_POINTS:
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
                checkout_count = text.count("uses: actions/checkout@v4")
                self.assertGreater(checkout_count, 0)
                self.assertEqual(text.count("fetch-depth: 0"), checkout_count)


if __name__ == "__main__":
    unittest.main()
