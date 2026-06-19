# code-nexus-lite

**CodeNexus Lite** — Rust binary that runs as an **iii worker** (Worker / Function / Trigger).
Wraps **Repowise** + **Sentrux** for cheap, Agent-friendly code-understanding context.

Cross-platform replacement for the Windows-only `Invoke-CodeNexusLite.ps1`.

## Why

The PS1 Lite entry point is Windows-only. The Rust binary re-implements the same
shape as an iii Rust worker, so the same Agent can call it from Windows / Mac / Linux.

## Functions (3)

| Function ID | Input | Purpose |
|-------------|-------|---------|
| `codenexus::scan` | `{ repo, skip_init_if_cached? }` | Run Repowise on a repo, write `.repowise/wiki.db` |
| `codenexus::lite` | `{ repo, max_files?, max_references_per_file? }` | Read `.repowise/wiki.db`, return compact Agent context |
| `codenexus::doctor` | `{ }` | Check Repowise / Sentrux / rg availability, return JSON |

## HTTP triggers (3)

| Endpoint | Method | Bound function |
|----------|--------|----------------|
| `/scan` | POST | `codenexus::scan` |
| `/lite` | POST | `codenexus::lite` |
| `/doctor` | POST | `codenexus::doctor` |

## Build

```bash
cargo build --release
# binary: target/release/code-nexus-lite(.exe)
```

Output: ~5.2 MB stripped + LTO.

## Run

```bash
# 1. One-time — scaffold an iii project (creates myapp/ with .iii config)
iii project init myapp
cd myapp
iii                       # start the engine (default ws://127.0.0.1:49134)

# 2. In another shell — start this worker
cd "$CODE_INTEL_HOME/crates/code-nexus-lite"
./target/release/code-nexus-lite

# 3. In a third shell — call functions
curl -X POST http://127.0.0.1:49134/scan \
  -H 'Content-Type: application/json' \
  -d '{"repo": "<repo-path>", "skip_init_if_cached": false}'

curl -X POST http://127.0.0.1:49134/lite \
  -H 'Content-Type: application/json' \
  -d '{"repo": "<repo-path>"}'

curl -X POST http://127.0.0.1:49134/doctor \
  -H 'Content-Type: application/json' \
  -d '{}'
```

## Dependencies (external, on PATH)

- `repowise` v0.10.0+ (Python, `pip install repowise`)
- `sqlite3` CLI (Linux: `apt install sqlite3`; Windows: built-in)
- `rg` (ripgrep) — used by Sentrux, not by Lite directly
- `iii` engine running on `ws://127.0.0.1:49134`

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                    iii Engine :49134                        │
└──────────────┬─────────────────────────────────────────────┘
               │ WebSocket
               ▼
┌────────────────────────────────────────────────────────────┐
│  code-nexus-lite (Rust, 5.2 MB)                             │
│                                                             │
│  ┌─────────────────────┐  ┌─────────────────────┐         │
│  │ codenexus::scan     │  │ repowise init/augment│ ←──┐   │
│  │ codenexus::lite     │  │ sqlite3 wiki.db read  │    │   │
│  │ codenexus::doctor   │  │                       │    │   │
│  └─────────────────────┘  └─────────────────────┘    │   │
│            │                                            │   │
│            ▼                                            │   │
│  ┌─────────────────────────────────────────────────┐   │   │
│  │  HTTP triggers: POST /scan /lite /doctor         │   │   │
│  └─────────────────────────────────────────────────┘   │   │
└────────────────────────────────────────────────────────────┘   │
                                                                │
        ┌───────────────────────────────────────────────────────┘
        ▼
┌──────────────────┐
│   Repowise CLI   │  (Python, 0.10.0)
│   .repowise/     │
│   wiki.db        │
└──────────────────┘
```

## Comparison with PS1

| Aspect | `Invoke-CodeNexusLite.ps1` (PS1) | `code-nexus-lite` (Rust + iii) |
|--------|-----------------------------------|-------------------------------|
| OS | Windows-only | Win / Mac / Linux |
| Binary size | PS1 script (no binary) | 5.2 MB stripped |
| Engine | none (just PowerShell) | iii (Worker / Function / Trigger) |
| HTTP API | none | built-in (POST /scan/lite/doctor) |
| A2A protocol | n/a | via iii triggers |
| Cross-language interop | none | iii (TS / Python / Rust) |
| Discovery | none | iii workers.iii.dev catalog |
| Tracing | none | OpenTelemetry built-in |
| Status | v0.1.0 (Windows PS1) | v0.1.0 (Rust + iii) |

## License

Apache-2.0 (matches iii SDK).
