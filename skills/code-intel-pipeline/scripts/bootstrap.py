#!/usr/bin/env python3
"""Install a verified Code Intel Pipeline GitHub Release for this Skill."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any


REPOSITORY = "2233admin/code-intel-pipeline"
API_ROOT = f"https://api.github.com/repos/{REPOSITORY}"
USER_AGENT = "code-intel-pipeline-skill-bootstrap/1"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TAG_PATTERN = re.compile(r"^[0-9A-Za-z][0-9A-Za-z._-]*$")
WINDOWS_RESERVED_NAMES = {
    "con",
    "conin$",
    "conout$",
    "prn",
    "aux",
    "nul",
    *(f"com{index}" for index in range(1, 10)),
    *(f"lpt{index}" for index in range(1, 10)),
    *(f"com{index}" for index in ("¹", "²", "³")),
    *(f"lpt{index}" for index in ("¹", "²", "³")),
}
PLATFORM_NAMES = {
    "win32": "windows",
    "darwin": "macos",
    "linux": "linux",
}
MAX_ARCHIVE_MEMBERS = 10_000
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MAX_MEMBER_BYTES = 256 * 1024 * 1024
MAX_TOTAL_BYTES = 1024 * 1024 * 1024
MAX_COMPRESSION_RATIO = 200
RELEASE_MARKER = ".code-intel-release.json"


class BootstrapError(RuntimeError):
    """Raised when the release cannot be installed safely."""


def resolve_platform_name(platform: str) -> str:
    """Map a ``sys.platform`` value onto a release asset platform name."""
    platform_name = PLATFORM_NAMES.get(platform)
    if platform_name is None:
        supported = ", ".join(
            f"{key} ({value})" for key, value in sorted(PLATFORM_NAMES.items())
        )
        raise BootstrapError(
            f"Unsupported platform for the published Skill bootstrap: {platform!r}. "
            f"Supported platforms: {supported}."
        )
    return platform_name


def release_binary_name(platform_name: str) -> str:
    """Packaged CLI binary file name for a release platform."""
    return "code-intel.exe" if platform_name == "windows" else "code-intel"


def request_json(url: str) -> Any:
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": USER_AGENT,
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            try:
                return json.load(response)
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                raise BootstrapError(
                    f"GitHub returned invalid JSON for {url}."
                ) from error
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise BootstrapError(
            f"GitHub API request failed ({error.code}) for {url}: {detail[:500]}"
        ) from error
    except (urllib.error.URLError, TimeoutError) as error:
        raise BootstrapError(f"GitHub API request failed for {url}: {error}") from error


def normalize_tag(version: str) -> str:
    tag = version.strip()
    if not tag:
        raise BootstrapError("Release version cannot be empty.")
    if not tag.startswith("v") and re.match(r"^\d", tag):
        tag = f"v{tag}"
    if not TAG_PATTERN.fullmatch(tag):
        raise BootstrapError(f"Unsupported release tag: {version!r}")
    return tag


def resolve_version(version: str | None, channel: str) -> str | None:
    if version:
        return normalize_tag(version)
    return None


def fetch_release(version: str | None, channel: str) -> dict[str, Any]:
    if version:
        tag = normalize_tag(version)
        encoded_tag = urllib.parse.quote(tag, safe="")
        release = request_json(f"{API_ROOT}/releases/tags/{encoded_tag}")
    elif channel == "stable":
        release = request_json(f"{API_ROOT}/releases/latest")
    else:
        releases = request_json(f"{API_ROOT}/releases?per_page=30")
        if not isinstance(releases, list):
            raise BootstrapError("GitHub returned an invalid release list.")
        release = next(
            (
                item
                for item in releases
                if item.get("prerelease") and not item.get("draft")
            ),
            None,
        )
        if release is None:
            raise BootstrapError("No published prerelease was found.")
    if not isinstance(release, dict):
        raise BootstrapError("GitHub returned an invalid release payload.")
    if release.get("draft"):
        raise BootstrapError("Draft releases cannot be installed.")
    actual_channel = "prerelease" if release.get("prerelease") else "stable"
    if actual_channel != channel:
        raise BootstrapError(
            f"Release channel mismatch: requested {channel}, received {actual_channel}."
        )
    return release


def select_release_asset(release: dict[str, Any], platform_name: str) -> dict[str, str]:
    tag = normalize_tag(str(release.get("tag_name", "")))
    expected_name = f"code-intel-pipeline-{tag}-{platform_name}.zip"
    assets = release.get("assets")
    if not isinstance(assets, list):
        raise BootstrapError(f"Release {tag} has no asset list.")
    asset = next(
        (
            item
            for item in assets
            if isinstance(item, dict) and item.get("name") == expected_name
        ),
        None,
    )
    if asset is None:
        raise BootstrapError(
            f"Release {tag} does not contain the required asset {expected_name}."
        )
    url = str(asset.get("browser_download_url", ""))
    digest_value = str(asset.get("digest", ""))
    if not url.startswith("https://github.com/"):
        raise BootstrapError(f"Release asset URL is not a GitHub HTTPS URL: {url!r}")
    if not digest_value.startswith("sha256:"):
        raise BootstrapError(f"Release asset {expected_name} has no SHA-256 digest.")
    digest = digest_value.removeprefix("sha256:").lower()
    if not SHA256_PATTERN.fullmatch(digest):
        raise BootstrapError(f"Release asset {expected_name} has an invalid digest.")
    return {
        "tag": tag,
        "name": expected_name,
        "url": url,
        "sha256": digest,
    }


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download_file(url: str, destination: Path) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            declared_length = response.headers.get("Content-Length")
            if declared_length:
                try:
                    declared_bytes = int(declared_length)
                except ValueError as error:
                    raise BootstrapError(
                        f"Release download returned an invalid Content-Length: {declared_length!r}"
                    ) from error
                if declared_bytes < 0 or declared_bytes > MAX_ARCHIVE_BYTES:
                    raise BootstrapError(
                        f"Release download exceeds the {MAX_ARCHIVE_BYTES}-byte limit."
                    )
            downloaded = 0
            with destination.open("xb") as output:
                while chunk := response.read(1024 * 1024):
                    downloaded += len(chunk)
                    if downloaded > MAX_ARCHIVE_BYTES:
                        raise BootstrapError(
                            f"Release download exceeds the {MAX_ARCHIVE_BYTES}-byte limit."
                        )
                    output.write(chunk)
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
        raise BootstrapError(f"Release download failed for {url}: {error}") from error


def validated_archive_members(
    handle: zipfile.ZipFile,
) -> list[tuple[zipfile.ZipInfo, PurePosixPath]]:
    members = handle.infolist()
    if len(members) > MAX_ARCHIVE_MEMBERS:
        raise BootstrapError(
            f"Release archive contains too many members ({len(members)})."
        )

    total_bytes = 0
    seen: dict[str, bool] = {}
    validated: list[tuple[zipfile.ZipInfo, PurePosixPath]] = []
    for member in members:
        normalized_name = member.filename.replace("\\", "/")
        trimmed_name = normalized_name[:-1] if normalized_name.endswith("/") else normalized_name
        raw_parts = trimmed_name.split("/")
        relative = PurePosixPath(*raw_parts)
        if (
            not trimmed_name
            or relative.is_absolute()
            or any(part in ("", ".", "..") for part in raw_parts)
        ):
            raise BootstrapError(
                f"Release archive contains an unsafe path: {member.filename!r}"
            )
        for part in raw_parts:
            stem = part.split(".", 1)[0].casefold()
            if (
                ":" in part
                or part.endswith((" ", "."))
                or stem in WINDOWS_RESERVED_NAMES
                or any(ord(character) < 32 for character in part)
            ):
                raise BootstrapError(
                    f"Release archive contains a Windows-unsafe path: {member.filename!r}"
                )

        mode = member.external_attr >> 16
        if stat.S_ISLNK(mode):
            raise BootstrapError(
                f"Release archive contains an unsupported symlink: {member.filename!r}"
            )
        if member.file_size > MAX_MEMBER_BYTES:
            raise BootstrapError(
                f"Release archive member is too large: {member.filename!r}"
            )
        total_bytes += member.file_size
        if total_bytes > MAX_TOTAL_BYTES:
            raise BootstrapError("Release archive expands beyond the allowed size.")
        if member.file_size > 1024 * 1024:
            if member.compress_size == 0 or (
                member.file_size / member.compress_size > MAX_COMPRESSION_RATIO
            ):
                raise BootstrapError(
                    f"Release archive member has an unsafe compression ratio: {member.filename!r}"
                )

        canonical = "/".join(part.casefold() for part in raw_parts)
        is_directory = member.is_dir()
        if canonical in seen:
            raise BootstrapError(
                f"Release archive contains a duplicate Windows path: {member.filename!r}"
            )
        ancestors = [
            "/".join(part.casefold() for part in raw_parts[:index])
            for index in range(1, len(raw_parts))
        ]
        if any(ancestor in seen and not seen[ancestor] for ancestor in ancestors):
            raise BootstrapError(
                f"Release archive contains a file/directory conflict: {member.filename!r}"
            )
        if not is_directory and any(
            existing.startswith(canonical + "/") for existing in seen
        ):
            raise BootstrapError(
                f"Release archive contains a file/directory conflict: {member.filename!r}"
            )
        seen[canonical] = is_directory
        validated.append((member, relative))
    return validated


def member_unix_mode(member: zipfile.ZipInfo) -> int | None:
    """POSIX permission bits stored in a zip member, or None when absent.

    ``zipfile`` does not restore modes on extraction, so archives built on a
    POSIX host record them in the high 16 bits of ``external_attr`` and we
    re-apply them ourselves. Archives built on Windows leave those bits zero.
    """
    permissions = stat.S_IMODE(member.external_attr >> 16)
    if permissions == 0:
        return None
    return permissions


def restore_member_mode(member: zipfile.ZipInfo, target: Path) -> None:
    if os.name == "nt":
        return
    mode = member_unix_mode(member)
    if mode is not None:
        target.chmod(mode)


def safe_extract_zip(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    destination_root = destination.resolve()
    with zipfile.ZipFile(archive) as handle:
        members = validated_archive_members(handle)
        for member, relative in members:
            target = destination.joinpath(*relative.parts).resolve()
            try:
                target.relative_to(destination_root)
            except ValueError as error:
                raise BootstrapError(
                    f"Release archive escapes the destination: {member.filename!r}"
                ) from error
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            copied = 0
            with handle.open(member) as source, target.open("xb") as output:
                while chunk := source.read(1024 * 1024):
                    copied += len(chunk)
                    if copied > member.file_size or copied > MAX_MEMBER_BYTES:
                        raise BootstrapError(
                            f"Release archive member exceeded its declared size: {member.filename!r}"
                        )
                    output.write(chunk)
            if copied != member.file_size:
                raise BootstrapError(
                    f"Release archive member size mismatch: {member.filename!r}"
                )
            restore_member_mode(member, target)


ENTRY_POINTS = (
    "install-code-intel-pipeline.ps1",
    "check-code-intel-tools.ps1",
    "code-intel.ps1",
    "invoke-code-intel.ps1",
)

# Releases from 0.7.0-beta.2 onward carry the PowerShell entry points under
# legacy/. Earlier archives keep them at the payload root, and bootstrap has
# to stay able to install those, so both layouts are probed, newest first.
PAYLOAD_LAYOUTS = ("legacy/", "")


def find_payload_root(extracted_root: Path) -> Path:
    for prefix in PAYLOAD_LAYOUTS:
        installer = f"{prefix}{ENTRY_POINTS[0]}"
        depth = installer.count("/")
        candidates = [
            path.parents[depth]
            for path in extracted_root.glob(f"*/{installer}")
            if path.is_file()
        ]
        if (extracted_root / installer).is_file():
            candidates.append(extracted_root)
        unique_candidates = list(dict.fromkeys(candidates))
        if len(unique_candidates) > 1:
            raise BootstrapError(
                "Release archive must contain exactly one Code Intel Pipeline root."
            )
        if not unique_candidates:
            continue

        payload = unique_candidates[0]
        required = tuple(f"{prefix}{name}" for name in ENTRY_POINTS)
        missing = [name for name in required if not (payload / name).is_file()]
        if missing:
            raise BootstrapError(
                "Release archive is missing required files: " + ", ".join(missing)
            )
        return payload

    raise BootstrapError(
        "Release archive must contain exactly one Code Intel Pipeline root."
    )


def payload_manifest(payload: Path) -> dict[str, dict[str, Any]]:
    manifest: dict[str, dict[str, Any]] = {}
    for path in sorted(payload.rglob("*")):
        relative = path.relative_to(payload).as_posix()
        if relative == RELEASE_MARKER:
            continue
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise BootstrapError(f"Installed payload contains a symlink: {relative}")
        if path.is_dir():
            continue
        if not path.is_file():
            raise BootstrapError(f"Installed payload contains a non-file: {relative}")
        manifest[relative] = {
            "sha256": sha256_file(path),
            "size": metadata.st_size,
        }
    return manifest


def manifest_digest(manifest: dict[str, dict[str, Any]]) -> str:
    canonical = json.dumps(
        manifest, ensure_ascii=True, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def default_data_root() -> Path:
    override = os.environ.get("CODE_INTEL_DATA_ROOT")
    if override:
        return Path(override).expanduser()
    platform_name = PLATFORM_NAMES.get(sys.platform)
    if platform_name == "macos":
        return Path.home() / "Library" / "Application Support" / "code-intel"
    if platform_name == "linux":
        xdg_data_home = os.environ.get("XDG_DATA_HOME")
        base = Path(xdg_data_home).expanduser() if xdg_data_home else (
            Path.home() / ".local" / "share"
        )
        return base / "code-intel"
    local_app_data = os.environ.get("LOCALAPPDATA")
    if not local_app_data:
        local_app_data = str(Path.home() / "AppData" / "Local")
    return Path(local_app_data) / "code-intel"


def default_install_root() -> Path:
    return default_data_root() / "releases"


def find_pwsh() -> str:
    executable = shutil.which("pwsh")
    if executable:
        return executable
    raise BootstrapError(
        "PowerShell 7.2+ (pwsh) is required on every platform because the "
        "published installer is implemented in PowerShell."
    )


def ensure_release_binary_executable(release_root: Path, platform_name: str) -> None:
    """Fail closed unless the packaged CLI binary is runnable on this platform.

    Windows does not use POSIX execute bits, so the check only applies to the
    macos/linux payloads, which always ship ``bin/code-intel``.
    """
    if platform_name == "windows":
        return
    binary = release_root / "bin" / release_binary_name(platform_name)
    if not binary.is_file():
        raise BootstrapError(
            f"Release payload is missing the packaged binary: {binary}"
        )
    if not os.access(binary, os.X_OK):
        binary.chmod(0o755)
    if not os.access(binary, os.X_OK):
        raise BootstrapError(
            f"Packaged binary is not executable after restoring permissions: {binary}"
        )


def command_result(
    command: list[str], *, environment: dict[str, str] | None = None
) -> dict[str, Any]:
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        env=environment,
    )
    result = {
        "exit_code": completed.returncode,
        "stdout": completed.stdout[-8000:],
        "stderr": completed.stderr[-4000:],
    }
    if completed.returncode != 0:
        raise BootstrapError(
            f"Command failed with exit code {completed.returncode}: "
            + subprocess.list2cmdline(command)
            + f"\n{completed.stderr[-2000:] or completed.stdout[-2000:]}"
        )
    return result


def install_release(asset: dict[str, str], install_root: Path) -> tuple[Path, str]:
    tag = asset["tag"]
    destination = install_root / tag
    marker = destination / RELEASE_MARKER
    existing_metadata: dict[str, Any] | None = None
    if destination.exists():
        if not marker.is_file():
            raise BootstrapError(
                f"Existing release directory is unmanaged; refusing to overwrite: {destination}"
            )
        try:
            metadata = json.loads(marker.read_text(encoding="utf-8"))
            existing_metadata = metadata if isinstance(metadata, dict) else {}
        except (OSError, json.JSONDecodeError):
            existing_metadata = {}

    install_root.mkdir(parents=True, exist_ok=True)
    staging_root = install_root / f".staging-{tag}-{uuid.uuid4().hex}"
    try:
        with tempfile.TemporaryDirectory(prefix="code-intel-download-") as temp:
            archive = Path(temp) / asset["name"]
            download_file(asset["url"], archive)
            actual_digest = sha256_file(archive)
            if actual_digest != asset["sha256"]:
                raise BootstrapError(
                    "Release checksum mismatch: "
                    f"expected {asset['sha256']}, received {actual_digest}"
                )
            safe_extract_zip(archive, staging_root)
        payload = find_payload_root(staging_root)
        verified_manifest = payload_manifest(payload)
        verified_manifest_digest = manifest_digest(verified_manifest)
        verified_metadata = {
            "schema": "code-intel-skill-release.v2",
            "tag": tag,
            "asset": asset["name"],
            "url": asset["url"],
            "sha256": asset["sha256"],
            "manifest_sha256": verified_manifest_digest,
            "files": verified_manifest,
        }
        payload_marker = payload / RELEASE_MARKER
        payload_marker.write_text(
            json.dumps(verified_metadata, indent=2, sort_keys=True)
            + "\n",
            encoding="utf-8",
        )
        if existing_metadata is not None:
            try:
                find_payload_root(destination)
                installed_manifest = payload_manifest(destination)
            except (BootstrapError, OSError):
                installed_manifest = {}
            if (
                existing_metadata == verified_metadata
                and installed_manifest == verified_manifest
            ):
                return destination, "already_installed"
            replaced = install_root / f".replaced-{tag}-{uuid.uuid4().hex}"
            destination.replace(replaced)
            try:
                payload.replace(destination)
            except BaseException:
                replaced.replace(destination)
                raise
            shutil.rmtree(replaced)
            return destination, "repaired"

        payload.replace(destination)
        return destination, "installed"
    finally:
        if staging_root.exists():
            shutil.rmtree(staging_root)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Install a verified Code Intel Pipeline GitHub Release."
    )
    parser.add_argument(
        "--repo-path",
        type=Path,
        default=Path.cwd(),
        help="Repository that the installer and doctor should validate.",
    )
    parser.add_argument("--version", help="Release tag, for example v0.2.0.")
    parser.add_argument(
        "--channel",
        choices=("stable", "prerelease"),
        default="stable",
        help="Release channel when --version is omitted. Stable uses GitHub's latest release.",
    )
    parser.add_argument(
        "--install-root",
        type=Path,
        default=default_install_root(),
        help="Versioned release installation directory.",
    )
    parser.add_argument(
        "--install-missing",
        action="store_true",
        help="Allow the pipeline installer to install missing third-party tools.",
    )
    parser.add_argument(
        "--check-provider",
        action="store_true",
        help="Run the optional provider connectivity check.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Resolve and report the release plan without downloading or installing.",
    )
    parser.add_argument("--json", action="store_true", help="Emit JSON.")
    return parser.parse_args(argv)


def run(args: argparse.Namespace) -> dict[str, Any]:
    platform_name = resolve_platform_name(sys.platform)
    repo_path = args.repo_path.expanduser().resolve()
    if not repo_path.is_dir():
        raise BootstrapError(f"Repository path does not exist: {repo_path}")
    install_root = args.install_root.expanduser().resolve()
    requested_version = resolve_version(args.version, args.channel)
    release = fetch_release(requested_version, args.channel)
    asset = select_release_asset(release, platform_name)
    actual_channel = "prerelease" if release.get("prerelease") else "stable"
    plan: dict[str, Any] = {
        "schema": "code-intel-skill-bootstrap.v1",
        "status": "planned" if args.dry_run else "installing",
        "channel": actual_channel,
        "platform": platform_name,
        "tag": asset["tag"],
        "version_source": (
            "explicit"
            if args.version
            else "channel"
        ),
        "asset": asset["name"],
        "url": asset["url"],
        "sha256": asset["sha256"],
        "install_path": str(install_root / asset["tag"]),
        "repo_path": str(repo_path),
        "install_missing": bool(args.install_missing),
        "check_provider": bool(args.check_provider),
    }
    if args.dry_run:
        return plan

    release_root, install_status = install_release(asset, install_root)
    ensure_release_binary_executable(release_root, platform_name)
    pwsh = find_pwsh()
    installer = release_root / "legacy/install-code-intel-pipeline.ps1"
    install_command = [
        pwsh,
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        str(installer),
        "-RepoPath",
        str(repo_path),
        "-RepairSkillLinks",
        "-Json",
    ]
    if args.install_missing:
        install_command.append("-InstallMissing")
    if args.check_provider:
        install_command.append("-CheckProvider")
    installer_result = command_result(install_command)

    environment = os.environ.copy()
    environment["CODE_INTEL_HOME"] = str(release_root)
    data_root = default_data_root()
    environment["PATH"] = (
        str(data_root / "bin") + os.pathsep + environment.get("PATH", "")
    )
    doctor_result = command_result(
        [
            pwsh,
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(release_root / "legacy/check-code-intel-tools.ps1"),
            "-RepoPath",
            str(repo_path),
            "-RequireRepowise:$false",
            "-Json",
        ],
        environment=environment,
    )
    plan.update(
        {
            "status": install_status,
            "release_root": str(release_root),
            "installer": installer_result,
            "doctor": doctor_result,
        }
    )
    return plan


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        result = run(args)
    except (BootstrapError, OSError, zipfile.BadZipFile) as error:
        if args.json:
            print(
                json.dumps(
                    {
                        "schema": "code-intel-skill-bootstrap.v1",
                        "status": "failed",
                        "error": str(error),
                    },
                    indent=2,
                )
            )
        else:
            print(f"Code Intel bootstrap failed: {error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print(
            f"Code Intel bootstrap {result['status']}: "
            f"{result['tag']} -> {result['install_path']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
