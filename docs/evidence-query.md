# Verified Evidence Query

`artifact query` is the bounded, model-independent read port for A08 committed runs. It rebuilds
the committed-only index, selects the latest admitted run for one repository, re-verifies every
Artifact Ref against its registered schema, digest, and snapshot identity, and only then applies
schema, type, or content filters.

The optional `--repo-path` flag recomputes the repository snapshot with the policy and scope stored
in the committed `repository.snapshot` artifact. The result reports `current` or `stale`; without a
repository path it reports `unknown` instead of treating artifact presence as freshness.

The command returns deterministic JSON under `code-intel-evidence-query.v1`. Matches contain the
original Artifact Ref, the filters that matched, a bounded 400-character preview, and an explicit
verification explanation. It does not invoke a model, mutate a repository, generate code, or infer
semantic claims beyond the verified artifact bytes.

```text
code-intel artifact query --artifact-root <root> --repo <name> \
  --repo-path <checkout> --type inventory.files --contains src/lib.rs
```
