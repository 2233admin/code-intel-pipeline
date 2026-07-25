# Multi-Agent Workspace Governance

`Invoke-MultiAgentWorkspacePreflight.ps1` is a read-only admission gate for agents that share a local Git worktree. It inventories the complete worktree before mutation begins; it does not clean, stash, reset, commit, or write files in the inspected repository.

## Default mutation preflight

```powershell
pwsh -NoProfile -File ./Invoke-MultiAgentWorkspacePreflight.ps1 -RepoPath . -Intent mutation -Json
```

The default intent is `mutation`. A clean repository root exits `0`. A dirty repository root emits its inventory and exits `20`, so an agent must move to a clean dedicated worktree instead of adding more changes to the shared root.

## Explicit observation

```powershell
pwsh -NoProfile -File ./Invoke-MultiAgentWorkspacePreflight.ps1 -RepoPath . -Intent observation -Json
```

Observation is explicit and exits `0` even when the root is dirty. The result always carries `authority: observation_only`; this path permits inspection only and cannot be treated as write authority.

## Result contract

The JSON result includes:

- tracked, untracked, staged, worktree, and total change counts;
- normalized entries with porcelain status, change group, path, and original path for renames/copies;
- grouped counts for added, modified, deleted, renamed, copied, type-changed, unmerged, untracked, and other changes;
- a deterministic SHA-256 over canonical inventory JSON;
- the decision, reason, intent, and repository-root check.

The hash excludes timestamps and absolute repository paths. Identical change inventories therefore produce the same hash in different fixture locations. Any missing repository, subdirectory invocation, Git inspection error, malformed porcelain entry, or invalid policy fails closed.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Clean mutation preflight or explicit observation allowed |
| `2` | Policy invalid |
| `20` | Dirty root denied for mutation-oriented work |
| `21` | Git worktree root required |
| `22` | Repository or inventory inspection failed |

This preflight is local workspace governance. It complements project acceptance and merge serialization but does not replace either one, and it does not coordinate agents across machines.
