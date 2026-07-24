from __future__ import annotations

import hashlib
import importlib.util
import shutil
import tempfile
import unittest
import zipfile
from email.message import Message
from io import BytesIO
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


def write_release_archive(path: Path, *, installer: str = "Write-Output 'ok'\n") -> None:
    with zipfile.ZipFile(path, "w") as handle:
        for name, content in {
            "install-code-intel-pipeline.ps1": installer,
            "check-code-intel-tools.ps1": "Write-Output 'doctor'\n",
            "invoke-code-intel.ps1": "Write-Output 'invoke'\n",
        }.items():
            handle.writestr(f"code-intel-pipeline/{name}", content)


class SkillPackageTests(unittest.TestCase):
    def test_uses_canonical_skill_layout(self) -> None:
        self.assertTrue((SKILL_DIR / "SKILL.md").is_file())
        self.assertTrue((SKILL_DIR / "agents" / "openai.yaml").is_file())
        self.assertTrue(BOOTSTRAP_PATH.is_file())
        self.assertFalse((ROOT / "skill").exists())

    def test_installer_uses_canonical_skill_path(self) -> None:
        installer = (ROOT / "install-code-intel-pipeline.ps1").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            'Join-Path (Join-Path $root "skills") "code-intel-pipeline"',
            installer,
        )

    def test_selects_release_asset_with_digest(self) -> None:
        bootstrap = load_bootstrap_module()
        release = {
            "tag_name": "v1.2.3",
            "assets": [
                {
                    "name": "code-intel-pipeline-v1.2.3-windows.zip",
                    "browser_download_url": (
                        "https://github.com/2233admin/code-intel-pipeline/"
                        "releases/download/v1.2.3/pipeline.zip"
                    ),
                    "digest": "sha256:" + ("a" * 64),
                }
            ],
        }

        selected = bootstrap.select_release_asset(release, "windows")

        self.assertEqual(selected["name"], release["assets"][0]["name"])
        self.assertEqual(selected["sha256"], "a" * 64)

    def test_rejects_release_asset_without_digest(self) -> None:
        bootstrap = load_bootstrap_module()
        release = {
            "tag_name": "v1.2.3",
            "assets": [
                {
                    "name": "code-intel-pipeline-v1.2.3-windows.zip",
                    "browser_download_url": "https://example.invalid/pipeline.zip",
                }
            ],
        }

        with self.assertRaises(bootstrap.BootstrapError):
            bootstrap.select_release_asset(release, "windows")

    def test_fetch_release_enforces_requested_channel_and_publication(self) -> None:
        bootstrap = load_bootstrap_module()
        cases = [
            ("stable", {"tag_name": "v1.2.3", "prerelease": True}, "mismatch"),
            (
                "prerelease",
                {"tag_name": "v1.2.3", "prerelease": False},
                "mismatch",
            ),
            (
                "stable",
                {"tag_name": "v1.2.3", "prerelease": False, "draft": True},
                "draft",
            ),
        ]
        for channel, release, label in cases:
            with self.subTest(label=label):
                with mock.patch.object(
                    bootstrap, "request_json", return_value=release
                ):
                    with self.assertRaises(bootstrap.BootstrapError):
                        bootstrap.fetch_release("v1.2.3", channel)

    def test_fetch_release_accepts_matching_explicit_channel(self) -> None:
        bootstrap = load_bootstrap_module()
        release = {"tag_name": "v1.2.3-beta.1", "prerelease": True, "draft": False}
        with mock.patch.object(bootstrap, "request_json", return_value=release):
            self.assertIs(
                bootstrap.fetch_release("v1.2.3-beta.1", "prerelease"), release
            )

    def test_safe_extract_rejects_parent_traversal(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            archive = temp_path / "unsafe.zip"
            destination = temp_path / "out"
            with zipfile.ZipFile(archive, "w") as handle:
                handle.writestr("../escape.txt", "no")

            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.safe_extract_zip(archive, destination)

            self.assertFalse((temp_path / "escape.txt").exists())

    def test_safe_extract_rejects_windows_aliases_and_conflicts(self) -> None:
        bootstrap = load_bootstrap_module()
        cases = {
            "duplicate": [("same.txt", "one"), ("same.txt", "two")],
            "case collision": [("Name.txt", "one"), ("name.TXT", "two")],
            "alternate data stream": [("root/file.txt:stream", "no")],
            "reserved device": [("root/CON.txt", "no")],
            "console input device": [("root/CONIN$.txt", "no")],
            "console output device": [("root/CONOUT$", "no")],
            "superscript com device": [("root/COM¹.log", "no")],
            "superscript lpt device": [("root/LPT³", "no")],
            "trailing dot": [("root/name.", "no")],
            "ancestor conflict": [("root", "file"), ("root/child.txt", "no")],
        }
        for label, members in cases.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temp:
                temp_path = Path(temp)
                archive = temp_path / "unsafe.zip"
                with zipfile.ZipFile(archive, "w") as handle:
                    for name, content in members:
                        handle.writestr(name, content)
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.safe_extract_zip(archive, temp_path / "out")

    def test_safe_extract_and_sha256(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            archive = temp_path / "safe.zip"
            destination = temp_path / "out"
            with zipfile.ZipFile(archive, "w") as handle:
                handle.writestr(
                    "code-intel-pipeline/install-code-intel-pipeline.ps1",
                    "Write-Output 'ok'\n",
                )

            bootstrap.safe_extract_zip(archive, destination)

            extracted = (
                destination / "code-intel-pipeline" / "install-code-intel-pipeline.ps1"
            )
            self.assertTrue(extracted.is_file())
            expected = hashlib.sha256(archive.read_bytes()).hexdigest()
            self.assertEqual(bootstrap.sha256_file(archive), expected)

    def test_download_rejects_archive_larger_than_limit(self) -> None:
        bootstrap = load_bootstrap_module()

        class OversizedResponse(BytesIO):
            def __init__(self) -> None:
                super().__init__(b"unused")
                self.headers = Message()
                self.headers["Content-Length"] = str(bootstrap.MAX_ARCHIVE_BYTES + 1)

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                self.close()

        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp) / "release.zip"
            with mock.patch.object(
                bootstrap.urllib.request,
                "urlopen",
                return_value=OversizedResponse(),
            ):
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.download_file(
                        "https://github.com/example/release.zip", destination
                    )
            self.assertFalse(destination.exists())

    def test_existing_release_is_reverified_against_github_asset(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            archive = temp_path / "release.zip"
            write_release_archive(archive)
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            asset = {
                "tag": "v1.2.3",
                "name": "code-intel-pipeline-v1.2.3-windows.zip",
                "url": (
                    "https://github.com/2233admin/code-intel-pipeline/"
                    "releases/download/v1.2.3/"
                    "code-intel-pipeline-v1.2.3-windows.zip"
                ),
                "sha256": digest,
            }

            def copy_archive(_url: str, destination: Path) -> None:
                shutil.copyfile(archive, destination)

            with mock.patch.object(
                bootstrap, "download_file", side_effect=copy_archive
            ):
                destination, status = bootstrap.install_release(
                    asset, temp_path / "installs"
                )
                self.assertEqual(status, "installed")
                marker = destination / bootstrap.RELEASE_MARKER
                self.assertTrue(marker.is_file())

                _, repeated_status = bootstrap.install_release(
                    asset, temp_path / "installs"
                )
                self.assertEqual(repeated_status, "already_installed")

                (destination / "install-code-intel-pipeline.ps1").write_text(
                    "tampered\n", encoding="utf-8"
                )
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.install_release(asset, temp_path / "installs")


if __name__ == "__main__":
    unittest.main()
