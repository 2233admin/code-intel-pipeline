# Repository Snapshot Identity v1

`code-intel snapshot identity` implements A02 `repository.snapshot-identity`. It produces identity material only. It does not read or trust Artifact Ref payloads; that remains A03.

## Command

```powershell
code-intel snapshot identity --repo <root> --working-tree-policy <head_only|explicit_overlay> --scope <relative-path>
```

`--scope` is repeatable. Empty scope means `.`. Components are normalized to `/`, sorted, deduplicated, and remain case-sensitive contract values. Absolute paths and `..` are rejected. `--repo` must be the Git worktree root. Output contains no absolute path or timestamp.

Scope is reduced to its smallest prefix set: `src` absorbs `src/nested`, and `.` absorbs every other scope. Equivalent sets therefore have the same identity and input digest. On Windows, distinct spellings that collide case-insensitively fail closed.

## Identity contract

`snapshot.identity` is SHA-256 over length-prefixed v1 records for repository identity, resolved HEAD, working-tree policy, canonical scope, and `inputDigest`. Each digest uses a version/domain record.

- Normal Git: `repoIdentity=git-lineage-v1:<sha256>` from the sorted root commits reachable from the consumed HEAD. Remote URL, other refs, checkout path, branch, and clock are excluded. Attached and detached checkouts at the same commit agree. Unrelated orphan histories and other unreachable roots are intentionally excluded: this identity names the lineage actually consumed, not every ref stored in the local object database.
- Shallow Git: exit 69 because the lineage boundary is incomplete.
- Unborn Git: `explicit_overlay` uses `content-v1:<sha256>` and `head=unborn`; `head_only` exits 69.
- No Git metadata: `explicit_overlay` uses `content-v1:<sha256>` and `head=unversioned`; `head_only` exits 69.
- Missing Git executable: exit 69; it is never silently reclassified as unversioned.

## Input encoding and overlay membership

`head_only` resolves HEAD once and reads its immutable tree by object id. Sorted records retain mode, kind, object id, and relative path. Executable mode, symlink blobs, Gitlinks/submodules, and LFS pointer blobs are therefore checkout-independent.

`explicit_overlay` uses the Git index plus non-ignored untracked paths. Sorted length-prefixed records contain domain, kind, Git mode, relative path, byte length, and raw bytes. Deleted tracked files emit tombstones. Intent-to-add (`git add -N`, porcelain ` A`) is a modified overlay member. Gitlinks emit the indexed commit and do not recursively consume the submodule worktree. Symlinks emit their target text and are never followed. LFS worktree files are the bytes actually consumed. Ignored files are excluded and output declares `ignoredPolicy=excluded_by_git_ignore`.

The report separates `trackedModified`, `trackedDeleted`, `untracked`, `renamed`, `typeChanged`, and `staged`; a boolean alone is not the dirty-tree contract.
Porcelain v1 XY states are table-driven: ignored (`!!`) is excluded, intent-to-add is retained, and every unmerged state (`DD`, `AU`, `UD`, `UA`, `DU`, `AA`, `UU`) fails closed instead of producing a partial identity.

For TOCTOU resistance, overlay status and the complete input digest are computed before and after reading; the unversioned path set and digest receive the same double check. Any difference exits 74. The `inventory.rg` facade additionally opens a snapshot lease that retains the expected input manifest: canonical repository-relative path, kind, mode, content digest, policy, and canonical scope. Frozen bytes are also retained for snapshot-controlled `.gitignore`, `.ignore`, and `.rgignore` files. A repository-root `rg --no-ignore` traversal checks live path membership against an owned manifest mirror baseline; it does not consume live ignore contents or custom filtering globs. The filtered artifact set is produced only by running ripgrep over that mirror with frozen ignore bytes and the declared default/custom globs. Paths are normalized to repository-relative `/` form. Extra, missing, or transiently renamed baseline paths exit 65 and publish no inventory. The complete manifest is re-read and matched again before publication.

This lease is a set-and-content cross-check, not a filesystem lock: writers are not blocked, but a consumer cannot publish unless the observed path set and the post-consumption manifest still match the requested snapshot. It is a bounded consumer closure, not the A09 DAG or A03 Artifact Ref verifier.

## Exit classes

- `0`: one compact JSON document on stdout, stderr empty.
- `64`: invalid CLI, scope, or repository-root usage.
- `69`: Git/rg unavailable, incomplete lineage, or impossible policy.
- `74`: filesystem read or concurrent-change failure.
