# Second-repo cold start — friction log

Target: `D:/projects/tdxcli-rs` (foreign repo, first-ever non-self-hosted run of this
pipeline). Numbered items here are cross-referenced from
`second-repo-cold-start.json` step `findings` arrays as F1..F5. I did not file GitHub
issues for these — proposed titles are listed for the requester to file after review.

## F1 — Full-chain commands are invisible outside `--help --all`

- **Symptom**: SKILL.md and README (including its "Agent 工作流" section — the story
  this task was told to follow) document only `code-intel <repo-path>`,
  `code-intel doctor`, `code-intel change impact`, `code-intel capability exec
  edit.ast-grep-plan`, `code-intel audit`, and `code-intel snapshot identity`. Neither
  file mentions `run execute`, `run dag-coordinate`, `artifact query`, or
  `change risk` — the exact commands this task's own spec required running.
- **Exact evidence**: grepping README.md and skills/code-intel-pipeline/SKILL.md for
  `run execute`, `change risk`, `artifact query` returns no matches. Default
  `code-intel --help` lists 6 commands; the 4 missing ones surface only under
  `code-intel --help --all` ("Advanced commands") or by reading
  `.github/workflows/ci.yml` / `release.yml` / `crates/code-intel-cli/src/main.rs`.
- **Classification**: missing doc.
- **Proposed issue title**: "docs: README/SKILL.md never mention run execute /
  artifact query / change risk — the CI-grade full chain is discoverable only via
  --help --all or source"

## F2 — `run execute` silently commits runs that `artifact query` can never find

- **Symptom**: `run execute --authority-root <dir> --final-name <name>` (the exact
  shape CI's own self-scan step uses) exits 0 and reports
  `"publication":{"status":"committed"}`, but `artifact query --artifact-root <dir>
  --repo <anything>` then fails, for every value of `--repo` guessed.
- **Exact error** (both guesses, only the repo name differs):
  `no committed authoritative run is indexed for repository: second-repo-scan`
  `no committed authoritative run is indexed for repository: tdxcli-rs`
- **Root cause** (read from source, `crates/code-intel-cli/src/committed_evidence.rs:30`):
  the index/query path expects `<artifact-root>/<repo-name>/<run-id>/`, a two-level
  layout. `run execute --authority-root X --final-name Y` publishes flat, one level
  (`X/Y/`), because `--authority-root` and the repo name are two independent,
  uncoordinated inputs — nothing forces or even warns the caller to pre-nest the repo
  name into `--authority-root` themselves.
- **Confirmed workaround** (invocation-only, no pipeline code touched): re-running
  with `--authority-root <root>/tdxcli-rs --final-name run-001` (repo name folded
  into the authority path by hand) made
  `artifact query --artifact-root <root> --repo tdxcli-rs` succeed immediately and
  return real matches.
- **Classification**: real bug. A command that reports "committed" with exit 0 while
  producing permanently unqueryable output is a functional defect, not an absent docs
  page — SKILL.md itself says "Do not describe a partial or domain-failed run as
  clean"; this is the query-side mirror of that same principle: a clean write that is
  actually a dead end.
- **Proposed issue title**: "bug: run execute --authority-root/--final-name accepts
  layouts that artifact query can never index, with no validation or warning"

## F3 — README's "输出在哪里" file tree does not exist on disk

- **Symptom**: README documents reading `diagnosis.hospital/hospital.md`,
  `doctor/doctor-observation.json`,
  `evidence.native-code/code-evidence/merged/agent/index.md`, etc. as literal paths
  under the run/authority directory. On both `code-intel D:/projects/tdxcli-rs`
  (documented quick-start) and `run execute` (CI-grade), the actual
  authority-root/artifact-root directory contains only `run-complete.json` plus a
  flat `objects/sha256/<hash>` content-addressed blob store — no directory named
  `diagnosis.hospital`, no file named `hospital.md`, anywhere on disk.
- **Exact evidence**: directory listing of `$TEMP/t4-auth/second-repo-scan/` after a
  successful `run execute` contains only `objects/sha256/*` (22 blobs) and
  `run-complete.json`. The strings `diagnosis.hospital/hospital.md` etc. exist only
  as logical `"path"` label fields inside the run manifest JSON (itself one more
  sha256 blob) — confirmed by reading the blob the manifest labels
  `diagnosis.hospital-view`: its bytes are in fact the hospital.md markdown
  (`# Code Intel Hospital Report ...`), just not stored at that path.
- **Classification**: missing doc — docs describe a layout that does not match
  shipped behavior for either run style exercised in this task.
- **Proposed issue title**: "docs: README '输出在哪里' file tree doesn't match the
  content-addressed objects/sha256 layout every real run produces"

## F4 — `change risk` has no `--repo` flag; every sibling command does

- **Symptom**: `code-intel --help --all`'s own usage line for `change risk` is
  `change risk <revspec> [--sample <N>] [--format json|text]` — no `--repo` /
  `--repo-path`. Every other subcommand touched in this task (`run execute --repo`,
  `snapshot identity --repo`, `artifact query --repo-path`, `audit --repo`) takes an
  explicit repo path.
- **Root cause** (source, `crates/code-intel-cli/src/change_risk/mod.rs:224` and
  `git.rs:19`): resolves the repo via `git::resolve_repo_root()`, which walks up from
  `std::env::current_dir()` — it always scores whatever git repo the CLI happens to
  be launched from, silently, with no flag to override it.
- **Adaptation**: ran
  `(cd D:/projects/tdxcli-rs && <pipeline-binary-abs-path> change risk HEAD~5..HEAD --format json)`
  — worked once CWD was moved into the target repo instead of passing a flag.
- **Classification**: hardcoded assumption (CWD-is-the-repo), inconsistent with the
  rest of the CLI's own `--repo` convention.
- **Proposed issue title**: "change risk has no --repo/--repo-path flag, unlike every
  sibling subcommand; silently scores whatever repo CWD is in"

## F5 — Ambient `CODE_INTEL_HOME` on this machine points at the main checkout, not any worktree (unconfirmed failure this session)

- **Symptom**: this machine's default shell environment carries
  `CODE_INTEL_HOME=D:\projects\code-intel-pipeline` (the primary checkout) at all
  times — not the t4 worktree this task's own step 1 explicitly required
  (`Set CODE_INTEL_HOME to the t4 worktree root`). At least 11 other
  `code-intel-pipeline` worktrees coexist on this machine (`git worktree list`), so
  this is a live, standing precondition for exactly the scenario
  `crates/code-intel-cli/src/providers.rs` has a dedicated guard for
  (`reject_foreign_checkout`: "orchestration manifest ambiguity ... These are
  different checkouts").
- **What was tested**: re-ran the step-06-shaped `run execute` command with
  `CODE_INTEL_HOME` left unset (ambient/main-checkout default) instead of pointed at
  the t4 worktree. Result: exit 0, no ambiguity error raised.
- **Why it likely didn't trigger**: `--manifest orchestration/integrations.json` was
  passed explicitly in every step this task ran. Per `run_cli.rs`, an explicit
  `--manifest` resolves as a plain CWD-relative path and never reaches the guarded
  `orchestration_manifest()` / `reject_foreign_checkout` resolver in `providers.rs` —
  that guard sits on a different code path (provider/doctor validation), not on
  `run execute`'s own `--manifest` argument handling. Whether that guarded path is
  ever reachable through `run execute` (e.g. via a provider validation node) was not
  isolated this session, and the main checkout and t4 worktree are presently
  near-identical, so no content divergence existed to expose even if the guard had
  fired.
- **Classification**: hardcoded assumption (environment-ambient-config-is-correct).
  Flagged with lower confidence than F1-F4 — no failure was actually reproduced this
  session — because the precondition is real and live, and this task's own "Why" section
  names "CODE_INTEL_HOME-style traps" by name as a thing a self-hosted-only pipeline
  cannot see.
- **Proposed issue title**: "investigate: can run execute's provider/doctor
  validation path ever hit the CODE_INTEL_HOME foreign-checkout guard, and does an
  unset/ambient CODE_INTEL_HOME reproduce it on a multi-worktree machine?"
