"""Real dependency-tool smoke; explicitly selected by CI, never silently skipped."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SAMPLES = ROOT / "tests/fixtures/dependency_tools/npm_guard"


def tracked_sources():
    result = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    )
    return {
        name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
        for name in result.stdout.decode("utf-8").split("\0")
        if name and (ROOT / name).is_file()
    }


def sandbox_sources(repository):
    sources = {}
    for directory, directories, filenames in os.walk(repository):
        # Fingerprint working files, including new/ignored sources, not Git's
        # index, locks, refs, or other internal bookkeeping.
        directories[:] = [name for name in directories if name != ".git"]
        for name in filenames:
            if name == ".git":
                continue
            path = Path(directory) / name
            sources[path.relative_to(repository).as_posix()] = hashlib.sha256(
                path.read_bytes()
            ).hexdigest()
    return sources


def queue_smoke(binary):
    binary = Path(binary).resolve()
    if not binary.is_file():
        raise RuntimeError(f"Required real npm queue executable is missing: {binary}")
    environment = dict(os.environ)
    environment.pop("CLAUDE_CODE_MERGE_QUEUE_EMERGENCY_PUSH", None)
    environment.pop("CLAUDE_CODE_MERGE_QUEUE_LANDING", None)
    version = subprocess.run(
        ["node", str(binary), "--version"], cwd=ROOT, env=environment,
        capture_output=True, text=True, encoding="utf-8", check=True, timeout=30
    ).stdout.strip()
    print(f"Actual queue version: {version}", flush=True)
    with tempfile.TemporaryDirectory(prefix="cip-queue-guard-") as directory:
        sandbox = Path(directory)
        subprocess.run(["git", "init", "--quiet", str(sandbox)], check=True)
        marker = sandbox / "acceptance-invoked"
        sentinel = sandbox / "acceptance.py"
        sentinel.write_text(
            "from pathlib import Path\n"
            "Path(__file__).with_name('acceptance-invoked').write_text('invoked')\n"
            "raise SystemExit(7)\n", encoding="utf-8"
        )
        # Only the consequential acceptance boundary is substituted. A guard bug
        # must not launch the project's acceptance/Cargo path on the isolated host.
        command = f'"{sys.executable}" "{sentinel}"'
        config_url = (ROOT / "claude-code-merge-queue.config.mjs").as_uri()
        (sandbox / "claude-code-merge-queue.config.mjs").write_text(
            f"import config from {json.dumps(config_url)};\n"
            f"export default {{...config, checkCommand: {json.dumps(command)}}};\n",
            encoding="utf-8"
        )
        subprocess.run(
            ["git", "add", "--", "acceptance.py", "claude-code-merge-queue.config.mjs"],
            cwd=sandbox, check=True,
        )
        for request_path in sorted(SAMPLES.glob("*.request.json")):
            name = request_path.name.removesuffix(".request.json")
            request = json.loads(request_path.read_text(encoding="utf-8"))
            approved = json.loads(
                request_path.with_name(name + ".approved.json").read_text(encoding="utf-8")
            )["response"]
            before = sandbox_sources(sandbox)
            checkout_before = tracked_sources()
            result = subprocess.run(
                ["node", str(binary), *request["argv"]], input=request["stdin"],
                cwd=sandbox, env=environment, capture_output=True, text=True, encoding="utf-8", timeout=30
            )
            actual = {
                "exitCode": result.returncode,
                "trackedSourceUnchanged": (
                    sandbox_sources(sandbox) == before
                    and tracked_sources() == checkout_before
                ),
            }
            if actual != approved or marker.exists():
                raise AssertionError(
                    f"{name}: expected {approved}, got {actual}; "
                    f"acceptanceInvoked={marker.exists()}; stdout={result.stdout!r}; "
                    f"stderr={result.stderr!r}"
                )
            print(f"PASS {name}: {json.dumps(actual)}; acceptanceInvoked=false", flush=True)




def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--queue-bin", default=str(ROOT / "node_modules/claude-code-merge-queue/dist/bin/claude-code-merge-queue.js")
    )
    args = parser.parse_args()
    queue_smoke(args.queue_bin)


if __name__ == "__main__":
    main()
