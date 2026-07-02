# repowise local patches

## anthropic-thinking-blocks (2026-07-02)

**Problem:** reasoning models served through Anthropic-compatible endpoints
(e.g. MiniMax-M2.7 / MiniMax-M3 at `https://api.minimaxi.com/anthropic`)
return a `ThinkingBlock` as `response.content[0]`. repowise
`core/providers/llm/anthropic.py` reads `response.content[0].text`, so every
page generation fails with `'ThinkingBlock' object has no attribute 'text'`
and docs silently end at 0 pages.

**Patch (applied to the installed uv tool venv,
`%APPDATA%\uv\tools\repowise\Lib\site-packages\repowise\core\providers\llm\anthropic.py`):**

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

**⚠️ Re-apply after any `uv tool upgrade repowise`** — the patch lives in the
installed venv, not the repo. Symptom of it being lost: docs runs exit 0 with
`total_pages=0` and `page_generation_failed error='ThinkingBlock' object has
no attribute 'text'` in the log. Verify with:

```powershell
rg "ThinkingBlock|getattr\(block" "$env:APPDATA\uv\tools\repowise\Lib\site-packages\repowise\core\providers\llm\anthropic.py"
```

Long-term fix belongs upstream (repowise should join text blocks / skip
thinking blocks for all Anthropic-compatible providers).
