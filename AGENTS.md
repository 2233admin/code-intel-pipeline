# Code Intel Pipeline agent rules

## Before starting any work

- Read `docs/decisions/README.md` (30 seconds). Live semantics decisions — pin
  semantics, discovery precedence, gate scope — live there, not in any one
  session's chat. Work that contradicts an active decision record must open an
  issue challenging the record first, not implement the opposite semantics.
- Claim your issue before writing code: add the `claimed` label and a comment
  naming your branch and one-line approach (DR-0004). An issue already claimed
  means join that branch or pick another issue — never a parallel rewrite.
- Count open fix PRs (`gh pr list --state open`). At 5 or more, this session's
  output must reduce that queue — merge, rebase, or close — not add features
  (DR-0005). Adopted cures outrank new cures.
- A fix for an install-class bug ("user cannot install / installed binary
  fails") is not complete until its reproduction joins the install-smoke CI
  gate in the same PR (DR-0001). 3794 checkout-topology tests missed every
  installed-topology bug that shipped with v0.7.0; only the topology gate
  catches this class.

## Language direction

- Do not add new PowerShell scripts or new product behavior to existing `.ps1` files.
- Treat `.ps1` files as retired compatibility surfaces. New production paths must not invoke
  PowerShell; migrate the capability to Rust and keep only a tested forwarding shim when external
  compatibility still requires it.
- Implement production CLI, orchestration, artifact, policy, and provider-boundary work in Rust.
- MoonBit is an approved experimental language for small, isolated components. Keep experiments
  outside the production path until they prove artifact-contract parity, cross-platform builds,
  tests, and a measured advantage over the Rust implementation.
- Retire a compatibility entry point only after its Rust replacement passes the existing contract
  tests and release packaging checks; do not delete historical rollback fixtures in the same change.

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
- Run `code-intel lint hardcoded-paths` before pushing (issue #275: the Rust
  implementation replaced `legacy/tools/check-hardcoded-paths.ps1`). It scans tracked
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
  move, and a pin can chain: editing `legacy/run-code-intel.ps1` to update a digest it
  quotes changes `legacy/run-code-intel.ps1`'s own pinned digest too.
- MoonBit experiments require `moon test` and parity fixtures against the current artifact contract before promotion.
- New documentation and command examples must lead with the compiled `code-intel` CLI. Do not
  document PowerShell as an operational prerequisite; mention it only when identifying a retained
  compatibility fixture or migration boundary.

## While writing code

Use the compiled Rust gate `code-intel sentrux gate "<scope-path>"` before editing and after the
focused tests. Mid-edit, query `code-intel change impact --changed <paths> --staleness advisory`
for impacted files and test candidates — advisory answers come from the last committed run and
never gate. Preview mechanical rewrites with `capability exec edit.ast-grep-plan` (preview-only,
`repositoryMutation=false`) before applying them. Apply a known-span change with `code-intel edit
apply --repo-path <checkout> --file <path> --span <startLine:startColumn-endLine:endColumn>
--expect-sha256 <sha256 of the span's current bytes> --replacement <text>` instead of rewriting
the surrounding line.

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues (`gh` CLI); repo issues are the delivery SSOT (DR-0007). See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles, label string = role name. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.
