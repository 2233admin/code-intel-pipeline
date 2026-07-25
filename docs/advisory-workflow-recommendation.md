# Advisory workflow recommendation

`advisory.workflow-recommend` is a deterministic, read-only proposal atom. It compares the existing matt-flow, gstack, OpenSpec OPSX, and spec-kit candidates using repository-local evidence and returns `code-intel-advisory-workflow-recommendation.v1`.

The atom owns recommendation only. Its `effects` array is always empty; it never prompts, initializes a tool, edits the repository, or emits an Adoption Decision. Promotion from `proposal` to `adoption_decision` remains protected by the A05 authority-transition gate and requires an explicit approved authority event.

`OpenSpec-Detector.ps1` is the standalone atom. `Invoke-WorkflowRecommendation.ps1` is the compatibility facade wrapped by the A01 runtime adapter; `run-code-intel.ps1` invokes the A01 capability. Historical `-SkipOpenSpec` disables the advisory call and `-AutoOpenSpec` maps to the facade's recorded `-Auto` compatibility option; neither flag grants adoption authority.

Production invocation crosses A01 through `code-intel capability exec advisory.workflow-recommend`. The capability declaration and request both use `allowedEffects: []`; stdout is one `code-intel-capability-result.v1` envelope, and the proposal crosses the boundary as `workflow-recommendation.json` with an Artifact Ref. The runtime adapter is `advisory.workflow-recommend.compat`, which wraps the same standalone PowerShell atom used by the compatibility facade.

OpenSpec OPSX, spec-kit, gstack, and matt-flow are candidates, not runtime dependencies. The rollback path is direct invocation of `OpenSpec-Detector.ps1`.
