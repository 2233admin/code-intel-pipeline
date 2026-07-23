# Evidence admissibility v1

`code-intel evidence validate` is the provider-neutral boundary between provider output and Pipeline Observed Evidence. It validates a versioned request, applies an explicit freshness policy, and verifies the payload through the A03 Artifact Ref verifier against the A02 snapshot identity.

The validator does not consult the integration registry to decide provider semantics. A provider unknown to the registry receives exactly the same checks as any registered provider. Provider-specific health probes and translation remain adapter responsibilities.

An admitted result remains Observed Evidence. `engineeringFacts` is structurally empty in every v1 result; A05 owns later authority transitions. The deterministic `admissionIdentity` makes a replay of the same observation recognizable without manufacturing a new authority event. Partial evidence may be admitted only when it is honestly labelled partial. Provider-unavailable and domain-unknown states remain partial/unknown; a process failure is not admissible evidence. Malformed, stale, snapshot-mismatched, digest-mismatched, or incomplete-as-complete evidence exits 65 with `domainVerdict: unknown`. Host I/O failure exits 74.

The executable fixture is under `tests/fixtures/evidence-admissibility/good`.
