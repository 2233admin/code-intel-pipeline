# Committed-run artifact index

A08 makes the A07 Run Commit boundary meaningful to index consumers. The authoritative Rust index accepts only run directories whose `run-complete.json` is the exact `code-intel-run-commit.v1` marker produced by A07, whose terminal manifest outcome is `completed`, and whose manifest contains exactly one succeeded, verified `repository.iteration` provenance Artifact Ref.

## Admission

For every candidate run, the Rust engine stably reads the completion marker and its content-addressed manifest, then validates:

- marker schema and closed field set;
- manifest path and digest;
- marker, manifest, snapshot, and run identity binding;
- terminal manifest shape;
- every Artifact Ref declared by completed nodes, including digest, schema, type, path boundary, and consumed snapshot identity.
- exactly one ref with artifact schema `code-intel-repository-iteration-provenance.v1` and type
  `repository.iteration`, under a node whose status is `succeeded`;
- provenance purpose exactly `repository_intelligence_iteration`;
- a producer object with the exact versioned `component`, `contract`, and `version` fields;
- provenance `runIdentity` matching the manifest and marker;
- provenance `snapshotIdentity` matching the manifest, marker, and ref
  `consumedSnapshotIdentity`;
- provenance `repositoryKey` matching the repository directory name and `publicationName`
  matching the run directory name.

Only the lexically latest valid completed repository iteration per repository is projected into `entries`. A snapshot-only administrative commit, governance/decision run, zero or duplicate provenance refs, wrong-status refs, mismatched bindings, and a publication copied beneath another repository or run name are rejected. A07 may retain `domain_failed` or `domain_unknown` runs and their verified artifacts for audit, but A08 classifies them as `non_completed` diagnostics and never exposes them as current authority. Staging directories, markerless A07 object trees, forged or tampered commits, process failures, and legacy report directories are likewise excluded and retained as stable diagnostic rows. The index never repairs, deletes, promotes, or synthesizes provenance for a run.

This boundary provides deterministic local integrity and provenance, not cryptographic authorship.
A local actor able to replace an entire publication coherently can also recompute its JSON digests
and forge the non-cryptographic provenance contract. Excluding that threat requires signed
attestations or an external trust root.

## Rebuild and incremental refresh

`rebuild` scans the artifact authority from scratch. `incremental` accepts an existing index as a hint but revalidates the full A07 boundary; it therefore produces the same canonical bytes as rebuild and cannot preserve an entry whose source was tampered after the prior refresh. The JSON projection has no clock field and is sorted by repository and run identifiers.

```text
code-intel artifact index --artifact-root <root> --output <index.json> --operation rebuild
code-intel artifact index --artifact-root <root> --output <index.json> --operation incremental --existing <index.json>
```

`legacy/update-code-intel-index.ps1` is the compatibility facade. Normal mode routes to the Rust engine and writes a Markdown projection alongside the JSON index. The previous report-based traversal remains available only with `-LegacyCompatibilityMode`; it is advisory, cannot establish repository authority, and does not delete or migrate legacy data.
