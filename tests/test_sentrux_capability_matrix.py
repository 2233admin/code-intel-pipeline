from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MATRIX_PATH = ROOT / "orchestration" / "sentrux-capability-matrix.v1.json"
INTEGRATIONS_PATH = ROOT / "orchestration" / "integrations.json"
RUST_SENTRUX_PATH = ROOT / "crates" / "code-intel-cli" / "src" / "sentrux.rs"
EXECUTOR_PATH = ROOT / "crates" / "code-intel-cli" / "src" / "sentrux_capability_artifacts.rs"
LEGACY_SENTRY_PATH = ROOT / "legacy" / "Invoke-SentruxAgentTool.ps1"


class SentruxCapabilityMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.matrix = json.loads(MATRIX_PATH.read_text(encoding="utf-8"))
        cls.integrations = json.loads(INTEGRATIONS_PATH.read_text(encoding="utf-8"))
        cls.rust_source = RUST_SENTRUX_PATH.read_text(encoding="utf-8")
        cls.executor_source = EXECUTOR_PATH.read_text(encoding="utf-8")
        cls.legacy_source = LEGACY_SENTRY_PATH.read_text(encoding="utf-8")

    def test_matrix_has_a_single_identity_for_every_capability_and_alias(self) -> None:
        capabilities = self.matrix["capabilities"]
        ids = [item["id"] for item in capabilities]
        self.assertEqual(len(ids), len(set(ids)))

        aliases: list[str] = []
        for item in capabilities:
            aliases.extend(item["aliases"])
            self.assertNotIn(item["operation"], item["aliases"])
        self.assertEqual(len(aliases), len(set(aliases)))
        self.assertTrue(set(ids).isdisjoint(aliases))

    def test_all_declared_structure_sentrux_capabilities_are_mapped(self) -> None:
        structure = next(
            item
            for item in self.integrations["integrations"]
            if item["id"] == "structure.sentrux"
        )
        operations = {
            capability
            for capability in structure["capabilities"]
            if capability != "rules"
        }
        mapped = {
            alias
            for item in self.matrix["capabilities"]
            for alias in [item["operation"], *item["aliases"]]
        }
        self.assertTrue(operations <= mapped)
        self.assertIn("rules", mapped)

    def test_rust_operations_are_present_in_the_matrix(self) -> None:
        expected = {"dsm", "scan", "health", "check", "check_rules", "gate", "gate_save"}
        matrix_operations = {item["operation"] for item in self.matrix["capabilities"]}
        self.assertTrue(expected <= matrix_operations)
        for operation in expected:
            with self.subTest(operation=operation):
                self.assertIn(f'"{operation}"', self.rust_source)

    def test_legacy_operations_and_aliases_are_present_in_the_matrix(self) -> None:
        validate_set = re.search(r"ValidateSet\(([^)]*)\)", self.legacy_source)
        self.assertIsNotNone(validate_set)
        declared = set(re.findall(r'"([a-z_]+)"', validate_set.group(1)))
        mapped = {
            value
            for item in self.matrix["capabilities"]
            for value in [item["operation"], *item["aliases"]]
        }
        self.assertTrue(declared <= mapped)

    def test_partial_coverage_is_explicit_until_all_release_capabilities_are_automatic(self) -> None:
        self.assertEqual(self.matrix["coverageStatus"], "partial")
        required = [item for item in self.matrix["capabilities"] if item["requiredForRelease"]]
        self.assertTrue(any(item["currentState"] != "authoritative_automatic" for item in required))
        self.assertEqual(
            set(self.matrix["completionPolicy"]["forbiddenSilentStates"]),
            {"declared_only", "compatibility_only"},
        )

    def test_automatic_matrix_routes_have_executor_entries_and_consumers(self) -> None:
        for capability in self.matrix["capabilities"]:
            with self.subTest(capability=capability["id"]):
                if capability["route"].startswith("provider.sentrux-adapt"):
                    self.assertIn(f'"{capability["id"]}"', self.executor_source)
                    self.assertNotEqual(capability["currentState"], "compatibility_only")
                    self.assertNotEqual(capability["currentState"], "declared_only")
                self.assertTrue(capability["artifacts"])
                self.assertTrue(capability["decisionConsumers"])

    def test_explicit_and_lifecycle_capabilities_are_not_automatic_dag_routes(self) -> None:
        states = {item["id"]: item["currentState"] for item in self.matrix["capabilities"]}
        self.assertEqual(states["sentrux.what_if"], "automatic_degraded")
        self.assertEqual(states["sentrux.baseline_save"], "explicit_authority_required")
        self.assertEqual(states["sentrux.session_start"], "lifecycle_external")
        self.assertEqual(states["sentrux.session_end"], "lifecycle_external")


if __name__ == "__main__":
    unittest.main()
