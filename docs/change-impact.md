# Snapshot-bound Change Impact

`change impact` derives conservative impact and test candidates from the latest A08-admitted run.
It first revalidates the committed evidence and recomputes the current repository snapshot. By
default a stale snapshot is a contract failure: impact is never inferred by mixing evidence from
one checkout state with changed paths from another. Every result carries a `freshness` object
whose `status` is `current` when the recorded and recomputed snapshot identities match.

`--staleness advisory` opts out of the fail-closed default. Mid-edit the working tree has always
diverged from the committed snapshot, so the strict contract makes the query unusable exactly when
writing. In advisory mode a mismatch no longer fails: the walk still runs over the latest committed
run's verified evidence, with the caller's `--changed` paths overlaid as the query input, and the
output labels the gap instead of hiding it. `freshness.status` becomes `stale-advisory`, top-level
`recordedSnapshotIdentity` and `currentSnapshotIdentity` fields expose both identities, and an
extra limitation names the degradation. Advisory answers may be based on an outdated graph and
must never gate; they exist only to prioritize while editing. `--staleness current` is the
explicit spelling of the default, and when the identities match, advisory output is identical to
default output (`freshness.status` stays `current` in both modes).

The v1 implementation walks the verified Native Code Evidence import list in reverse from explicit
`--changed` paths. Exact relative/module resolution is high confidence; a unique suffix resolution
is medium confidence. Impacted test files become the minimal candidates. When the graph reaches no
test, same-module test co-location is an explicit fallback. Returned commands are advisory strings
only and are not executed.

The output names its limitations: the native parser is heuristic and cannot prove runtime calls,
dynamic imports, generated-code edges, reflection, or build-system dependencies. This makes the
result useful for prioritization without presenting it as a semantic proof.
