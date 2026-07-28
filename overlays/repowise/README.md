# repowise local patches

## anthropic-thinking-blocks (2026-07-02)

**Problem:** reasoning models served through Anthropic-compatible endpoints
(e.g. MiniMax-M2.7 / MiniMax-M3 at `https://api.minimaxi.com/anthropic`)
return a `ThinkingBlock` as `response.content[0]`. repowise
`core/providers/llm/anthropic.py` reads `response.content[0].text`, so every
page generation fails with `'ThinkingBlock' object has no attribute 'text'`
and docs silently end at 0 pages.

**Patch (applied to the installed uv tool venv under
`<uv-tool-root>\Lib\site-packages\repowise\core\providers\llm\anthropic.py`):**

```python
# before
content=response.content[0].text,

# after
content="".join(
    block.text
    for block in response.content
    if getattr(block, "type", "") == "text"
),
```

**Self-healing:** `install-code-intel-pipeline.ps1` applies this patch
idempotently on every run (`Repair-RepowiseThinkingBlockPatch`, reported as
`repowise-thinking-patch` in the INSTALL output). After any
`uv tool upgrade repowise`, just re-run the installer. Symptom of the patch
being lost: docs runs exit 0 with `total_pages=0` and
`page_generation_failed error='ThinkingBlock' object has no attribute 'text'`
in the log. Verify with:

```powershell
rg "ThinkingBlock|getattr\(block|for block in response" "$env:APPDATA\uv\tools\repowise\Lib\site-packages\repowise\core\providers\llm\anthropic.py"
```

### Status: superseded upstream as of repowise 0.32.0 (verified 2026-07-29)

Upstream now walks the block list itself, so a stock install is already safe:

```python
text_content = ""
for block in response.content:
    if hasattr(block, "text"):
        text_content = block.text
        break
```

A `ThinkingBlock` has `.thinking`, not `.text`, so it is skipped. The installer
therefore reports four states, not three:

| Status | Meaning |
|---|---|
| `already_present` | our overlay is applied |
| `installed` | the vulnerable `response.content[0].text` was found and rewritten |
| `not_needed` | upstream carries its own fix; the overlay is obsolete here |
| `install_failed` | neither shape matched — upstream layout changed again, investigate |

Before this was distinguished, a fully healthy machine reported `install_failed`
on every install run, which trained us to ignore a real failure signal.

**Retirement:** delete `Repair-RepowiseThinkingBlockPatch` and this file once
every supported machine runs repowise >= 0.32.0. Kept for now because the
overlay is the only thing standing between an older pinned install and a silent
`total_pages=0`.
