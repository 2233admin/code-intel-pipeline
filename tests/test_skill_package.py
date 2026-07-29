from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import stat
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
            "archive/install-code-intel-pipeline.ps1": installer,
            "archive/check-code-intel-tools.ps1": "Write-Output 'doctor'\n",
            "archive/code-intel.ps1": "Write-Output 'launch'\n",
            "archive/invoke-code-intel.ps1": "Write-Output 'invoke'\n",
        }.items():
            handle.writestr(f"code-intel-pipeline/{name}", content)


class SkillPackageTests(unittest.TestCase):
    def test_defaults_to_latest_published_stable_release(self) -> None:
        bootstrap = load_bootstrap_module()
        self.assertIsNone(bootstrap.resolve_version(None, "stable"))
        self.assertEqual(bootstrap.resolve_version("0.2.0", "stable"), "v0.2.0")
        self.assertIsNone(bootstrap.resolve_version(None, "prerelease"))

    def test_uses_canonical_skill_layout(self) -> None:
        self.assertTrue((SKILL_DIR / "SKILL.md").is_file())
        self.assertTrue((SKILL_DIR / "agents" / "openai.yaml").is_file())
        self.assertTrue(BOOTSTRAP_PATH.is_file())
        self.assertFalse((ROOT / "skill").exists())

    def test_installer_uses_canonical_skill_path(self) -> None:
        installer = (ROOT / "archive/install-code-intel-pipeline.ps1").read_text(
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

    def test_maps_sys_platform_to_release_platform_names(self) -> None:
        bootstrap = load_bootstrap_module()
        self.assertEqual(bootstrap.resolve_platform_name("win32"), "windows")
        self.assertEqual(bootstrap.resolve_platform_name("darwin"), "macos")
        self.assertEqual(bootstrap.resolve_platform_name("linux"), "linux")
        with self.assertRaises(bootstrap.BootstrapError):
            bootstrap.resolve_platform_name("freebsd14")

    def test_release_binary_name_is_platform_specific(self) -> None:
        bootstrap = load_bootstrap_module()
        self.assertEqual(bootstrap.release_binary_name("windows"), "code-intel.exe")
        self.assertEqual(bootstrap.release_binary_name("macos"), "code-intel")
        self.assertEqual(bootstrap.release_binary_name("linux"), "code-intel")

    def test_selects_release_asset_for_each_platform(self) -> None:
        bootstrap = load_bootstrap_module()
        release = {
            "tag_name": "v1.2.3",
            "assets": [
                {
                    "name": f"code-intel-pipeline-v1.2.3-{platform_name}.zip",
                    "browser_download_url": (
                        "https://github.com/2233admin/code-intel-pipeline/"
                        f"releases/download/v1.2.3/{platform_name}.zip"
                    ),
                    "digest": "sha256:" + ("a" * 64),
                }
                for platform_name in ("windows", "macos", "linux")
            ],
        }
        for platform_name in ("windows", "macos", "linux"):
            with self.subTest(platform=platform_name):
                selected = bootstrap.select_release_asset(release, platform_name)
                self.assertEqual(
                    selected["name"],
                    f"code-intel-pipeline-v1.2.3-{platform_name}.zip",
                )

    def test_member_unix_mode_honors_external_attributes(self) -> None:
        bootstrap = load_bootstrap_module()
        posix_member = zipfile.ZipInfo("code-intel-pipeline/bin/code-intel")
        posix_member.external_attr = (stat.S_IFREG | 0o755) << 16
        self.assertEqual(bootstrap.member_unix_mode(posix_member), 0o755)

        windows_member = zipfile.ZipInfo("code-intel-pipeline/bin/code-intel.exe")
        self.assertIsNone(bootstrap.member_unix_mode(windows_member))

        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "release.zip"
            with zipfile.ZipFile(archive, "w") as handle:
                handle.writestr(posix_member, "binary")
            with zipfile.ZipFile(archive) as handle:
                reloaded = handle.infolist()[0]
                self.assertEqual(bootstrap.member_unix_mode(reloaded), 0o755)

    def test_restore_member_mode_applies_posix_modes(self) -> None:
        bootstrap = load_bootstrap_module()
        member = zipfile.ZipInfo("code-intel-pipeline/bin/code-intel")
        member.external_attr = (stat.S_IFREG | 0o755) << 16
        target = mock.Mock()
        with mock.patch.object(bootstrap.os, "name", "posix"):
            bootstrap.restore_member_mode(member, target)
        target.chmod.assert_called_once_with(0o755)

        modeless_member = zipfile.ZipInfo("code-intel-pipeline/readme.txt")
        untouched = mock.Mock()
        with mock.patch.object(bootstrap.os, "name", "posix"):
            bootstrap.restore_member_mode(modeless_member, untouched)
        untouched.chmod.assert_not_called()

    def test_ensure_release_binary_executable_fails_closed(self) -> None:
        bootstrap = load_bootstrap_module()
        with tempfile.TemporaryDirectory() as temp:
            release_root = Path(temp) / "v1.2.3"
            (release_root / "bin").mkdir(parents=True)

            bootstrap.ensure_release_binary_executable(release_root, "windows")

            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.ensure_release_binary_executable(release_root, "linux")

            binary = release_root / "bin" / "code-intel"
            binary.write_bytes(b"binary")
            with mock.patch.object(
                bootstrap.os, "access", return_value=False
            ), mock.patch.object(
                bootstrap.Path, "chmod", autospec=True
            ) as chmod:
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.ensure_release_binary_executable(
                        release_root, "linux"
                    )
            chmod.assert_called_once_with(binary, 0o755)

            with mock.patch.object(bootstrap.os, "access", return_value=True):
                bootstrap.ensure_release_binary_executable(release_root, "linux")

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

            # arbitrary member name -- this exercises safe_extract_zip, not the
            # release layout, so it must match what the fixture zip wrote above
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

                (destination / "archive/install-code-intel-pipeline.ps1").write_text(
                    "tampered\n", encoding="utf-8"
                )
                repaired_destination, repaired_status = bootstrap.install_release(
                    asset, temp_path / "installs"
                )
                self.assertEqual(repaired_destination, destination)
                self.assertEqual(repaired_status, "repaired")
                self.assertNotEqual(
                    (destination / "archive/install-code-intel-pipeline.ps1").read_text(
                        encoding="utf-8"
                    ),
                    "tampered\n",
                )

                marker_data = json.loads(marker.read_text(encoding="utf-8"))
                marker_data["manifest_sha256"] = "0" * 64
                marker.write_text(json.dumps(marker_data), encoding="utf-8")
                _, marker_status = bootstrap.install_release(
                    asset, temp_path / "installs"
                )
                self.assertEqual(marker_status, "repaired")
                self.assertNotEqual(
                    json.loads(marker.read_text(encoding="utf-8"))[
                        "manifest_sha256"
                    ],
                    "0" * 64,
                )

                (destination / "archive/code-intel.ps1").unlink()
                _, missing_file_status = bootstrap.install_release(
                    asset, temp_path / "installs"
                )
                self.assertEqual(missing_file_status, "repaired")
                self.assertTrue((destination / "archive/code-intel.ps1").is_file())


if __name__ == "__main__":
    unittest.main()
