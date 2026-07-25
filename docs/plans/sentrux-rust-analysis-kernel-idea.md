# Sentrux Rust Analysis Kernel — Idea Record

## Outcome

Move the performance-sensitive DSM analysis path from `Invoke-SentruxAgentTool.ps1` into the
`code-intel` Rust CLI without changing the artifact contract consumed by `run-code-intel.ps1`.

## First vertical slice

- Add `code-intel sentrux dsm <path>` as a native Rust operation.
- Port governed-source inventory, file/function metrics, stable IDs, module aggregation,
  dependency edges, derived risk, and the nine DSM color modes.
- Preserve the existing root JSON fields and nested field names, including `file_details`.
- Prefer the Rust command in production pipeline execution.
- Keep the PowerShell DSM implementation only as an explicit compatibility fallback when the
  Rust executable is unavailable or fails.

## Non-goals for this slice

- Reimplement Sentrux `check`, `gate`, or parser/plugin ownership.
- Port `evolution`, `what_if`, or session persistence yet; they will consume the shared Rust
  snapshot in later slices.
- Add a new runtime dependency or change admission policy.

## Acceptance

1. Fixture tests lock inventory exclusions, stable IDs, function metrics, module metrics, and JSON
   shape.
2. The production pipeline records Rust as the DSM provider on a successful native invocation and
   records an explicit fallback note otherwise.
3. Rust tests, targeted PowerShell tests, and a fresh normal pipeline pass.
4. Native DSM is measurably faster than the prior PowerShell DSM on this repository.

## Rollback

Set `CODE_INTEL_SENTRUX_DSM_PROVIDER=powershell` to select the compatibility path while retaining
the same downstream artifact contract.
