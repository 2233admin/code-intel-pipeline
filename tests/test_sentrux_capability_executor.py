from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EXECUTOR = ROOT / "crates" / "code-intel-cli" / "src" / "sentrux_capability_artifacts.rs"


class SentruxCapabilityExecutorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source = EXECUTOR.read_text(encoding="utf-8")

    def test_dispatch_table_names_every_remaining_canonical_capability(self) -> None:
        for capability in (
            "sentrux.provider_discovery",
            "sentrux.rescan",
            "sentrux.git_stats",
            "sentrux.evolution",
            "sentrux.test_gaps",
            "sentrux.what_if",
            "sentrux.session_start",
            "sentrux.session_end",
            "sentrux.baseline_save",
        ):
            with self.subTest(capability=capability):
                self.assertIn(f'"{capability}"', self.source)

    def test_dag_unsupported_routes_are_explicitly_non_applicable(self) -> None:
        self.assertIn("RouteKind::NotApplicable", self.source)
        for failure_kind in (
            "explicit_mutation_required",
            "session_lifecycle_outside_dag",
        ):
            with self.subTest(failure_kind=failure_kind):
                self.assertIn(f'failure_kind: "{failure_kind}"', self.source)
        self.assertIn('"authority":"declared_only"', self.source)

    def test_unimplemented_builtin_routes_are_not_reported_as_success(self) -> None:
        self.assertIn('"status":"unavailable"', self.source)
        self.assertIn('"authority":"compatibility"', self.source)
        self.assertIn("capability_unavailable", self.source)


if __name__ == "__main__":
    unittest.main()
