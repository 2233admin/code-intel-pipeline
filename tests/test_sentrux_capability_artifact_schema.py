from __future__ import annotations

import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = (
    ROOT
    / "orchestration"
    / "schemas"
    / "code-intel-sentrux-capability-artifact.v1.schema.json"
)


class SentruxCapabilityArtifactSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))

    def test_envelope_is_closed_and_requires_shared_fields(self) -> None:
        self.assertEqual(self.schema["type"], "object")
        self.assertFalse(self.schema["additionalProperties"])
        self.assertEqual(
            set(self.schema["required"]),
            {
                "schema",
                "contractVersion",
                "capabilityId",
                "operation",
                "runId",
                "snapshotIdentity",
                "provider",
                "status",
                "authority",
                "inputs",
                "outputs",
                "failure",
                "freshness",
                "decisionConsumers",
            },
        )

    def test_status_and_authority_are_explicit_enums_without_silent_skip(self) -> None:
        status = self.schema["properties"]["status"]["enum"]
        authority = self.schema["properties"]["authority"]["enum"]

        self.assertIn("skipped", status)
        self.assertNotIn("silent_skip", authority)
        self.assertNotIn("none", authority)
        self.assertEqual(
            set(status),
            {"succeeded", "degraded", "unavailable", "skipped", "not_applicable", "failed"},
        )
        self.assertEqual(
            set(authority),
            {"authoritative", "fallback", "compatibility", "declared_only"},
        )

    def test_provider_and_freshness_bind_evidence_to_a_snapshot(self) -> None:
        provider = self.schema["$defs"]["provider"]
        freshness = self.schema["$defs"]["freshness"]

        self.assertEqual(
            set(provider["required"]), {"mode", "id", "version", "digest"}
        )
        self.assertEqual(
            self.schema["$id"], "code-intel-sentrux-capability-artifact.v1"
        )
        self.assertEqual(
            self.schema["properties"]["schema"]["const"],
            "code-intel-sentrux-capability-artifact.v1",
        )
        self.assertEqual(
            set(freshness["required"]),
            {"status", "evaluatedAt", "consumedSnapshotIdentity"},
        )
        self.assertEqual(
            set(provider["properties"]["mode"]["enum"]),
            {"external", "builtin", "lite_fallback", "legacy_compatibility"},
        )

    def test_non_success_requires_failure_and_special_states_name_the_failure(self) -> None:
        branches = self.schema["allOf"]
        outcome_branch = branches[0]
        unavailable_branch = branches[1]
        skipped_branch = branches[2]
        not_applicable_branch = branches[3]

        self.assertEqual(
            outcome_branch["if"]["properties"]["status"]["const"], "succeeded"
        )
        self.assertEqual(outcome_branch["then"]["properties"]["failure"]["type"], "null")
        self.assertEqual(
            outcome_branch["else"]["properties"]["failure"]["$ref"],
            "#/$defs/failure",
        )
        self.assertEqual(
            unavailable_branch["then"]["properties"]["failure"]["properties"]["kind"]["const"],
            "provider_unavailable",
        )
        self.assertEqual(
            skipped_branch["then"]["properties"]["failure"]["properties"]["kind"]["const"],
            "skipped",
        )
        self.assertEqual(
            not_applicable_branch["then"]["properties"]["failure"]["properties"]["kind"]["const"],
            "not_applicable",
        )

    def test_artifact_type_is_fixed_inside_outputs(self) -> None:
        artifact_ref = self.schema["$defs"]["artifactRef"]

        self.assertEqual(
            artifact_ref["properties"]["type"]["const"],
            "provider.sentrux.capability-artifact",
        )
        self.assertIn("artifacts", self.schema["properties"]["outputs"]["properties"])

    def test_decision_consumers_cannot_be_empty(self) -> None:
        consumers = self.schema["properties"]["decisionConsumers"]

        self.assertEqual(consumers["type"], "array")
        self.assertEqual(consumers["minItems"], 1)
        self.assertTrue(consumers["uniqueItems"])


if __name__ == "__main__":
    unittest.main()
