# Code Intel Pipeline agent rules

## Language direction

- Do not add new PowerShell scripts or new product behavior to existing `.ps1` files.
- Treat current PowerShell entry points as legacy compatibility surfaces only. Limit edits to critical fixes or thin forwarding shims while their Rust replacements are being delivered.
- Implement production CLI, orchestration, artifact, policy, and provider-boundary work in Rust by default.
- MoonBit is an approved experimental language for small, isolated components. Keep experiments outside the production path until they prove artifact-contract parity, cross-platform builds, tests, and a measured advantage over the Rust implementation.
- Do not perform a big-bang PowerShell deletion. Retire each compatibility entry point only after its Rust or promoted MoonBit replacement passes the existing contract tests and release packaging checks.

## Reading the crate's warning count

`cargo check` on `code-intel` reports ~100 dead-code warnings on a clean tree.
Almost none of them are removable dead code, and the count is not a debt metric.

The crate has no `lib.rs`. Modules are shared by `#[path = "..."] mod x;`
re-inclusion, so one source file is compiled once per including module. Each
instantiation uses only the slice of the file it needs and warns that the rest
is unused — `hardened_git.rs` even keys a test temp directory on `module_path!()`
because its tests run once per instantiation. Deleting an item that one
instantiation does not use will break the instantiation that does.

Consequences:

- Do not add `-D warnings` to CI. It would fail on a clean tree.
- Do not treat the count as a cleanup backlog, and do not "fix" it in bulk.
- Before removing anything the compiler calls dead, grep the whole `src/` tree
  for it. Genuine dead code here looks like a *duplicate*: an item that also
  exists, live, in the module that took over its job.

## Verification

- Rust changes require focused `cargo test` coverage plus the relevant integration-contract checks.
- Run `tools/check-hardcoded-paths.ps1` before pushing. It scans tracked
  `.ps1`/`.psm1`/`.md`/`.yml` for machine-specific paths, and it strips
  `$env:VAR` references before matching. So naming one of the scanned Windows
  user-directory variables bare — in **prose or a comment**, not just in code —
  fails the scan, while the `$env:`-prefixed form passes. Always write the
  `$env:` form, including in comments and documentation.
- Editing a file pinned by `orchestration/**/*.json` (`toolchainDigests`, or a
  `{path, sha256}` pair) makes its contract test fail with a stale-digest
  assertion. Re-sync those pins **once, after every other edit**, by literal
  string replacement — never by reserializing the JSON. Several of those files
  are historical attestations whose formatting and unrelated digests must not
  move, and a pin can chain: editing `run-code-intel.ps1` to update a digest it
  quotes changes `run-code-intel.ps1`'s own pinned digest too.
- MoonBit experiments require `moon test` and parity fixtures against the current artifact contract before promotion.
- New documentation and command examples should lead with the compiled `code-intel` CLI. Mention PowerShell only when documenting an existing compatibility path.

## While writing code

Wrap every coding session in the pipeline gate: `Invoke-SentruxAgentTool.ps1 session_start` before the first edit, `session_end` after the last (`session_end` fails on structural regression). Mid-edit, query `code-intel change impact --changed <paths> --staleness advisory` for impacted files and test candidates — advisory answers come from the last committed run and never gate. Preview mechanical rewrites with `capability exec edit.ast-grep-plan` (preview-only, `repositoryMutation=false`) before applying them.
