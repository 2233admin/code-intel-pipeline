# Sentrux provider adapter

`provider sentrux-adapt` is the only structural-evidence ingress for Sentrux observations. In the default normal DAG, the built-in provider executes real `sentrux gate` and `sentrux check` commands, records their command-level outcomes, normalizes the six registered authoritative rule kinds, binds evidence to a repository snapshot, and submits the result to A04 before exposing diagnosis eligibility.

Unknown rule kinds, partial collections, command failures, and provider crashes remain `partial`/`unknown`; they never become passing diagnoses. `Invoke-SentruxAgentTool.ps1` and the bundled shim remain compatibility/rollback paths, not the normal authority route. The Rust boundary owns invocation, normalization, effects, and evidence contracts; Sentrux owns its scanning algorithms, while Hospital owns diagnosis policy.

The checked contracts are `code-intel-structural-evidence-port.v1` and `code-intel-sentrux-route-result.v1`. Successful routes emit structural observations and command evidence, not Engineering Facts; downstream diagnosis and fact promotion remain separate authority decisions.
