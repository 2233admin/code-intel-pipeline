# Verified Evidence Query

`query` is the routine, model-independent read port for A08 committed runs. Its `ProjectContext`
resolves the checkout, repository key, and configured/default artifact root from one repository
path. Callers do not select a run, manifest, repository key, or artifact placement. The controller
then selects the latest admitted run, re-verifies every Artifact Ref against its registered schema,
digest, and snapshot identity, and only then applies schema, type, or content filters.

The repository path is always checked against the committed `repository.snapshot` identity. A
successful strict query reports `current`; a stale checkout or a different repository with the same
directory name is refused instead of receiving unrelated evidence. The explicit low-level query can
still report stale evidence for advisory administration flows.

For unborn Git and unversioned directories, `ProjectContext` rebuilds the recorded snapshot with its
committed working-tree policy and scope and binds the resulting `content-v1` identity. An unchanged
checkout therefore supports run → query → rerun. Once its inputs change, content identity alone
cannot prove repository continuity, so the automatic interface fails closed; initialize Git before
establishing long-lived authority, or intentionally start a new authority in a distinct artifact
root.

The command returns deterministic JSON under `code-intel-evidence-query.v2`. Matches contain the
original Artifact Ref, the filters that matched, a bounded 400-character preview, an explicit
verification explanation, and publication-time `anchorEvidence`. It does not invoke a model, mutate
a repository, generate code, or infer semantic claims beyond the verified artifact bytes. The
historical v1 schema remains immutable; current query routes produce only v2.

The result separates four dimensions that must not be treated as a requirement acceptance verdict:

- `evidenceAssessment` records `artifactIntegrity: verified` and `snapshotBinding: verified` for
  registered-schema, digest, and artifact/run snapshot checks. `behaviorVerification: not_assessed`
  means the query has not verified repository behavior or a requirement. Freshness is separate.
- `artifactAvailability` lists the sorted unique `availableArtifactTypes` in this run.
  `requestedEvidenceStatus` is `available` when the requested schema/type contract exists,
  `unavailable` when it does not, or `not_requested` when neither filter was supplied. Content
  matching does not change contract availability.
- `searchCoverage` has `scope: committed_artifacts`. Its status is `complete` when matching search
  in the admitted run is exhausted, or `truncated` when one additional matching record exists beyond
  the requested limit. Exactly the limit is complete if no further match exists. No extra match is
  returned, and search stops once overflow is known. This is not whole-project or semantic coverage.
- `unknowns` explains absent requested contracts, defensive noncompleted run outcomes, and stale or
  unknown freshness. A complete search with no hits is not a behavioral pass; an absent contract can
  have complete search coverage while evidence availability is unavailable.

Content filters match full authenticated artifact bytes, not previews. `previewTruncated` describes
only the preview bound and is independent of search truncation.

Each match's `anchorEvidence` delivers existing authenticated `verification.anchors` from the same
run without reading live source files or recomputing verification. Its `basis` is `publication_time`
and `currentValidity` is always `not_assessed`, even when snapshot freshness is current under
`head_only`. `status` (`available`, `unavailable`, or `not_applicable`) describes source/report
availability, not a blanket verification result. `reportRef` preserves the original report Artifact
Ref when present; `reason` explains missing or unsupported evidence.

Supported source contracts are `agent-code-slice-ranking.v1` / `code_evidence.agent_slice` file
claims, `code-evidence-symbols.v1` / `code_evidence.symbols` symbol claims, and
`code-intel-surgery-plan.v1` / `diagnosis.surgery-plan` primary target file claims. Association uses
the exact source artifact type, artifact path, anchor kind, and claim identity; missing or
conflicting associations never become verified by aggregate counts or approximate names.

At most 20 original-order claims appear in `items`; `itemsTruncated` means additional supported
source claims exist. Each item preserves `path`, `name`, `claimedLine`, `state`, `resolvedLine`,
`navigationTarget`, and `reason`. Verified file targets contain the path and a null line; verified
symbol targets use the claimed line; approximate targets use the resolved line. Dropped and
not-assessed claims remain visible with a null navigation target and an explanation. Historical
runs without a companion report remain queryable with explicitly unavailable navigation evidence.
New reports bind each source to its final `objects/sha256/<digest>` Artifact Ref path, not its
pre-publication staging name. Older reports whose named source paths cannot be associated exactly
remain unavailable; the query does not guess aliases or implicitly rerun the pipeline.
Tampered report bytes still fail the ordinary evidence verification before querying.

```text
code-intel query <checkout> --kind evidence \
  --type inventory.files --contains src/lib.rs --json
```

`artifact query --artifact-root <root> --repo <name> ...` remains the explicit low-level
administration and compatibility surface. It traverses the same verified evidence format, but it is
not the default interface for agents or humans working in a checkout.
