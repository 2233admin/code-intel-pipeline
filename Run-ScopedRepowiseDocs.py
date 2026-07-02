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
    parser.add_argument("--provider", default=os.environ.get("REPOWISE_PROVIDER", "anthropic"))
    parser.add_argument("--model", default=os.environ.get("REPOWISE_MODEL", ""))
    parser.add_argument("--reasoning", default=os.environ.get("REPOWISE_REASONING", "auto"))
    return parser.parse_args()


async def generate_docs(repo_path: Path, coverage_pct: float, concurrency: int, provider_name: str, model: str, reasoning: str) -> dict[str, object]:
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
    if provider_name == "ccw":
        provider_name = "codex_cli"
    provider_kwargs = {"reasoning": reasoning, "with_rate_limiter": False}
    if model:
        provider_kwargs["model"] = model
    if provider_name == "anthropic":
        provider_kwargs["api_key"] = os.environ["ANTHROPIC_API_KEY"]
        provider_kwargs["base_url"] = os.environ.get("ANTHROPIC_BASE_URL")
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
    result = asyncio.run(generate_docs(Path(args.repo).resolve(), args.coverage_pct, args.concurrency, args.provider, args.model, args.reasoning))
    print(json.dumps(result, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
