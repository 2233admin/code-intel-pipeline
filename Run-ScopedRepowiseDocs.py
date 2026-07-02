from __future__ import annotations

import argparse
import asyncio
import json
import os
import subprocess
from pathlib import Path

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
    return parser.parse_args()


# Providers whose __init__ does not accept an api_key kwarg.
_KEYLESS_PROVIDERS = {"ollama", "codex_cli", "opencode", "mock"}

_DEFAULT_MODELS = {
    "anthropic": "MiniMax-M2.7",
}


def _env(name: str) -> str:
    return (os.environ.get(name) or "").strip()


def resolve_provider_settings() -> tuple[str, dict[str, object]]:
    """Resolve provider name + kwargs from CODE_INTEL_* env vars.

    Env contract:
        CODE_INTEL_PROVIDER  provider name from repowise registry (default: anthropic)
        CODE_INTEL_MODEL     model name (anthropic default: MiniMax-M2.7)
        CODE_INTEL_API_KEY   generic credential (skipped for keyless providers e.g. ollama)
        CODE_INTEL_BASE_URL  generic endpoint override

    Backward compat: for provider=anthropic, missing generic vars fall back to
    the process-scoped ANTHROPIC_API_KEY / ANTHROPIC_BASE_URL that the calling
    PowerShell wrapper injects from user-scoped CODE_INTEL_ANTHROPIC_*.
    """
    name = _env("CODE_INTEL_PROVIDER").lower() or "anthropic"
    model = _env("CODE_INTEL_MODEL") or _DEFAULT_MODELS.get(name, "")
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


async def generate_docs(repo_path: Path, coverage_pct: float, concurrency: int) -> dict[str, object]:
    traverser = FileTraverser(repo_path, extra_exclude_patterns=[".repowise/**"])
    file_infos = list(traverser.traverse())
    repo_structure = traverser.get_repo_structure()
    parser = ASTParser()
    graph_builder = GraphBuilder(repo_path)
    parsed_files = []
    source_map: dict[str, bytes] = {}

    for fi in file_infos:
        try:
            source = Path(fi.abs_path).read_bytes()
            parsed = parser.parse_file(fi, source)
            parsed_files.append(parsed)
            source_map[fi.path] = source
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

    provider_name, provider_kwargs = resolve_provider_settings()
    provider = get_provider(provider_name, **provider_kwargs)
    config = GenerationConfig(
        max_concurrency=concurrency,
        reasoning="auto",
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
    result = asyncio.run(generate_docs(Path(args.repo).resolve(), args.coverage_pct, args.concurrency))
    print(json.dumps(result, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
