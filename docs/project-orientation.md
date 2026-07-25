# Project Orientation

`project.orientation` is D01's deterministic first actionable project view. It consumes only A03-verified A02 snapshot, inventory, B05 survival-scan, and B08 native evidence artifacts. It does not read repository content directly, invoke an LLM, or create semantic architecture claims.

The machine artifact is `project-orientation.json` (`code-intel-project-orientation.v1`). Every known, risk, confidence, and unknown claim carries the input artifact type, SHA-256, and JSON pointer that support it. When no admitted purpose evidence is present, `purpose.status` is `unknown` and `purpose.evidence` is empty.

`project-orientation.md` is a rebuildable Summary-compatible projection with the sections Identity, Purpose, Languages, Boundaries, Entry Points, Commands, Active Change, Risks, Unknowns, and Confidence. Existing run summaries remain authoritative during the additive D01 rollout.

The current A01 atom is registered but is not silently added to the default A09 DAG: B05 is still exposed as a verified standalone survival scan rather than an A01 DAG producer. Adding D01 to the default DAG before that dependency can supply an Artifact Ref would invent a dependency result. The atom is executable now with six verified inputs; the default DAG route remains an explicit follow-on integration boundary.
