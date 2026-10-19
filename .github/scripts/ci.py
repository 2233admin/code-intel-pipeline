"""CI and release helpers shared by every workflow (DR-0018: no PowerShell).

Each subcommand is one step's logic, so ci.yml, release.yml and pr-gate.yml
run the same checks on Windows, macOS and Linux with only Python's standard
library. Failures raise SystemExit with a message; GitHub shows it as the
step's error.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
PLATFORM = {"win32": "windows", "darwin": "macos"}.get(sys.platform, "linux")
EXE = ".exe" if PLATFORM == "windows" else ""
RELEASE_DIR = "code-intel-pipeline"
CAPABILITY_SCHEMA = "code-intel-sentrux-capability-artifact.v2"

# Pinned tool releases. orchestration/toolchain-versions.v1.json reads the
# ast-grep version from this line; keep each version a single literal.
RG_VERSION = "15.2.0"
AST_GREP_VERSION = "0.45.3"
TOOLS: dict[str, tuple[str, dict[str, tuple[str, str]]]] = {
    "rg": (
        RG_VERSION,
        {
            "windows": (
                f"ripgrep-{RG_VERSION}-x86_64-pc-windows-msvc.zip",
                "71b2fef860abe467217a538ff31de02f5258807c0129f771846f87bd029aafc5",
            ),
            "macos": (
                f"ripgrep-{RG_VERSION}-aarch64-apple-darwin.tar.gz",
                "3750b2e93f37e0c692657da574d7019a101c0084da05a790c83fd335bad973e4",
            ),
            "linux": (
                f"ripgrep-{RG_VERSION}-x86_64-unknown-linux-musl.tar.gz",
                "33e15bcf1624b25cdd2a55813a47a2f95dbe126268203e76aa6a585d1e7b149c",
            ),
        },
    ),
    "ast-grep": (
        AST_GREP_VERSION,
        {
            "windows": (
                "app-x86_64-pc-windows-msvc.zip",
                "3751b7d6be7fd39a80df1180ffe7e053903dcf92d3190e5a8336ff1746af9059",
            ),
            "macos": (
                "app-aarch64-apple-darwin.zip",
                "6d2279dea5bea2ad79c66ea93f5fe54ba926e398a8a26de76c56db68fe59eac6",
            ),
            "linux": (
                "app-x86_64-unknown-linux-gnu.zip",
                "f8ac830881339d1edee6b2652f54798c0f4da5a827f2db38a08ee31117783ce8",
            ),
        },
    ),
}
TOOL_URLS = {
    "rg": "https://github.com/BurntSushi/ripgrep/releases/download/{version}/{asset}",
    "ast-grep": "https://github.com/ast-grep/ast-grep/releases/download/{version}/{asset}",
}


def fail(message: str) -> None:
    # A workflow-command annotation is readable from the checks API; the raw
    # job log is not.
    encoded = message.replace("%", "%25").replace("\r", "").replace("\n", "%0A")
    print(f"::error title=ci.py::{encoded}", flush=True)
    raise SystemExit(f"error: {message}")


def run(command: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
    print("+ " + subprocess.list2cmdline(command), flush=True)
    return subprocess.run(command, text=True, encoding="utf-8", errors="replace", **kwargs)


def check(command: list[str], what: str, **kwargs: Any) -> subprocess.CompletedProcess[str]:
    completed = run(command, **kwargs)
    if completed.returncode != 0:
        fail(f"{what} failed with exit code {completed.returncode}")
    return completed


def github_file(variable: str, line: str) -> None:
    target = os.environ.get(variable)
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            handle.write(line + "\n")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def version_of(binary: str, pattern: str) -> tuple[int, ...] | None:
    try:
        output = subprocess.run(
            [binary, "--version"], capture_output=True, text=True, check=False
        ).stdout
    except OSError:
        return None
    match = re.search(pattern, output)
    return tuple(int(part) for part in match.group(1).split(".")) if match else None


# --- install-tool -----------------------------------------------------------


def install_tool(args: argparse.Namespace) -> None:
    name = args.tool
    version, assets = TOOLS[name]
    wanted = tuple(int(part) for part in version.split("."))
    pattern = r"^ripgrep (\d+\.\d+\.\d+)" if name == "rg" else r"ast-grep (\d+\.\d+\.\d+)"
    existing = shutil.which(name)
    # DR-0002: the rg pin is a floor; ast-grep must match exactly.
    if existing and name == "rg" and (version_of(existing, pattern) or ()) >= wanted:
        print(f"{name} already present at {existing} (floor {version})")
        return
    if PLATFORM == "macos" and platform.machine() != "arm64":
        fail("only Apple Silicon macOS runners are pinned")
    asset, expected = assets[PLATFORM]
    directory = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / name
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / asset
    url = TOOL_URLS[name].format(version=version, asset=asset)
    for attempt in range(3):
        try:
            urllib.request.urlretrieve(url, archive)
            break
        except OSError as error:
            if attempt == 2:
                fail(f"download {url}: {error}")
    actual = sha256(archive)
    if actual != expected:
        fail(f"{name} artifact digest mismatch: {actual}")
    if asset.endswith(".zip"):
        with zipfile.ZipFile(archive) as handle:
            handle.extractall(directory)
    else:
        with tarfile.open(archive) as handle:
            handle.extractall(directory, filter="data")
    binary_name = ("rg" if name == "rg" else "ast-grep") + EXE
    binary = next(directory.rglob(binary_name), None)
    if binary is None:
        fail(f"{binary_name} not found in {asset}")
    binary.chmod(0o755)
    if version_of(str(binary), pattern) != wanted:
        fail(f"{name} version mismatch at {binary}")
    github_file("GITHUB_PATH", str(binary.parent))
    print(f"installed {name} {version} -> {binary}")


# --- package ----------------------------------------------------------------


def add_zip_entry(archive: zipfile.ZipFile, name: str, data: bytes, mode: int) -> None:
    info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.create_system = 3  # Unix, so bootstrap restores the mode bits
    info.external_attr = (0o100000 | mode) << 16
    archive.writestr(info, data)


def package(args: argparse.Namespace) -> None:
    """Zip `git archive HEAD` plus the release binary, never the working tree."""
    binary = ROOT / "target" / "release" / f"code-intel{EXE}"
    if not binary.is_file():
        fail(f"release binary missing: {binary}")
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    zip_path = out / f"code-intel-pipeline-{args.tag}-{PLATFORM}.zip"
    tarball = subprocess.run(
        ["git", "archive", "--format=tar", "HEAD"], cwd=ROOT, capture_output=True, check=True
    ).stdout
    with zipfile.ZipFile(zip_path, "w") as archive, tarfile.open(
        fileobj=io.BytesIO(tarball)
    ) as tree:
        for member in tree.getmembers():
            if member.isfile():
                data = tree.extractfile(member).read()  # type: ignore[union-attr]
                add_zip_entry(archive, f"{RELEASE_DIR}/{member.name}", data, member.mode & 0o777)
        add_zip_entry(archive, f"{RELEASE_DIR}/bin/code-intel{EXE}", binary.read_bytes(), 0o755)
    digest = sha256(zip_path)
    Path(f"{zip_path}.sha256").write_text(f"{digest} *{zip_path.name}\n", encoding="ascii")
    toolchain = subprocess.run(["rustc", "--version"], capture_output=True, text=True, cwd=ROOT)
    manifest = {
        "schema": "code-intel-release-manifest.v1",
        "tag": args.tag,
        "commit": subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=ROOT, check=True
        ).stdout.strip(),
        "zipSha256": digest,
        "toolchain": toolchain.stdout.strip(),
        "builtAt": datetime.now(timezone.utc).isoformat(),
    }
    Path(f"{zip_path}.release-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="ascii"
    )
    github_file("GITHUB_OUTPUT", f"platform={PLATFORM}")
    github_file("GITHUB_OUTPUT", f"zip={zip_path}")
    print(f"packaged {zip_path} sha256={digest}")


# --- install-package --------------------------------------------------------


def install_package(args: argparse.Namespace) -> None:
    """Install a packaged zip exactly as a user would: through the Skill bootstrap."""
    zip_path = Path(args.zip).resolve()
    with zipfile.ZipFile(zip_path) as archive:
        names = set(archive.namelist())
    required = [
        "skills/code-intel-pipeline/SKILL.md",
        "skills/code-intel-pipeline/agents/openai.yaml",
        "skills/code-intel-pipeline/scripts/bootstrap.py",
        f"bin/code-intel{EXE}",
    ]
    missing = [path for path in required if f"{RELEASE_DIR}/{path}" not in names]
    if missing:
        fail(f"release package is missing required files: {missing}")
    install_root = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / f"code-intel-install-{args.tag}"
    completed = check(
        [
            sys.executable,
            str(ROOT / "skills/code-intel-pipeline/scripts/bootstrap.py"),
            "--local-asset", str(zip_path),
            "--version", args.tag,
            "--repo-path", str(Path(args.repo_path).resolve()),
            "--install-root", str(install_root),
            "--json",
        ],
        "bootstrap install",
        capture_output=True,
    )
    result = json.loads(completed.stdout)
    if result.get("status") not in ("installed", "repaired", "already_installed"):
        fail(f"unexpected bootstrap status: {completed.stdout}")
    Path(args.report).write_text(json.dumps(result, indent=2), encoding="utf-8")
    setup = result["setup"]
    print(f"bootstrap {result['status']} {result['tag']} -> {result['release_root']}")
    for hint in result.get("missing_tools", []):
        print(f"::warning::missing {hint['tool']}: {hint['install']}")
    github_file("GITHUB_PATH", setup["paths"]["bin"])
    github_file("GITHUB_OUTPUT", f"release_root={result['release_root']}")
    github_file("GITHUB_OUTPUT", f"release_bin={Path(result['release_root']) / 'bin'}")


def installed_report(path: str) -> tuple[dict[str, Any], Path, Path]:
    report = read_json(Path(path))
    release_root = Path(report["release_root"]).resolve()
    binary = Path(report["setup"]["paths"]["bin"]) / f"code-intel{EXE}"
    return report, release_root, binary


# --- installed-checks -------------------------------------------------------


def installed_checks(args: argparse.Namespace) -> None:
    """The installed binary finds itself as the graph provider, and setup's
    PATH and CODE_INTEL_HOME reach a fresh shell."""
    report, release_root, binary = installed_report(args.report)
    observation = run(
        [str(binary), "doctor", "bootstrap", "--pipeline-root", str(release_root),
         "--repo-path", str(ROOT), "--no-require-repowise", "--json"],
        capture_output=True,
    )
    if observation.returncode not in (0, 1):
        fail(f"installed doctor failed with exit code {observation.returncode}: {observation.stderr}")
    graph = json.loads(observation.stdout)["checks"]["graphProvider"]
    if not graph.get("binaryFound"):
        fail(f"installed doctor did not find its graph binary: {graph}")
    packaged = release_root / "bin" / f"code-intel{EXE}"
    if Path(graph["binaryPath"]).resolve() != packaged:
        fail(f"installed doctor resolved {graph['binaryPath']} instead of {packaged}")
    print(f"installed doctor ({binary}) found the packaged graph binary {packaged}")

    bin_dir = str(Path(report["setup"]["paths"]["bin"]))
    home = report["setup"]["paths"]["codeIntelHome"]
    if PLATFORM == "windows":
        import winreg

        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment") as key:
            user_path, path_type = winreg.QueryValueEx(key, "Path")
            persisted_home, _ = winreg.QueryValueEx(key, "CODE_INTEL_HOME")
        entries = [entry.rstrip("\\").lower() for entry in user_path.split(";")]
        if bin_dir.rstrip("\\").lower() not in entries:
            fail(f"user Path lacks {bin_dir}: {user_path}")
        if path_type != winreg.REG_EXPAND_SZ:
            fail("user Path was downgraded from REG_EXPAND_SZ")
        if Path(persisted_home).resolve() != Path(home).resolve():
            fail(f"CODE_INTEL_HOME is {persisted_home}, expected {home}")
        print(f"user environment: {bin_dir} on Path, CODE_INTEL_HOME={persisted_home}")
        return
    # A fresh shell with a minimal environment must pick both up from the
    # startup files setup edited, with no manual profile step.
    shells = [["bash", "-l", "-c"]] + ([["zsh", "-c"]] if shutil.which("zsh") else [])
    for shell in shells:
        probe = run(
            shell + ['printf "%s\\n%s\\n" "$PATH" "$CODE_INTEL_HOME"'],
            capture_output=True,
            env={"HOME": os.environ["HOME"], "PATH": "/usr/bin:/bin"},
        )
        lines = probe.stdout.splitlines()
        if len(lines) < 2 or bin_dir not in lines[-2].split(":") or lines[-1] != home:
            fail(f"fresh {shell[0]} shell lacks setup's environment:\n{probe.stdout}{probe.stderr}")
        print(f"fresh {shell[0]} shell: {bin_dir} on PATH, CODE_INTEL_HOME={home}")


# --- sentrux-closure --------------------------------------------------------


def sentrux_closure(args: argparse.Namespace) -> None:
    """Run the pipeline and accept it only through its committed artifacts:
    the completion marker, the content-addressed manifest and every Sentrux
    capability ref the artifact index re-admits. Stdout is never authority."""
    binary = Path(args.binary).resolve()
    if not binary.is_file():
        binary = binary.with_name(binary.name + EXE)
    repo = Path(args.repo).resolve()
    work = Path(args.work).resolve()
    staging, authority = work / "staging", work / "authority"
    authority.mkdir(parents=True, exist_ok=True)
    final = args.final_name
    completed = run(
        [str(binary), "run", "execute", "--repo", str(repo), "--out", str(staging),
         "--authority-root", str(authority), "--final-name", final,
         "--manifest", str(repo / "orchestration/integrations.json"),
         "--doctor-require-repowise", "false"],
        capture_output=True,
    )
    print(completed.stdout, completed.stderr, sep="\n", flush=True)
    if completed.returncode != 0:
        try:
            failures = json.dumps(json.loads(completed.stdout).get("failures"), indent=1)
        except (json.JSONDecodeError, AttributeError):
            failures = completed.stderr[-2000:]
        fail(
            f"run execute failed with exit code {completed.returncode} (10 = architecture/domain "
            f"gate failure, 70 = process failure, 73 = --final-name already published, 76 = "
            f"budget stopped):\n{failures}"
        )
    index_path = authority / "index.json"
    check([str(binary), "artifact", "index", "--artifact-root", str(authority),
           "--output", str(index_path), "--operation", "rebuild"], "artifact index rebuild")

    run_root = authority / repo.name / final
    marker = read_json(run_root / "run-complete.json")
    if marker.get("schema") != "code-intel-run-commit.v1":
        fail("run completion marker is invalid")
    manifest = read_json(run_root / marker["manifest"]["path"])
    if manifest.get("schema") != "code-intel-run-manifest.v1" or manifest.get("outcome") != "completed":
        fail("run manifest is not completed")
    snapshot = marker["snapshotIdentity"]
    if manifest.get("snapshotIdentity") != snapshot:
        fail("completion marker and run manifest snapshots differ")
    if args.require_pass:
        node = manifest.get("nodes", {}).get("evidence.sentrux", {})
        if node.get("status") != "succeeded" or node.get("verdict") != "pass":
            fail(f"Sentrux evidence node is not a passing verified result: {node}")

    entries = [
        entry for entry in read_json(index_path).get("entries", [])
        if entry.get("repo") == repo.name and entry.get("run") == final
    ]
    if len(entries) != 1:
        fail("run was not admitted exactly once by the artifact index")
    entry = entries[0]
    if entry.get("outcome") != "completed" or entry.get("snapshotIdentity") != snapshot:
        fail("artifact index identity does not match the completion marker")
    refs = entry.get("artifactRefs", [])
    if sum(1 for ref in refs if ref.get("type") == "diagnosis.hospital") != 1:
        fail("run is missing its verified hospital report ref")
    capability_refs = [
        ref for ref in refs
        if ref.get("artifactSchema") == CAPABILITY_SCHEMA
        and ref.get("type") == "provider.sentrux.capability-artifact"
    ]
    if not capability_refs:
        fail("run has no verified Sentrux capability refs")

    matrix = read_json(repo / "orchestration/sentrux-capability-matrix.v1.json")
    by_id = {capability["id"]: capability for capability in matrix["capabilities"]}
    observed: dict[str, dict[str, Any]] = {}
    for ref in capability_refs:
        if ref["path"] != f"objects/sha256/{ref['sha256']}":
            fail(f"capability ref is not content-addressed: {ref['path']}")
        object_path = run_root / ref["path"]
        if not object_path.is_file() or sha256(object_path) != ref["sha256"]:
            fail(f"capability object is missing or its digest differs: {ref['path']}")
        payload = read_json(object_path)
        capability_id = payload.get("capabilityId")
        if payload.get("schema") != CAPABILITY_SCHEMA or payload.get("snapshotIdentity") != snapshot:
            fail(f"capability artifact is not snapshot-bound: {ref['path']}")
        if capability_id not in by_id:
            fail(f"capability is absent from the matrix: {capability_id}")
        if by_id[capability_id]["operation"] != payload.get("operation"):
            fail(f"capability operation disagrees with the matrix: {capability_id}")
        if capability_id in observed:
            fail(f"duplicate capability artifact: {capability_id}")
        if payload.get("status") != "succeeded" and payload.get("failure") is None:
            fail(f"non-success capability has no explicit failure: {capability_id}")
        observed[capability_id] = payload

    required = [capability for capability in matrix["capabilities"] if capability.get("requiredForRelease")]
    if not required:
        fail("capability matrix exposes no required capabilities")
    for capability in required:
        capability_id = capability["id"]
        payload = observed.get(capability_id)
        if payload is None:
            fail(f"required Sentrux capability is missing: {capability_id}")
        status, mode, state = payload.get("status"), capability.get("executionMode"), capability.get("currentState")
        if mode == "automatic":
            accepted = {"authoritative_automatic": {"succeeded"}, "automatic_degraded": {"succeeded", "degraded"}}
            if state not in accepted:
                fail(f"automatic capability has an unsupported matrix state: {capability_id} ({state})")
            if status not in accepted[state]:
                fail(f"automatic capability {capability_id} ({state}) ended {status}")
        elif mode in ("explicit_authority", "lifecycle_external"):
            if status != "not_applicable":
                fail(f"explicit/lifecycle capability must be not_applicable: {capability_id} ({status})")
            if not str((payload.get("failure") or {}).get("kind") or "").strip():
                fail(f"explicit/lifecycle capability has no failure reason: {capability_id}")
        else:
            fail(f"capability has no recognized execution mode: {capability_id} ({mode})")
    print(
        f"Sentrux closure verified from committed refs: {len(capability_refs)} capability refs; "
        f"required={len(required)}; matrix coverage={matrix.get('coverageStatus')}"
    )

    # ai-safety-003 (#34): an audit report anywhere in the scanned snapshot or
    # the run output must pass the fail-closed validator before it can be
    # treated as authoritative.
    reports = {path.resolve() for root in (repo, staging) if root.is_dir()
               for path in root.rglob("audit-report.json")}
    for report in sorted(reports):
        check([str(binary), "audit", "--operation", "validate", "--repo", str(repo),
               "--report", str(report)], f"audit report validation of {report}")
    if not reports:
        print("no audit-report.json in the snapshot or run output")


# --- secret-guard -----------------------------------------------------------


SECRET_PATTERNS = [
    re.compile(pattern)
    for pattern in (
        r"sk-[A-Za-z0-9_-]{20,}",
        r"ghp_[A-Za-z0-9_]{20,}",
        r"github_pat_[A-Za-z0-9_]{20,}",
        r"AKIA[0-9A-Z]{16}",
    )
]


def secret_guard(_args: argparse.Namespace) -> None:
    files = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout.decode().split("\0")
    hits = []
    for name in filter(None, files):
        if re.match(r"^(target/|artifacts/|cache/)", name) or re.search(r"\.(png|jpg|jpeg|gif|zip|exe)$", name):
            continue
        try:
            text = (ROOT / name).read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        hits += [f"{name} matches {pattern.pattern}" for pattern in SECRET_PATTERNS if pattern.search(text)]
    if hits:
        print("\n".join(hits))
        fail("secret-like patterns found")
    print(f"no secret-like patterns in {len(files)} tracked files")


# --- quality-summary --------------------------------------------------------


def shown(value: Any) -> str:
    """A projection value as the summary prints it: no Python repr leaks."""
    if value is None:
        return ""
    if isinstance(value, (dict, list)):
        return json.dumps(value, sort_keys=True)
    return str(value)


def quality_summary(args: argparse.Namespace) -> None:
    """Render the Quality Signal projection as the job summary, a sticky
    comment body and annotations. The projection itself comes from the
    compiled CLI (#386); this only formats it."""
    projection = read_json(Path(args.projection))
    signal = projection["qualitySignal"]
    total = signal["total"]
    delta = total.get("delta")
    admission = projection.get("admission") or {}
    policy = admission.get("policy") or {}
    policy_text = f"{policy['policyId']} v{policy.get('policyVersion')}" if policy.get("policyId") else "unbound"
    regression = signal.get("regression")
    if isinstance(regression, dict):
        regression = regression.get("status")

    def short(value: Any, length: int) -> str:
        return str(value or "")[:length]

    lines = [
        "### Code Intel Quality Signal",
        "",
        f"Completeness: **{projection.get('completeness')}** &middot; snapshot "
        f"`{short(projection.get('snapshotIdentity'), 12)}` &middot; commit "
        f"`{short(projection.get('binding', {}).get('commit'), 12)}`",
        f"Admission: **{admission.get('verdict') or 'unknown'}** &middot; policy **{policy_text}** "
        f"&middot; aggregate regression: **{shown(regression)}** (advisory, not a blocking gate)",
        "",
        "| Total | Baseline | Delta |",
        "|---|---|---|",
        f"| {shown(total.get('current'))} | {shown(total.get('baseline'))} | "
        f"{'[DOWN]' if delta is not None and delta < 0 else '[OK]'} {'n/a' if delta is None else delta} |",
        "",
        f"Bottleneck: **{signal.get('bottleneck') or 'none'}**",
        "",
        "| Root cause | Baseline | Current | Delta |",
        "|---|---|---|---|",
    ]
    for cause in signal.get("rootCauses") or []:
        raw = cause.get("raw") or {}
        raw_delta = "n/a" if raw.get("delta") is None else round(float(raw["delta"]), 2)
        lines.append(f"| {cause.get('label')} | {shown(raw.get('baseline'))} | {shown(raw.get('current'))} | {raw_delta} |")
    lines.append("")
    findings = projection.get("findings") or []
    if findings:
        lines += [
            f"**Findings** ({len(findings)} total)",
            "",
            "| Kind | Severity | Disposition | Rule | Message |",
            "|---|---|---|---|---|",
        ]
        lines += [
            f"| {f.get('kind')} | {f.get('severityNormalized')} | {f.get('disposition')} | "
            f"`{f.get('rule')}` | {f.get('message')} |"
            for f in findings[:10]
        ]
        lines.append("")
    lines += [f"> {diagnostic}" for diagnostic in projection.get("diagnostics") or []]
    text = "\n".join(lines) + "\n"
    github_file("GITHUB_STEP_SUMMARY", text)
    Path(args.comment).write_text("<!-- code-intel-quality-signal -->\n" + text, encoding="utf-8")
    for finding in findings:
        label = f"[{finding.get('rule')}]: {finding.get('message')}"
        if finding.get("disposition") == "advisory":
            print(f"::warning::Quality Signal advisory {label}")
        elif finding.get("severityNormalized") in ("high", "critical"):
            targets = finding.get("targets") or []
            suffix = f" ({', '.join(targets)})" if targets else ""
            print(f"::error::Quality Signal {finding.get('kind')} {label}{suffix}")
        else:
            print(f"::warning::Quality Signal {finding.get('kind')} {label}")


# --- verify-inventory -------------------------------------------------------


def verify_inventory(args: argparse.Namespace) -> None:
    """Exactly one zip, checksum sidecar and manifest per platform, with real digests."""
    directory = Path(args.dir)
    entries = list(directory.iterdir())
    if len(entries) != 9 or any(entry.is_dir() for entry in entries):
        fail(f"release inventory must be nine flat files, found {sorted(e.name for e in entries)}")
    for target in ("windows", "macos", "linux"):
        zip_path = directory / f"code-intel-pipeline-{args.tag}-{target}.zip"
        sidecar = Path(f"{zip_path}.sha256")
        manifest_path = Path(f"{zip_path}.release-manifest.json")
        for path in (zip_path, sidecar, manifest_path):
            if not path.is_file():
                fail(f"release asset is missing: {path}")
        digest = sha256(zip_path)
        expected = f"{digest} *{zip_path.name}"
        if sidecar.read_text(encoding="ascii").strip() != expected:
            fail(f"checksum sidecar mismatch for {zip_path}: expected '{expected}'")
        manifest = read_json(manifest_path)
        if (
            manifest.get("schema") != "code-intel-release-manifest.v1"
            or manifest.get("tag") != args.tag
            or manifest.get("zipSha256") != digest
            or (args.commit and manifest.get("commit") != args.commit)
        ):
            fail(f"release manifest identity/digest mismatch for {zip_path}")
        print(f"verified {zip_path.name} sha256={digest}")


# --- release-tag ------------------------------------------------------------


def release_tag(args: argparse.Namespace) -> None:
    """The tag is v<Cargo version>; GA (non beta/rc) tags must be signed."""
    match = re.fullmatch(r"v([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?)", args.tag)
    if not match:
        fail(f"unsupported release tag: {args.tag}")
    cargo = (ROOT / "crates/code-intel-cli/Cargo.toml").read_text(encoding="utf-8")
    version = re.search(r'(?m)^version\s*=\s*"([^"]+)"', cargo)
    if not version or version.group(1) != match.group(1):
        fail(f"release tag {args.tag} does not match Cargo package version {version and version.group(1)}")
    if re.search(r"-(beta|rc)\.[0-9]+$", args.tag):
        print(f"pre-release tag {args.tag}: signed-tag gate skipped")
        return
    check(["git", "config", "gpg.ssh.allowedSignersFile", ".github/allowed_signers"], "git config", cwd=ROOT)
    check(["git", "verify-tag", args.tag], f"signature verification of {args.tag} (see docs/RELEASE_SIGNING.md)", cwd=ROOT)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    command = commands.add_parser("install-tool")
    command.add_argument("tool", choices=sorted(TOOLS))
    command.set_defaults(handler=install_tool)

    command = commands.add_parser("package")
    command.add_argument("--tag", required=True)
    command.add_argument("--out", default="dist")
    command.set_defaults(handler=package)

    command = commands.add_parser("install-package")
    command.add_argument("--zip", required=True)
    command.add_argument("--tag", required=True)
    command.add_argument("--repo-path", default=".")
    command.add_argument("--report", required=True)
    command.set_defaults(handler=install_package)

    command = commands.add_parser("installed-checks")
    command.add_argument("--report", required=True)
    command.set_defaults(handler=installed_checks)

    command = commands.add_parser("sentrux-closure")
    command.add_argument("--binary", required=True)
    command.add_argument("--repo", default=".")
    command.add_argument("--work", required=True)
    command.add_argument("--final-name", required=True)
    command.add_argument("--require-pass", action="store_true")
    command.set_defaults(handler=sentrux_closure)

    command = commands.add_parser("secret-guard")
    command.set_defaults(handler=secret_guard)

    command = commands.add_parser("quality-summary")
    command.add_argument("--projection", required=True)
    command.add_argument("--comment", required=True)
    command.set_defaults(handler=quality_summary)

    command = commands.add_parser("verify-inventory")
    command.add_argument("--dir", default="dist")
    command.add_argument("--tag", required=True)
    command.add_argument("--commit")
    command.set_defaults(handler=verify_inventory)

    command = commands.add_parser("release-tag")
    command.add_argument("--tag", required=True)
    command.set_defaults(handler=release_tag)

    args = parser.parse_args()
    args.handler(args)


if __name__ == "__main__":
    main()
