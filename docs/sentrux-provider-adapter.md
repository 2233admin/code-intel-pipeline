# Sentrux provider adapter

`provider sentrux-adapt` is the only structural-evidence ingress for Sentrux observations. In the default normal DAG, the built-in provider executes real `sentrux gate` and `sentrux check` commands, records their command-level outcomes, normalizes the six registered authoritative rule kinds, binds evidence to a repository snapshot, and submits the result to A04 before exposing diagnosis eligibility.

Unknown rule kinds, partial collections, command failures, and provider crashes remain `partial`/`unknown`; they never become passing diagnoses. `legacy/Invoke-SentruxAgentTool.ps1` and the bundled shim remain compatibility/rollback paths, not the normal authority route. The Rust boundary owns invocation, normalization, effects, and evidence contracts; Sentrux owns its scanning algorithms, while Hospital owns diagnosis policy.

The checked contracts are `code-intel-structural-evidence-port.v1` and `code-intel-sentrux-route-result.v1`. Successful routes emit structural observations and command evidence, not Engineering Facts; downstream diagnosis and fact promotion remain separate authority decisions.

## Stable-source disposition (#419)

The [upstream v0.5.7 release](https://github.com/sentrux/sentrux/releases/tag/v0.5.7) resolves to `f36da08a53e7f06c5b6aec33eb05816b371000ea`. Its `sentrux-core/src/metrics/root_causes.rs` and the existing formula pin `6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11` have the same Git blob `937336d97a21307337a97477643cbf24b8d714be` (15003 bytes). Retain the formula pin and DR-0011 semantics, including the source's per-factor `max(0.01)` floor; a stable tag does not justify a source downgrade or a baseline reset.

The separate V overlay still declares npm `tree-sitter-v` [1.0.7](https://registry.npmjs.org/tree-sitter-v/1.0.7), whose published `gitHead` is `43d06fda327dcd99fb073697389df59e0d8ee7d3`. This is not proof of the bundled DLL's source/compiler ancestry: its existing recorded revision remains explicitly unverified. No DLL rebuild, claimed ancestry, full-provider promotion, or change to missing-platform fail-closed behavior is made in this migration.
