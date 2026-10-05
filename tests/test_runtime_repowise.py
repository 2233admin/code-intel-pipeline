"""Exercise an actual Repowise executable through the public Pipeline CLI."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

SAMPLES = Path(__file__).resolve().parent / "fixtures/runtime_tools/repowise"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--provider-bin", required=True)
    parser.add_argument("--previous-provider-bin")
    args = parser.parse_args()
    cli = str(Path(args.cli).resolve())
    provider = Path(args.provider_bin).resolve()
    if not provider.is_file():
        raise RuntimeError(f"Required real provider executable missing: {provider}")
    initial_provider = provider
    if args.previous_provider_bin:
        initial_provider = Path(args.previous_provider_bin).resolve()
        if not initial_provider.is_file():
            raise RuntimeError(f"Required recorded-version provider missing: {initial_provider}")
    with tempfile.TemporaryDirectory(prefix="cip-runtime-provider-") as directory:
        root = Path(directory)
        repo = root / "repo"
        repo.mkdir()
        source = repo / "example.py"
        source.write_text('def greet(name):\n    return "hello " + name\n\nprint(greet("world"))\n', encoding="utf-8")
        before = source.read_bytes()
        subprocess.run(["git", "init", "--quiet", str(repo)], check=True)
        environment = {
            key: value for key, value in os.environ.items()
            if not key.endswith(("_KEY", "_TOKEN"))
        }
        environment.update({
            "PATH": str(initial_provider.parent) + os.pathsep + environment["PATH"],
            "HOME": str(root / "home"),
            "USERPROFILE": str(root / "home"),
            "PYTHONUTF8": "1",
            "PYTHONIOENCODING": "utf-8",
            # Isolate the fixture's optional telemetry, not the production consumer.
            # Upstream documents this control; it avoids a detached flusher holding cwd.
            "DO_NOT_TRACK": "1",
        })
        operations = ("status", "index", "status", "index", "commit-fixture", "index")
        operations += ("switch-provider", "status", "index") if args.previous_provider_bin else ("index",)
        for operation in operations:
            if operation == "switch-provider":
                environment["PATH"] = str(provider.parent) + os.pathsep + environment["PATH"]
                print("Switching the real provider over the same recorded-version index", flush=True)
                continue
            if operation == "commit-fixture":
                subprocess.run(["git", "-C", str(repo), "add", "--", "example.py"], check=True)
                subprocess.run(
                    ["git", "-C", str(repo), "-c", "user.name=Runtime smoke",
                     "-c", "user.email=runtime-smoke@example.invalid",
                     "-c", "commit.gpgsign=false",
                     "-c", f"core.hooksPath={root / 'no-hooks'}",
                     "commit", "--quiet", "-m", "Own smoke source"],
                    check=True,
                )
                continue
            request = json.loads((SAMPLES / (operation + ".request.json")).read_text(encoding="utf-8"))
            argv = [str(repo) if value == "$SANDBOX" else value for value in request["argv"]]
            result = subprocess.run(
                [cli, *argv],
                cwd=repo, env=environment, capture_output=True, text=True,
                encoding="utf-8", timeout=180,
            )
            if result.returncode != 0:
                raise AssertionError(f"{operation}: exit {result.returncode}: {result.stderr}")
            response = json.loads(result.stdout)
            expected = json.loads((SAMPLES / (operation + ".approved.json")).read_text(encoding="utf-8"))["response"]
            actual = {key: response.get(key) for key in expected}
            if actual != expected:
                raise AssertionError(f"{operation}: expected {expected}, got {actual}; raw={response}")
            if source.read_bytes() != before:
                raise AssertionError(f"{operation}: provider modified input source")
            if operation == "index" and not Path(response["artifact"]).is_file():
                raise AssertionError("Successful local indexing did not produce its declared artifact")
            print(f"PASS real provider {operation}: {json.dumps(actual)}; sourceUnchanged=true", flush=True)


if __name__ == "__main__":
    main()
