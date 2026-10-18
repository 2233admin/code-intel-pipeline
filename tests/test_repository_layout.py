from __future__ import annotations

import json
import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

# A line-start git conflict marker: `<<<<<<< [label]`, a bare `=======`
# divider, `>>>>>>> [label]`, or the diff3 `||||||| [label]` ancestor marker.
# Requiring a following space/tab (or end-of-line for the bare `=======`
# divider) avoids false positives on unrelated runs of the same character.
CONFLICT_MARKER_PATTERN = re.compile(
    r"^(?:<{7}(?=[ \t]|$)|={7}$|>{7}(?=[ \t]|$)|\|{7}(?=[ \t]|$))"
)


class RepositoryLayoutTests(unittest.TestCase):
    def test_no_powershell_files_are_tracked(self) -> None:
        # DR-0018: the shipped product contains zero PowerShell.
        result = subprocess.run(
            ["git", "ls-files", "-z", "--", "*.ps1", "*.psm1"],
            cwd=ROOT,
            capture_output=True,
            check=True,
        )
        tracked = sorted(p.decode("utf-8") for p in result.stdout.split(b"\0") if p)
        self.assertEqual(tracked, [], "PowerShell files are still tracked")

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

    def test_shared_mcp_config_has_no_machine_specific_paths(self) -> None:
        # Hosts (Claude Code, OMP, Codex) import the root `.mcp.json` as-is,
        # so a drive-letter or home-directory argument breaks every server on
        # any machine but the author's (#446). Server arguments must be
        # repository-relative or come from the host's working directory.
        config = json.loads((ROOT / ".mcp.json").read_text(encoding="utf-8"))
        absolute = re.compile(r"^(?:[A-Za-z]:[\\/]|/|~)")
        offenders = [
            f"{name}: {arg}"
            for name, server in config["mcpServers"].items()
            for arg in [server.get("command", ""), *server.get("args", [])]
            if isinstance(arg, str) and absolute.match(arg)
        ]
        self.assertEqual(offenders, [], "machine-specific paths in .mcp.json")

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
