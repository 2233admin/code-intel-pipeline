# Diaphora result-database adapter

`code-intel provider diaphora-inspect` imports a result database that an operator has already produced with [Diaphora](https://github.com/joxeankoret/diaphora). It is an advisory, read-only evidence boundary.

It does not install, embed, invoke, or distribute Diaphora, IDA, or the source and binary databases used to generate a comparison. Diaphora remains an external AGPL-3.0 tool; Code Intel only links an observation to its supplied input files.

## Produce and inspect an observation

Use the upstream tool to export two databases and create a result database first. For example, Diaphora documents this form:

```text
diaphora.py <base-export.sqlite> <candidate-export.sqlite> -o <results.diaphora>
```

Then import that result database with the actual binaries that the comparison represents:

```text
code-intel provider diaphora-inspect \
  --result-db <results.diaphora> \
  --base-binary <base-binary> \
  --candidate-binary <candidate-binary> \
  --source-revision <source-revision> \
  --provider-version <diaphora-version> \
  --observed-at <unix-seconds> \
  --out <observation.json>
```

The observation contains SHA-256 identities for the two binaries and result database, the Diaphora result-schema version, exact category counts, and at most 20 function-match summaries. Filesystem paths are never emitted. Function names and heuristic descriptions are truncated to 512 Unicode scalar values.

## Status and authority

- `observed` / exit `0`: the three inputs exist and the result database has the Diaphora `config`, `results`, and `unmatched` layouts.
- `unavailable` / exit `69`: a required input cannot be read. `failure.kind` is `provider_unavailable`; `comparison` and `summary` are `null`. This is not a clean comparison.
- `rejected` / exit `65`: the database cannot be read as the expected Diaphora result layout. `failure.kind` is `process_failure`; `comparison` and `summary` are `null`.
- invalid CLI input exits `64` before an artifact is written.

Every emitted observation has `authority.observationOnly: true` and an empty `engineeringFacts` array. Diaphora matches can focus human review, but they do not prove an engineering conclusion and cannot make a CI or governance gate green.
