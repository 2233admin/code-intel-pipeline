# Architecture graph provider adapter

`provider graph-adapt` is the B02 seam between graph producers and engineering consumers. It translates the existing internal Rust graph output or an explicitly selected Understand-compatible fallback into `code-intel-architecture-graph-port.v1`, then submits the observation to A04 evidence admissibility. It does not build, enrich, rank, or repair a graph.

## Public route

```text
code-intel provider graph-adapt \
  --request <native.json|-> \
  --artifact-root <artifact-directory> \
  --evaluated-at <unix-seconds> \
  --max-age-seconds <seconds>
```

The PowerShell facade exposes the same route through `-GraphAdapterRequest`, `-GraphAdapterArtifactRoot`, `-GraphAdapterEvaluatedAt`, and `-GraphAdapterMaxAgeSeconds`. Success emits one `code-intel-graph-route-result.v1` document and exits `0`. Usage errors exit `64`. Invalid input, stale evidence, a wrong snapshot, digest failure, or payload/port drift emits one rejected route document and exits `65`. Diagnostics never include request bytes.

## Provider-neutral port

Internal and external producers share the same closed port fields: status, completeness, freshness, expected and source snapshot identities, provider identity, provenance, payload Artifact Ref, and `anatomyUsable`. The provider mode changes identity, not structure.

An internal producer must set `fallback` to `null`. An external producer must name a fallback identity and declare activation as `explicit_fallback` or `legacy_rollback`. External execution is never an automatic primary path.

`anatomyUsable` starts false and becomes true only after all of these conditions hold:

1. the producer reports a current and complete graph;
2. the graph is bound to the requested snapshot identity;
3. its Artifact Ref digest, schema, and consumed snapshot are valid;
4. A04 returns `domainVerdict=observed`; and
5. the admitted payload agrees with the port provider and provenance.

A graph file merely being present is not evidence of current anatomy. Wrong-HEAD and stale graphs fail closed through A04. Missing and partial graphs may remain admissible as explicit unknown evidence, but they never become usable anatomy and never produce engineering facts.

## Payload boundary

The referenced evidence payload carries `data.architectureGraph` with schema `code-intel-architecture-graph-evidence.v1`. Its snapshot, provider identity, completeness, and provenance must equal the translated port. Current evidence must contain an Understand-compatible graph document; missing evidence must contain `null`; partial evidence may contain either.

The route deliberately emits `engineeringFacts: []`. Fact promotion belongs to the A04/A05 authority path and later diagnosis atoms, not this adapter.

## Rollback

Rollback selects the legacy Understand command as an external provider with `activation=legacy_rollback`. It does not bypass the adapter, change the port contract, relax snapshot binding, or skip A04.
