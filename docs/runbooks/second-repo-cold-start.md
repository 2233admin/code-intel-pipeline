# Second-repo cold start — tdxcli-rs

Derived view of `second-repo-cold-start.json` (source of truth — regenerate this file
by hand-mirroring the JSON if it changes). Friction detail: `frictions.md`.

**Target**: `D:/projects/tdxcli-rs` @ `18d777f7` (dirty: 7 modified, 2 untracked —
byte-identical before and after this task, zero writes made).
**Pipeline**: t4-secondrepo worktree @ `0255735`, built from source
(`cargo build -p code-intel --release --locked`), invoked by full binary path
(never installed to PATH).

## Headline

- 13 steps, **181s** total wall-clock.
- **8** worked exactly as documented/specified · **5** needed adaptation · **2**
  hard-failed outright (both later fixed by a later step).
- **5** friction items: F1-F5 (frictions.md). Top: F2 (real bug), F1 and F3 (missing
  docs).

## Steps

| # | step | s | exit | adapted |
|---|------|---|------|---------|
| 01 | build (cargo build --release) | 82 | 0 | no |
| 02 | `--help` | 0 | 0 | no |
| 03 | `--help --all` | 0 | 0 | no |
| 04 | `doctor --json` | 0 | 0 | no |
| 05 | `code-intel D:/projects/tdxcli-rs` (documented quick-start) | 12 | 0 | no |
| 06 | `run execute --repo tdxcli-rs ...` (CI-mirrored) | 13 | 0 | no |
| 07 | `artifact query --repo second-repo-scan` | 0 | **65** | yes |
| 08 | `artifact query --repo tdxcli-rs` | 0 | **65** | yes |
| 09 | `run execute` retry, nested `--authority-root` | 37 | 0 | yes |
| 10 | `artifact query` retry (succeeds) | 0 | 0 | yes |
| 11 | `snapshot identity --repo tdxcli-rs` | 1 | 0 | no |
| 12 | `change risk HEAD~5..HEAD` (cwd=target repo) | 2 | 0 | yes |
| 13 | `run execute`, ambient `CODE_INTEL_HOME` probe | 34 | 0 | no |

## Findings, one line each

- **F1** (missing doc): `run execute` / `artifact query` / `change risk` /
  `run dag-coordinate` appear nowhere in SKILL.md or README — only in
  `--help --all` / CI YAML / source.
- **F2** (real bug): `run execute --authority-root X --final-name Y` commits flat
  (`X/Y/`) with exit 0; `artifact query` needs `X/<repo>/<run>/` and can never find
  it. No warning either way.
- **F3** (missing doc): README's per-node file tree (`hospital.md`, etc.) doesn't
  exist on disk — real output is a flat `objects/sha256/<hash>` blob store on every
  run style tested.
- **F4** (hardcoded assumption): `change risk` has no `--repo` flag; silently scores
  whatever repo the CWD is in.
- **F5** (hardcoded assumption, unconfirmed): ambient `CODE_INTEL_HOME` on this
  machine points at the main checkout, not any worktree; no failure reproduced this
  session, but the precondition is real.

Full detail, exact error text, and proposed issue titles: `frictions.md`.
