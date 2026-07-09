# ADR 0001 -- Merge my-code-machine into code-intel-pipeline as a Rust-unified monorepo

- Status: Proposed (alignment phase, no code authorized)
- Date: 2026-06-17
- Deciders: Curry (owner), Claude (drafting)
- Supersedes: none

## Context

Two local tools have overlapping "operator infrastructure" identity but different scope:

- **my-code-machine (mcm)** -- host-machine env manager. Python package `mmc`:
  ~40 modules, pydantic schemas, detector plugin system (runtime / projects / ssh /
  claude_env / hooks / standards / npm / dotfiles / symlink_refs / scripts_scan / backups),
  three sync backends (gitea / github / gitlab), organizer / cleaner / checkpoint,
  full pytest suite (~20 test modules). Concern: *this machine* -- drives, Python/uv,
  archive/clean hygiene, drift vs SKILL.md.
- **code-intel-pipeline (CIP)** -- target-repo understanding pipeline. 15 PowerShell
  scripts + `crates/code-nexus-lite` (Rust) + 1 Python helper. Concern: *a cloned repo* --
  rg inventory, repowise semantic index, Understand Anything graph, Sentrux structure gate,
  hospital-mode triage state machine, surgery-plan generation, 4-bucket failure model.
  The .ps1 line count is small but the hospital / triage / sentruxInsight / surgery-plan
  logic is real business logic, not thin forwarding.

Three goals drove this decision, in priority order:

1. **Kill PowerShell (primary driver).** The motivation is NOT program runtime --
   it is *operator friction*. Owner: "every time PowerShell trips me up it kills
   efficiency." Every interaction with these tools currently routes through PowerShell
   quoting / nested escaping (and the `bash-unification` workaround rule). That daily
   tax is the real cost being paid. A single static binary invoked identically on every
   OS (`cip pipeline --repo X`) removes the wrapper and the quoting layer entirely.
2. **Linux + Windows compatibility.** Both tools are Windows-bound today (mcm via
   `py`/Windows paths, CIP via PowerShell). Target is Linux + Windows. **macOS is out of
   scope** -- the platform adapter only needs two targets, which keeps the drive/path model
   to a single either/or (Windows drive letters vs Linux mountpoint/XDG).
3. **Consolidate** mcm + CIP into one repo (owner request).

Reinforcing signal: mcm is BROKEN right now precisely because of Python-runtime-bootstrap
fragility (the uv trampoline lost its backing python after the install-dir migration). A
Python-unified merge would re-incur that whole class of pain on every fresh machine. A Rust
static binary deletes the class -- no runtime, no trampoline, no bootstrap.

## Decision

Merge mcm **into** code-intel-pipeline. The surviving repo is `2233admin/code-intel-pipeline`,
restructured as a **Rust-unified Cargo workspace**. All PowerShell is removed; all Python
(`mmc` + the one CIP helper) is rewritten to Rust. `crates/code-nexus-lite` is kept and
absorbed as a workspace member.

Target layout:

```
code-intel-pipeline/   (Cargo workspace)
  crates/
    cli/             clap entrypoint -- one binary, subcommands: pipeline + machine
    pipeline/        repo-understanding (was run-code-intel.ps1 + adapters + hospital)
    governance/      hospital state machine + triage + surgery-plan + sentruxInsight
    machine/         host-env mgmt (was mmc: detectors, archive, clean, doctor)
    sync/            git-backed sync backends (gitea/github/gitlab; was mmc/sync)
    core/            shared: serde schemas (replaces pydantic), platform adapter, fs utils
    code-nexus-lite/ kept as-is, workspace member
  docs/adr/
```

`machine` and `pipeline` stay **separate crates**, not fused into one command surface.
The category boundary (host-machine vs target-repo) is real and survives the merge --
one binary, two top-level subcommand trees.

### Honest rationale (this is NOT a runtime-speed decision)

The motivating phrase was "more efficient code," but the owner clarified the efficiency is
*operator* efficiency, not program runtime. The justification has to be precise about *what*
gets more efficient -- otherwise the ADR lies to its future readers:

- **Operator friction is the #1 win.** The cost being eliminated is human time lost to
  PowerShell quoting / nested escaping every time the tools are driven. One binary, one
  invocation form across Linux + Windows, no shell wrapper, no `bash-unification` detour.
- **Glue runtime is NOT the win.** CIP's orchestration spends its wall-clock waiting on
  external binaries (`rg`, `repowise`, `Understand Anything`, `sentrux`). Rewriting that
  glue Rust vs Python vs PowerShell changes the spawn overhead by milliseconds against
  multi-second tool runs. Near-zero runtime gain on the glue layer. Do not claim otherwise.
- **The other real wins of Rust-unified:**
  1. **Single static binary** -- no Python runtime, no PowerShell, no `uv` bootstrap on a
     fresh machine. `bootstrap-new-machine.ps1` (currently a script) collapses into "drop one
     binary." This also deletes the runtime-bootstrap fragility class that has mcm broken today.
  2. **One language** for the whole repo (code-nexus-lite is already Rust).
  3. **Strongest cross-platform story** -- `sysinfo`/`std::fs`/`camino` give portable drive,
     process, and path handling with no per-OS shell.
  4. **Compile-time correctness** over the hospital state machine + report schemas
     (serde enums beat both pydantic-at-runtime and PowerShell hashtables).

### Honest costs (the part that bites later)

- **mcm py->rs is the big rewrite, not the PS removal.** ~40 modules + ~20 pytest files.
  The PowerShell is the small job. Budget the merge as "rebuild mcm in Rust," not "delete some scripts."
- **pydantic + pytest investment is written off.** Schemas become serde structs; tests
  become `#[test]` / `insta` snapshots. The validation logic transfers; the framework muscle does not.
- **Rust glue is more verbose** for process spawning, JSON munging, and report templating
  than Python. Report generation (summary.md, hospital.md, surgery-plan.md) needs a
  templating crate (`askama` compile-time or `tera` runtime) -- more ceremony than f-strings.
- **Owner/Claude Python muscle memory** does not carry; iteration speed drops during the rewrite.
- **git operations** in `sync/` move from GitPython to `git2` (libgit2) or shelling `git`.
  `git2` is a heavier dependency but portable; decide per-backend.

## Consequences

- mcm-the-Python-package is retired once parity lands. Until then it stays runnable (see migration).
- `dev-env-standards.md` "Paired with my-code-machine (CLI: mcm.exe ...)" reference updates
  to point at the new binary's `machine` subcommand. The detector-crosswalk table stays valid
  (kinds are language-agnostic).
- The currently-BROKEN mcm trampoline (uv install-dir migration) becomes moot -- a static
  Rust binary has no trampoline. This is a real side-benefit: the merge fixes the breakage by deleting its cause.
- Multi-OS is achievable but is its own work item *inside* the Rust crates (platform adapter),
  not a free byproduct of the rewrite.

## Migration (incremental -- no big-bang, PS stays working until parity)

Per `test-before-bulk` discipline: port crate-by-crate behind a stable CLI, keep the old
path runnable, smoke each stage before deleting the predecessor.

1. **Phase 0 -- workspace scaffold.** Add Cargo workspace + `cli` + `core` crates around the
   existing `code-nexus-lite`. No behavior change. `cli` does nothing but route.
2. **Phase 1 -- core schemas.** Port mcm pydantic schemas (machine.py, snapshot.py, audit.py,
   skill_index.py) to serde structs in `core`. Golden-file test: serialize matches current mcm JSON output.
3. **Phase 2 -- machine crate.** Port detectors one at a time (runtime first -- it is already
   `shutil.which`-based and portable). Keep mcm Python runnable; diff outputs detector-by-detector.
4. **Phase 3 -- sync crate.** Port gitea/github/gitlab backends (git2 or shell git). Smoke against a throwaway repo.
5. **Phase 4 -- pipeline + governance crates.** Port run-code-intel orchestration, hospital
   state machine, sentruxInsight parsing, surgery-plan. This is the largest single phase;
   golden-file the report.json / hospital-report.json against current PowerShell output.
6. **Phase 5 -- cutover.** Delete all .ps1 + the mmc Python package. Update README,
   dev-env-standards.md, bootstrap docs. Single `cargo build --release` produces the binary.
7. **Phase 6 (separate work item) -- multi-OS.** Exercise the platform adapter on Linux/macOS;
   fix path/drive/process assumptions surfaced by CI on those targets.

Each phase is independently shippable; the repo never has a period where neither old nor new path works.

## Alternatives considered

- **Python-unified (glue Python + keep Rust core).** Lower total effort -- mcm merges in its
  own language, only CIP's PS -> Python. But: needs a Python runtime on every machine
  (no single binary), and the owner explicitly wants PS replaced with "more efficient code,"
  which read against the single-binary/distribution goal points at Rust. Rejected on owner's call,
  recorded here because it is the lower-cost path if priorities shift.
- **Go single-binary.** Single binary + easier glue than Rust, but adds a 4th language
  (Rust crate + Go orchestration), no existing Go in the stack/team, and mcm still gets rewritten. Rejected: language sprawl for no ecosystem payoff.
- **Status quo (two repos, two languages).** Rejected by the consolidation goal; also leaves
  the mcm trampoline breakage and the Windows-only constraint in place.

## Unresolved questions

1. **Report templating:** `askama` (compile-time, type-checked, no runtime template files)
   vs `tera` (runtime, Jinja-like, easier to hand-edit). Affects how summary.md / hospital.md / surgery-plan.md are emitted.
2. **git in sync/:** `git2`/libgit2 (portable, heavy build dep) vs shelling `git` (needs git on
   PATH, simpler). Could differ per backend.
3. **code-nexus-lite absorption:** keep its current module boundaries as-is, or refactor its
   `functions`/`triggers` into the new `pipeline`/`governance` split? Default: leave as-is in Phase 0, revisit in Phase 4.
4. **CLI surface:** single binary with `pipeline`/`machine` subcommand trees (decided), but do
   the legacy mcm verbs (onboard/list/show/audit/doctor/archive/clean) stay 1:1, or get renamed under `machine`?
5. **Cross-platform drive model:** mcm's drive-layout rules (C:/D:/E:/F:) are Windows-specific.
   Scope is now Linux + Windows only, so `core` needs exactly one either/or abstraction:
   Windows drive letters vs Linux mountpoint/XDG dirs. No macOS third case. Design of the
   enum + per-OS resolver is still TBD but bounded to two arms.
6. **RESOLVED -- execution model:** owner is not hand-writing the Rust (the PowerShell
   friction that motivated this is the same friction they want off their plate). The rewrite
   runs as an agent-driven background long-burn, orchestrated phase by phase by Claude; owner
   reviews at phase boundaries. Phase 2-5 can be parallelized across agents where golden-file
   contracts make them independent.
7. **RESOLVED -- multi-OS scope:** Linux + Windows only. macOS is explicitly out of scope.
   Phase 6 platform-adapter surface is bounded to those two targets.
