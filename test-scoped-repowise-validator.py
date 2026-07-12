from __future__ import annotations

import asyncio
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


class FakeFileTraverser:
    def __init__(self, repo_root: Path, **_kwargs) -> None:
        self.repo_root = Path(repo_root)

    def traverse(self):
        for path in sorted(self.repo_root.rglob("*.py")):
            if any(part in {".git", ".repowise"} for part in path.parts):
                continue
            relative = path.relative_to(self.repo_root).as_posix()
            if relative == "src/ignored.py" or path.stat().st_size > 500 * 1024:
                continue
            yield SimpleNamespace(
                path=relative,
                abs_path=str(path),
                language="python",
                size_bytes=path.stat().st_size,
                is_entry_point=False,
            )

    def get_repo_structure(self, files=None):
        if files is None:
            files = list(self.traverse())
        return SimpleNamespace(total_files=len(files))


def install_repowise_stubs() -> None:
    """Make the production module importable without installing Repowise."""
    module_names = [
        "repowise",
        "repowise.cli",
        "repowise.cli.helpers",
        "repowise.core",
        "repowise.core.generation",
        "repowise.core.ingestion",
        "repowise.core.persistence",
        "repowise.core.providers",
        "repowise.core.providers.llm",
        "repowise.core.providers.llm.registry",
    ]
    stubs = {name: types.ModuleType(name) for name in module_names}
    for name, module in stubs.items():
        if name.rpartition(".")[2] not in {"helpers", "generation", "ingestion", "persistence", "registry"}:
            module.__path__ = []  # type: ignore[attr-defined]
        sys.modules[name] = module

    helpers = stubs["repowise.cli.helpers"]
    helpers.get_db_url_for_repo = lambda *_args, **_kwargs: "sqlite://"
    helpers.save_config = lambda *_args, **_kwargs: None
    helpers.save_state = lambda *_args, **_kwargs: None

    class Unused:
        def __init__(self, *_args, **_kwargs) -> None:
            pass

    generation = stubs["repowise.core.generation"]
    generation.ContextAssembler = Unused
    generation.GenerationConfig = Unused
    generation.PageGenerator = Unused

    ingestion = stubs["repowise.core.ingestion"]
    ingestion.ASTParser = Unused
    ingestion.FileTraverser = FakeFileTraverser
    ingestion.GraphBuilder = Unused

    persistence = stubs["repowise.core.persistence"]
    for name in (
        "FullTextSearch",
        "create_engine",
        "create_session_factory",
        "get_session",
        "init_db",
        "upsert_page_from_generated",
        "upsert_repository",
    ):
        setattr(persistence, name, Unused)

    stubs["repowise.core.providers.llm.registry"].get_provider = (
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("provider must be monkeypatched"))
    )


install_repowise_stubs()
SCRIPT_PATH = Path(__file__).with_name("Run-ScopedRepowiseDocs.py")
SPEC = importlib.util.spec_from_file_location("run_scoped_repowise_docs", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT_PATH}")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ScopedRepowiseValidatorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="cip-repowise-validator-")
        self.repo = Path(self.temp_dir.name) / "repo"
        (self.repo / "src").mkdir(parents=True)
        (self.repo / "src" / "app.py").write_text("VALUE = 'clean'\n", encoding="utf-8")
        self._git("init", "-q")
        self._git("config", "user.email", "test@example.invalid")
        self._git("config", "user.name", "Code Intel Test")
        self._git("add", "src/app.py")
        self._git("commit", "-qm", "fixture")
        self.manifest_path = self.repo / ".repowise" / "egress-manifest.json"
        self._write_manifest()

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _git(self, *args: str) -> str:
        return subprocess.check_output(
            ["git", "-C", str(self.repo), *args],
            text=True,
            stderr=subprocess.STDOUT,
        ).strip()

    def _manifest(self) -> dict[str, object]:
        path = self.repo / "src" / "app.py"
        return {
            "schema_version": 2,
            "generated_at_utc": "2026-07-13T00:00:00Z",
            "head": self._git("rev-parse", "HEAD"),
            "scope": {"paths": ["src"], "root_files": []},
            "scope_inventory": [
                {
                    "path": "src/app.py",
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                }
            ],
            "provider_payload_state": "pending",
            "provider_payload": [],
            "provider": "mock",
            "working_tree_policy": "head-tracked-only",
        }

    def _write_manifest(self, mutate=None) -> None:
        manifest = self._manifest()
        if mutate is not None:
            mutate(manifest)
        self.manifest_path.parent.mkdir(parents=True, exist_ok=True)
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    def _generate(self) -> None:
        env = {
            "CODE_INTEL_PROVIDER": "",
            "CODE_INTEL_MODEL": "",
            "CODE_INTEL_API_KEY": "",
            "CODE_INTEL_BASE_URL": "",
        }
        with mock.patch.dict(os.environ, env), mock.patch.object(
            MODULE, "get_provider", side_effect=AssertionError("provider constructed before validation")
        ):
            asyncio.run(
                MODULE.generate_docs(
                    self.repo,
                    0.02,
                    1,
                    "mock",
                    "",
                    "auto",
                    self.manifest_path,
                )
            )

    def test_rejects_schema_before_provider_construction(self) -> None:
        self._write_manifest(lambda manifest: manifest.__setitem__("schema_version", 1))
        with self.assertRaisesRegex(RuntimeError, "schema_version"):
            self._generate()

    def test_rejects_changed_head_before_provider_construction(self) -> None:
        (self.repo / "README.md").write_text("new commit\n", encoding="utf-8")
        self._git("add", "README.md")
        self._git("commit", "-qm", "advance head")
        with self.assertRaisesRegex(RuntimeError, "HEAD"):
            self._generate()

    def test_rejects_hash_mismatch_before_provider_construction(self) -> None:
        self._write_manifest(
            lambda manifest: manifest["scope_inventory"][0].__setitem__("sha256", "0" * 64)  # type: ignore[index]
        )
        with self.assertRaisesRegex(RuntimeError, "hash mismatch"):
            self._generate()

    def test_rejects_provider_mismatch_before_provider_construction(self) -> None:
        self._write_manifest(lambda manifest: manifest.__setitem__("provider", "anthropic"))
        with self.assertRaisesRegex(RuntimeError, "provider"):
            self._generate()

    def test_rejects_unmanifested_traversal_before_provider_construction(self) -> None:
        (self.repo / "src" / "extra.py").write_text("EXTRA = True\n", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "absent from egress manifest"):
            self._generate()

    def test_mixed_scope_freezes_only_real_provider_payload(self) -> None:
        asset_path = self.repo / "src" / "opaque.bin"
        asset_path.write_bytes(b"not a traversable source file")
        ignored_path = self.repo / "src" / "ignored.py"
        ignored_path.write_text("IGNORED = True\n", encoding="utf-8")
        oversized_path = self.repo / "src" / "oversized.py"
        oversized_path.write_bytes(b"#" * (501 * 1024))

        def add_filtered_files(manifest) -> None:
            manifest["working_tree_policy"] = "include-working-tree"
            for relative, path in (
                ("src/ignored.py", ignored_path),
                ("src/opaque.bin", asset_path),
                ("src/oversized.py", oversized_path),
            ):
                manifest["scope_inventory"].append(
                    {"path": relative, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                )
            manifest["scope_inventory"].sort(key=lambda entry: entry["path"])

        self._write_manifest(add_filtered_files)
        manifest = MODULE.validate_egress_manifest(self.repo, self.manifest_path, "mock")
        traverser = MODULE.FileTraverser(self.repo, extra_exclude_patterns=[".repowise/**"])
        file_infos = list(traverser.traverse())
        sources = MODULE.freeze_provider_payload(self.repo, self.manifest_path, file_infos, manifest)
        frozen = MODULE.validate_egress_manifest(
            self.repo, self.manifest_path, "mock", require_frozen=True
        )
        self.assertEqual({"src/app.py"}, {path.replace("\\", "/") for path in sources})
        self.assertEqual(["src/app.py"], [entry["path"] for entry in frozen["provider_payload"]])
        self.assertEqual(4, len(frozen["scope_inventory"]))

    def test_rejects_toctou_change_before_provider_construction(self) -> None:
        app_path = self.repo / "src" / "app.py"

        class MutatingTraverser:
            def __init__(self, *_args, **_kwargs) -> None:
                pass

            def traverse(self):
                app_path.write_text("VALUE = 'changed after validation'\n", encoding="utf-8")
                return [
                    SimpleNamespace(
                        path="src/app.py",
                        abs_path=str(app_path),
                        language="python",
                        size_bytes=app_path.stat().st_size,
                        is_entry_point=False,
                    )
                ]

            def get_repo_structure(self, files=None):
                if files is None:
                    raise AssertionError("get_repo_structure triggered a second traversal")
                return SimpleNamespace()

        with mock.patch.object(MODULE, "FileTraverser", MutatingTraverser):
            with self.assertRaisesRegex(RuntimeError, "hash mismatch"):
                self._generate()

    def test_head_tracked_policy_rejects_dirty_file_even_with_matching_manifest(self) -> None:
        app_path = self.repo / "src" / "app.py"
        app_path.write_text("VALUE = 'dirty but rehashed'\n", encoding="utf-8")
        self._write_manifest()
        with self.assertRaisesRegex(RuntimeError, "HEAD blob"):
            self._generate()

    def test_valid_manifest_validates_and_reads_exact_sources(self) -> None:
        manifest = MODULE.validate_egress_manifest(self.repo, self.manifest_path, "mock")
        traverser = MODULE.FileTraverser(self.repo, extra_exclude_patterns=[".repowise/**"])
        file_infos = list(traverser.traverse())
        sources = MODULE.freeze_provider_payload(self.repo, self.manifest_path, file_infos, manifest)
        frozen = MODULE.validate_egress_manifest(
            self.repo, self.manifest_path, "mock", require_frozen=True
        )
        self.assertEqual({"src/app.py"}, {path.replace("\\", "/") for path in sources})
        self.assertEqual((self.repo / "src" / "app.py").read_bytes(), next(iter(sources.values())))
        self.assertEqual("frozen", frozen["provider_payload_state"])
        self.assertEqual(
            hashlib.sha256(next(iter(sources.values()))).hexdigest(),
            frozen["provider_payload"][0]["sha256"],
        )

    def test_generate_freezes_exact_payload_before_provider_construction(self) -> None:
        class ProviderReached(Exception):
            pass

        class FakeParser:
            def parse_file(self, file_info, source):
                return SimpleNamespace(path=file_info.path, source=source)

        class FakeGraph:
            def __init__(self, *_args, **_kwargs) -> None:
                pass

            def add_file(self, _parsed) -> None:
                pass

            def build(self) -> None:
                pass

            def add_framework_edges(self, _names) -> None:
                pass

        def assert_frozen_then_stop(*_args, **_kwargs):
            frozen = json.loads(self.manifest_path.read_text(encoding="utf-8"))
            self.assertEqual("frozen", frozen["provider_payload_state"])
            self.assertEqual(["src/app.py"], [entry["path"] for entry in frozen["provider_payload"]])
            raise ProviderReached

        with mock.patch.object(MODULE, "ASTParser", FakeParser), mock.patch.object(
            MODULE, "GraphBuilder", FakeGraph
        ), mock.patch.object(MODULE, "get_provider", side_effect=assert_frozen_then_stop):
            with self.assertRaises(ProviderReached):
                asyncio.run(
                    MODULE.generate_docs(
                        self.repo, 0.02, 1, "mock", "", "auto", self.manifest_path
                    )
                )

    def test_installed_file_traverser_filters_mixed_scope_without_provider(self) -> None:
        real_python = Path(os.environ.get("APPDATA", "")) / "uv" / "tools" / "repowise" / "Scripts" / "python.exe"
        if not real_python.is_file():
            self.skipTest("installed Repowise runtime is unavailable")

        (self.repo / "src" / "ignored.py").write_text("IGNORED = True\n", encoding="utf-8")
        (self.repo / "src" / "opaque.bin").write_bytes(b"\x00opaque")
        (self.repo / "src" / "oversized.py").write_bytes(b"#" * (501 * 1024))
        (self.repo / ".gitignore").write_text("src/ignored.py\n", encoding="utf-8")
        command = (
            "import json,sys; from pathlib import Path; "
            "from repowise.core.ingestion import FileTraverser; "
            "root=Path(sys.argv[1]); "
            "print(json.dumps(sorted(f.path for f in FileTraverser(root, "
            "extra_exclude_patterns=['.repowise/**']).traverse())))"
        )
        output = subprocess.check_output(
            [str(real_python), "-c", command, str(self.repo)], text=True, stderr=subprocess.STDOUT
        )
        self.assertEqual(["src/app.py"], json.loads(output.strip().splitlines()[-1]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
