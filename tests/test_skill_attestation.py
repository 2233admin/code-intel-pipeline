from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SKILL_DIR = ROOT / "skills" / "code-intel-pipeline"
BOOTSTRAP_PATH = SKILL_DIR / "scripts" / "bootstrap.py"


def load_bootstrap_module():
    spec = importlib.util.spec_from_file_location(
        "code_intel_skill_bootstrap", BOOTSTRAP_PATH
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to load {BOOTSTRAP_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SkillAttestationTests(unittest.TestCase):
    def test_gh_cli_version_none_when_gh_missing(self) -> None:
        bootstrap = load_bootstrap_module()
        with mock.patch.object(bootstrap.shutil, "which", return_value=None):
            self.assertIsNone(bootstrap.gh_cli_version())

    def test_gh_cli_version_parses_stdout(self) -> None:
        bootstrap = load_bootstrap_module()
        completed = mock.Mock(returncode=0, stdout="gh version 2.49.0 (2024-05-01)\n")
        with mock.patch.object(
            bootstrap.shutil, "which", return_value="/usr/bin/gh"
        ), mock.patch.object(bootstrap.subprocess, "run", return_value=completed):
            self.assertEqual(bootstrap.gh_cli_version(), (2, 49))

    def test_gh_cli_version_none_on_timeout(self) -> None:
        bootstrap = load_bootstrap_module()
        with mock.patch.object(
            bootstrap.shutil, "which", return_value="/usr/bin/gh"
        ), mock.patch.object(
            bootstrap.subprocess,
            "run",
            side_effect=bootstrap.subprocess.TimeoutExpired(cmd="gh", timeout=30),
        ):
            self.assertIsNone(bootstrap.gh_cli_version())

    def test_verify_build_provenance_raises_on_timeout(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "release.zip"
            archive.write_bytes(b"unused")
            with mock.patch.object(
                bootstrap, "gh_cli_version", return_value=(2, 49)
            ), mock.patch.object(
                bootstrap.subprocess,
                "run",
                side_effect=bootstrap.subprocess.TimeoutExpired(cmd="gh", timeout=30),
            ):
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.verify_build_provenance(archive)

    def test_verify_build_provenance_degrades_when_gh_missing(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "release.zip"
            archive.write_bytes(b"unused")
            with mock.patch.object(bootstrap, "gh_cli_version", return_value=None):
                self.assertEqual(
                    bootstrap.verify_build_provenance(archive), "degraded_missing_gh"
                )

    def test_verify_build_provenance_degrades_when_gh_too_old(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "release.zip"
            archive.write_bytes(b"unused")
            with mock.patch.object(bootstrap, "gh_cli_version", return_value=(2, 40)):
                self.assertEqual(
                    bootstrap.verify_build_provenance(archive), "degraded_missing_gh"
                )

    def test_verify_build_provenance_accepts_passing_attestation(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "release.zip"
            archive.write_bytes(b"unused")
            completed = mock.Mock(returncode=0, stdout="", stderr="")
            with mock.patch.object(
                bootstrap, "gh_cli_version", return_value=(2, 49)
            ), mock.patch.object(bootstrap.subprocess, "run", return_value=completed):
                self.assertEqual(bootstrap.verify_build_provenance(archive), "verified")

    def test_verify_build_provenance_rejects_failing_attestation(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "release.zip"
            archive.write_bytes(b"unused")
            completed = mock.Mock(
                returncode=1, stdout="", stderr="no matching attestations"
            )
            with mock.patch.object(
                bootstrap, "gh_cli_version", return_value=(2, 49)
            ), mock.patch.object(bootstrap.subprocess, "run", return_value=completed):
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.verify_build_provenance(archive)


if __name__ == "__main__":
    unittest.main()
