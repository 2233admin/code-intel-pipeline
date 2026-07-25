# Code Intel Pipeline agent rules

## Language direction

- Do not add new PowerShell scripts or new product behavior to existing `.ps1` files.
- Treat current PowerShell entry points as legacy compatibility surfaces only. Limit edits to critical fixes or thin forwarding shims while their Rust replacements are being delivered.
- Implement production CLI, orchestration, artifact, policy, and provider-boundary work in Rust by default.
- MoonBit is an approved experimental language for small, isolated components. Keep experiments outside the production path until they prove artifact-contract parity, cross-platform builds, tests, and a measured advantage over the Rust implementation.
- Do not perform a big-bang PowerShell deletion. Retire each compatibility entry point only after its Rust or promoted MoonBit replacement passes the existing contract tests and release packaging checks.

## Verification

- Rust changes require focused `cargo test` coverage plus the relevant integration-contract checks.
- MoonBit experiments require `moon test` and parity fixtures against the current artifact contract before promotion.
- New documentation and command examples should lead with the compiled `code-intel` CLI. Mention PowerShell only when documenting an existing compatibility path.
