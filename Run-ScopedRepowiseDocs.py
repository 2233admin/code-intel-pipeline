from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import subprocess
import uuid
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath

from repowise.cli.helpers import get_db_url_for_repo, save_config, save_state
from repowise.core.generation import ContextAssembler, GenerationConfig, PageGenerator
from repowise.core.ingestion import ASTParser, FileTraverser, GraphBuilder
from repowise.core.persistence import (
    FullTextSearch,
    create_engine,
    create_session_factory,
    get_session,
    init_db,
    upsert_page_from_generated,
    upsert_repository,
)
from repowise.core.providers.llm.registry import get_provider


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--coverage-pct", type=float, default=0.02)
    parser.add_argument("--concurrency", type=int, default=1)
    parser.add_argument("--provider", default=os.environ.get("REPOWISE_PROVIDER", "anthropic"))
    parser.add_argument("--model", default=os.environ.get("REPOWISE_MODEL", ""))
    parser.add_argument("--reasoning", default=os.environ.get("REPOWISE_REASONING", "auto"))
    parser.add_argument("--egress-manifest", required=True)
    return parser.parse_args()


# Providers whose __init__ does not accept an api_key kwarg.
_KEYLESS_PROVIDERS = {"ollama", "codex_cli", "opencode", "mock"}

_DEFAULT_MODELS = {
    "anthropic": "MiniMax-M2.7",
}


def _env(name: str) -> str:
    return (os.environ.get(name) or "").strip()


def resolve_provider_settings(default_provider: str = "", default_model: str = "") -> tuple[str, dict[str, object]]:
    """Resolve provider name + kwargs from CODE_INTEL_* env vars.

    Env contract:
        CODE_INTEL_PROVIDER  provider name from repowise registry (default: anthropic)
        CODE_INTEL_MODEL     model name (anthropic default: MiniMax-M2.7)
        CODE_INTEL_API_KEY   generic credential (skipped for keyless providers e.g. ollama)
        CODE_INTEL_BASE_URL  generic endpoint override

    Backward compat: for provider=anthropic, missing generic vars fall back to
    the process-scoped ANTHROPIC_API_KEY / ANTHROPIC_BASE_URL that the calling
    PowerShell wrapper injects from user-scoped CODE_INTEL_ANTHROPIC_*.

    default_provider/default_model come from --provider/--model CLI args
    (REPOWISE_PROVIDER/REPOWISE_MODEL env), used only when the CODE_INTEL_*
    vars are unset -- CODE_INTEL_* takes priority since it is what the
    PowerShell wrapper actively manages for this pipeline.
    """
    name = _env("CODE_INTEL_PROVIDER").lower() or (default_provider or "").lower() or "anthropic"
    if name == "ccw":
        name = "codex_cli"
    model = _env("CODE_INTEL_MODEL") or default_model or _DEFAULT_MODELS.get(name, "")
    api_key = _env("CODE_INTEL_API_KEY")
    base_url = _env("CODE_INTEL_BASE_URL")

    if name == "anthropic":
        api_key = api_key or _env("CODE_INTEL_ANTHROPIC_API_KEY") or _env("ANTHROPIC_API_KEY")
        base_url = base_url or _env("CODE_INTEL_ANTHROPIC_BASE_URL") or _env("ANTHROPIC_BASE_URL")

    kwargs: dict[str, object] = {"with_rate_limiter": False}
    if model:
        kwargs["model"] = model
    if base_url:
        kwargs["base_url"] = base_url
    if api_key and name not in _KEYLESS_PROVIDERS:
        kwargs["api_key"] = api_key
    return name, kwargs


def _validate_file_entries(
    repo_path: Path,
    entries: object,
    policy: str,
    label: str,
) -> dict[str, str]:
    if not isinstance(entries, list):
        raise RuntimeError(f"egress manifest {label} list is missing")

    seen: dict[str, str] = {}
    ordered_paths: list[str] = []
    for entry in entries:
        if not isinstance(entry, dict):
            raise RuntimeError(f"egress manifest contains an invalid {label} entry")
        relative = entry.get("path")
        expected_hash = entry.get("sha256")
        if not isinstance(relative, str) or not isinstance(expected_hash, str):
            raise RuntimeError(f"egress manifest {label} entry is incomplete")
        if len(expected_hash) != 64 or any(char not in "0123456789abcdefABCDEF" for char in expected_hash):
            raise RuntimeError(f"egress manifest contains an invalid SHA-256: {relative}")
        posix_path = PurePosixPath(relative)
        if posix_path.is_absolute() or ".." in posix_path.parts or relative in {"", "."}:
            raise RuntimeError(f"egress manifest contains unsafe path: {relative}")
        try:
            candidate = (repo_path / Path(*posix_path.parts)).resolve(strict=True)
        except OSError as exc:
            raise RuntimeError(f"egress manifest path is unavailable: {relative}") from exc
        if not candidate.is_relative_to(repo_path) or not candidate.is_file():
            raise RuntimeError(f"egress manifest path escapes the scoped repository: {relative}")
        actual_hash = hashlib.sha256(candidate.read_bytes()).hexdigest()
        if actual_hash != expected_hash.lower():
            raise RuntimeError(f"egress manifest hash mismatch: {relative}")
        if relative in seen:
            raise RuntimeError(f"egress manifest contains duplicate path: {relative}")
        seen[relative] = expected_hash.lower()
        ordered_paths.append(relative)

        if policy == "head-tracked-only":
            try:
                head_blob_oid = subprocess.check_output(
                    ["git", "-C", str(repo_path), "rev-parse", f"HEAD:{relative}"],
                    text=True,
                    stderr=subprocess.STDOUT,
                ).strip()
                working_blob_oid = subprocess.check_output(
                    [
                        "git",
                        "-C",
                        str(repo_path),
                        "hash-object",
                        f"--path={relative}",
                        str(candidate),
                    ],
                    text=True,
                    stderr=subprocess.STDOUT,
                ).strip()
            except (OSError, subprocess.CalledProcessError) as exc:
                raise RuntimeError(f"egress manifest file is not tracked at HEAD: {relative}") from exc
            if working_blob_oid != head_blob_oid:
                raise RuntimeError(f"egress manifest file does not match HEAD blob: {relative}")

    if ordered_paths != sorted(ordered_paths):
        raise RuntimeError(f"egress manifest {label} list must be sorted by path")
    return seen


def validate_egress_manifest(
    repo_path: Path,
    manifest_path: Path,
    provider_name: str,
    *,
    require_frozen: bool = False,
) -> dict[str, object]:
    """Fail closed before provider construction if egress evidence is absent or stale."""
    repo_path = repo_path.resolve(strict=True)
    manifest_path = manifest_path.resolve(strict=True)
    expected_manifest = (repo_path / ".repowise" / "egress-manifest.json").resolve(strict=True)
    if manifest_path != expected_manifest:
        raise RuntimeError("egress manifest must be .repowise/egress-manifest.json inside the scoped repository")

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"cannot read egress manifest: {exc}") from exc
    if not isinstance(manifest, dict):
        raise RuntimeError("egress manifest must be a JSON object")
    if type(manifest.get("schema_version")) is not int or manifest.get("schema_version") != 2:
        raise RuntimeError("egress manifest schema_version must be 2")

    try:
        head = subprocess.check_output(
            ["git", "-C", str(repo_path), "rev-parse", "HEAD"],
            text=True,
            stderr=subprocess.STDOUT,
        ).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise RuntimeError(f"cannot resolve scoped repository HEAD: {exc}") from exc
    if manifest.get("head") != head:
        raise RuntimeError("egress manifest HEAD does not match the scoped repository")
    if manifest.get("provider") != provider_name:
        raise RuntimeError("egress manifest provider does not match the requested provider")
    if manifest.get("working_tree_policy") not in {"head-tracked-only", "include-working-tree"}:
        raise RuntimeError("egress manifest has an invalid working-tree policy")

    scope = manifest.get("scope")
    if not isinstance(scope, dict) or not isinstance(scope.get("paths"), list) or not isinstance(scope.get("root_files"), list):
        raise RuntimeError("egress manifest scope is incomplete")
    policy = manifest["working_tree_policy"]
    inventory = _validate_file_entries(repo_path, manifest.get("scope_inventory"), policy, "scope inventory")
    payload = _validate_file_entries(repo_path, manifest.get("provider_payload"), policy, "provider payload")
    state = manifest.get("provider_payload_state")
    if state not in {"pending", "frozen"}:
        raise RuntimeError("egress manifest has an invalid provider payload state")
    if state == "pending" and payload:
        raise RuntimeError("pending egress manifest must not contain a provider payload")
    if require_frozen and state != "frozen":
        raise RuntimeError("egress manifest provider payload is not frozen")
    for relative, payload_hash in payload.items():
        if inventory.get(relative) != payload_hash:
            raise RuntimeError(f"provider payload is absent from scope inventory: {relative}")
    return manifest


def read_manifested_sources(
    repo_path: Path,
    file_infos: list[object],
    manifest: dict[str, object],
) -> dict[str, bytes]:
    """Read the exact provider inputs and recheck their manifest hashes.

    This second check closes the window between initial manifest validation and
    traversal. The bytes checked here are the same immutable bytes passed to
    Repowise generation; read or validation failures deliberately propagate.
    """
    repo_path = repo_path.resolve(strict=True)
    entries = manifest["scope_inventory"]
    if not isinstance(entries, list):
        raise RuntimeError("egress manifest scope inventory list is missing")
    expected_by_path = {
        str(entry["path"]): str(entry["sha256"]).lower()
        for entry in entries
        if isinstance(entry, dict)
    }

    source_map: dict[str, bytes] = {}
    traversed_paths: set[str] = set()
    for file_info in file_infos:
        traversed_path = str(getattr(file_info, "path")).replace("\\", "/")
        if traversed_path in traversed_paths:
            raise RuntimeError(f"provider input contains duplicate path: {traversed_path}")
        traversed_paths.add(traversed_path)
        expected_hash = expected_by_path.get(traversed_path)
        if expected_hash is None:
            raise RuntimeError(f"provider input contains file absent from egress manifest: {traversed_path}")

        posix_path = PurePosixPath(traversed_path)
        candidate = (repo_path / Path(*posix_path.parts)).resolve(strict=True)
        file_info_path = Path(str(getattr(file_info, "abs_path"))).resolve(strict=True)
        if candidate != file_info_path or not candidate.is_relative_to(repo_path) or not candidate.is_file():
            raise RuntimeError(f"provider input path does not match egress manifest: {traversed_path}")

        source = candidate.read_bytes()
        if hashlib.sha256(source).hexdigest() != expected_hash:
            raise RuntimeError(f"egress manifest hash mismatch while reading provider input: {traversed_path}")
        source_map[str(getattr(file_info, "path"))] = source

    return source_map


def freeze_provider_payload(
    repo_path: Path,
    manifest_path: Path,
    file_infos: list[object],
    manifest: dict[str, object],
) -> dict[str, bytes]:
    """Atomically freeze the exact, already-read bytes that generation may send."""
    source_map = read_manifested_sources(repo_path, file_infos, manifest)
    payload = [
        {"path": path.replace("\\", "/"), "sha256": hashlib.sha256(source).hexdigest()}
        for path, source in source_map.items()
    ]
    payload.sort(key=lambda entry: entry["path"])

    frozen = dict(manifest)
    frozen["provider_payload_state"] = "frozen"
    frozen["provider_payload_frozen_at_utc"] = datetime.now(timezone.utc).isoformat()
    frozen["provider_payload"] = payload

    manifest_path = manifest_path.resolve(strict=True)
    temp_path = manifest_path.with_name(f"{manifest_path.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temp_path.open("w", encoding="utf-8", newline="\n") as handle:
            json.dump(frozen, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp_path, manifest_path)
    finally:
        temp_path.unlink(missing_ok=True)

    provider_name = str(frozen["provider"])
    validated = validate_egress_manifest(repo_path, manifest_path, provider_name, require_frozen=True)
    validated_payload = {
        str(entry["path"]): str(entry["sha256"])
        for entry in validated["provider_payload"]  # type: ignore[union-attr]
    }
    actual_payload = {
        path.replace("\\", "/"): hashlib.sha256(source).hexdigest()
        for path, source in source_map.items()
    }
    if validated_payload != actual_payload:
        raise RuntimeError("frozen provider payload does not match the exact provider source bytes")
    return source_map


async def generate_docs(
    repo_path: Path,
    coverage_pct: float,
    concurrency: int,
    provider_name: str,
    model: str,
    reasoning: str,
    egress_manifest: Path,
) -> dict[str, object]:
    provider_name, provider_kwargs = resolve_provider_settings(provider_name, model)
    manifest = validate_egress_manifest(repo_path, egress_manifest, provider_name)

    traverser = FileTraverser(repo_path, extra_exclude_patterns=[".repowise/**"])
    file_infos = list(traverser.traverse())
    source_map = freeze_provider_payload(repo_path, egress_manifest, file_infos, manifest)
    repo_structure = traverser.get_repo_structure(file_infos)
    parser = ASTParser()
    graph_builder = GraphBuilder(repo_path)
    parsed_files = []
    for fi in file_infos:
        source = source_map[fi.path]
        try:
            parsed = parser.parse_file(fi, source)
            parsed_files.append(parsed)
            graph_builder.add_file(parsed)
        except Exception:
            continue

    graph_builder.build()
    try:
        from repowise.core.generation.editor_files.tech_stack import detect_tech_stack

        tech_items = detect_tech_stack(repo_path)
        graph_builder.add_framework_edges([item.name for item in tech_items])
    except Exception:
        pass

    # reasoning is a generation-time setting (GenerationConfig below); provider
    # __init__ signatures don't uniformly accept it (MockProvider rejects it).
    provider = get_provider(provider_name, **provider_kwargs)
    config = GenerationConfig(
        max_concurrency=concurrency,
        reasoning=reasoning,
        coverage_pct=coverage_pct,
        max_pages_pct=coverage_pct,
        enable_rag_context=False,
        enable_onboarding=True,
    )
    assembler = ContextAssembler(config)
    generator = PageGenerator(provider, assembler, config, language=config.language)
    pages = await generator.generate_all(
        parsed_files,
        source_map,
        graph_builder,
        repo_structure,
        repo_path.name,
    )

    engine = create_engine(get_db_url_for_repo(repo_path))
    await init_db(engine)
    session_factory = create_session_factory(engine)
    async with get_session(session_factory) as session:
        repo = await upsert_repository(session, name=repo_path.name, local_path=str(repo_path))
        for page in pages:
            await upsert_page_from_generated(session, page, repo.id)

    fts = FullTextSearch(engine)
    await fts.ensure_index()
    for page in pages:
        await fts.index(page.page_id, page.title, page.content)
    await engine.dispose()

    head = subprocess.check_output(["git", "-C", str(repo_path), "rev-parse", "HEAD"], text=True).strip()
    docs_enabled = len(pages) > 0
    state: dict[str, object] = {
        "last_sync_commit": head,
        "total_pages": len(pages),
        "docs_enabled": docs_enabled,
        "provider": provider.provider_name,
        "model": provider.model_name,
    }
    if not docs_enabled:
        state["docs_skip_reason"] = "no pages generated; likely provider quota or rate limit"
    save_config(repo_path, provider.provider_name, provider.model_name, "mock", reasoning="auto")
    save_state(repo_path, state)
    return {
        "pages": len(pages),
        "docs_enabled": docs_enabled,
    }


def main() -> int:
    args = parse_args()
    result = asyncio.run(
        generate_docs(
            Path(args.repo).resolve(),
            args.coverage_pct,
            args.concurrency,
            args.provider,
            args.model,
            args.reasoning,
            Path(args.egress_manifest),
        )
    )
    print(json.dumps(result, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
