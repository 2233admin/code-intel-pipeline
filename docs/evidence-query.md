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

The command returns deterministic JSON under `code-intel-evidence-query.v1`. Matches contain the
original Artifact Ref, the filters that matched, a bounded 400-character preview, and an explicit
verification explanation. It does not invoke a model, mutate a repository, generate code, or infer
semantic claims beyond the verified artifact bytes.

```text
code-intel query <checkout> --kind evidence \
  --type inventory.files --contains src/lib.rs --json
```

`artifact query --artifact-root <root> --repo <name> ...` remains the explicit low-level
administration and compatibility surface. It traverses the same verified evidence format, but it is
not the default interface for agents or humans working in a checkout.
