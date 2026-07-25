# Session evidence adapter

> Lifecycle: optional research adapter; implemented and tested, but not default or production.

`provider session-adapt` is an optional session-review workflow. It consumes Mindwalk trace v1 and
normalizes the parts Code Intel needs without invoking Mindwalk at runtime: event order, coarse tool
family, action, error state, and repository-relative targets.

Code Intel owns the production boundary:

- repository snapshot binding;
- safe repository-relative path normalization;
- removal of prompts, user-message marks, raw summaries, absolute paths, and outside-path values;
- `exact`, `estimated`, and `unavailable` observability grades;
- optional joins to `sentrux-hotspots.json` or raw Sentrux DSM `file_details`;
- advisory signals for structurally notable edits, related errors, and edits after the last observed
  verification event.

Mindwalk remains replaceable. Its parser, Go runtime, city map, server, and LLM analysis are not part
of the Code Intel core. The adapter is compatible with Mindwalk trace schema v1 at commit
`e208b6b8504138843f671e031f28129b66003a67`, licensed MIT. No upstream implementation source is
copied.

## Workflow

Generate the raw trace with Mindwalk when available, keeping it outside the repository and normal
artifact publication directory:

```powershell
mindwalk trace <codex-or-claude-session.jsonl> -o <private-trace.json>
```

Then normalize and enrich it with Code Intel:

```powershell
target/debug/code-intel.exe provider session-adapt `
  --repo <repo-path> `
  --trace <private-trace.json> `
  --hotspots <sentrux-hotspots.json> `
  --out <session-evidence.json>
```

`--hotspots` and `--out` are optional. Without `--hotspots`, structural state remains `unknown`.
Without `--out`, the normalized artifact is written to stdout. Output creation is fail-closed and
does not overwrite an existing file.

The default snapshot policy is `explicit_overlay`, which binds evidence to the files actually
present during a coding session. Use `--working-tree-policy head_only` only when the review is
intentionally about committed state.

To carry the normalized report into an authoritative run, opt in explicitly:

```powershell
target/debug/code-intel.exe run dag-coordinate `
  --repo <repo-path> `
  --out <run-staging-directory> `
  --session-evidence <session-evidence.json>
```

A09 validates the closed runtime contract, requires the report snapshot to equal the run snapshot,
admits it through A03, and includes it in A07 atomic publication. Omitting the flag leaves the
default DAG unchanged. A stale session report fails closed instead of being silently attached to a
newer repository state.

## Authority and invocation

The artifact schema is `code-intel-session-evidence.v1`. It is advisory tool evidence and has no
gate, diagnosis, discharge, or Engineering Fact authority. It is not invoked by normal repository
scans. Call it explicitly for session review; there is no policy-triggered invocation until real
usage establishes thresholds.

Current structural-attention defaults are intentionally advisory: maximum complexity at least 20,
Git churn at least 5, or a dirty Sentrux file record. These are review routing hints, not quality
gate thresholds.

Promotion remains blocked on a privacy-safe representative real-session corpus whose raw evidence
can be independently reproduced, plus hostile-trace and latency/maintenance measurements. Until
then, passing unit/integration tests proves the boundary behavior only; it does not justify default
invocation or a production lifecycle label.
