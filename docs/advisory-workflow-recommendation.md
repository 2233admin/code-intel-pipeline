# Advisory workflow recommendation

`advisory.workflow-recommend` is a deterministic, read-only proposal atom. It compares the existing matt-flow, gstack, OpenSpec OPSX, spec-kit, and BMAD candidates using repository-local evidence and returns `code-intel-advisory-workflow-recommendation.v1`.

The atom owns recommendation only. Its `effects` array is always empty; it never prompts, initializes a tool, edits the repository, or emits an Adoption Decision. Promotion from `proposal` to `adoption_decision` remains protected by the A05 authority-transition gate and requires an explicit approved authority event.

The atom is invoked through the A01 runtime adapter. Neither the historical skip flag nor the auto flag grants adoption authority.

Production invocation crosses A01 through `code-intel capability exec advisory.workflow-recommend`. The capability declaration and request both use `allowedEffects: []`; stdout is one `code-intel-capability-result.v1` envelope, and the proposal crosses the boundary as `workflow-recommendation.json` with an Artifact Ref.

OpenSpec OPSX, spec-kit, gstack, and matt-flow are candidates, not runtime dependencies.
