# Snapshot-bound Change Impact

`change impact` derives conservative impact and test candidates from the latest A08-admitted run.
It first revalidates the committed evidence and recomputes the current repository snapshot. A stale
snapshot is a contract failure: impact is never inferred by mixing evidence from one checkout state
with changed paths from another.

The v1 implementation walks the verified Native Code Evidence import list in reverse from explicit
`--changed` paths. Exact relative/module resolution is high confidence; a unique suffix resolution
is medium confidence. Impacted test files become the minimal candidates. When the graph reaches no
test, same-module test co-location is an explicit fallback. Returned commands are advisory strings
only and are not executed.

The output names its limitations: the native parser is heuristic and cannot prove runtime calls,
dynamic imports, generated-code edges, reflection, or build-system dependencies. This makes the
result useful for prioritization without presenting it as a semantic proof.
