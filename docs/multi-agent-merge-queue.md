# Multi-Agent Merge Queue

Code Intel Pipeline treats parallel implementation and repository landing as separate contracts:

1. Agents work in isolated worktrees.
2. The project acceptance command decides whether a lane is green. It may use Pon-derived
   conformance, language adapters, tests, lint, type checks, builds, or any equivalent project-local
   standard.
3. `claude-code-merge-queue` serializes rebase, acceptance, and push through its FIFO landing queue.
4. A human promotes the integration branch to the production branch.

The Pipeline adapter does not copy the queue implementation. It invokes a repository-local install
pinned by the target project and fails closed before `land` unless all eight readiness gates pass.
Ordinary code-intel analysis never requires the queue.

## Status and validation

```powershell
./Invoke-MultiAgentMergeQueue.ps1 -Action status -RepoPath C:\path\to\repo -Json
./Invoke-MultiAgentMergeQueue.ps1 -Action validate -RepoPath C:\path\to\repo
```

The default command resolver accepts only `node_modules/.bin/claude-code-merge-queue`; it never uses
an unpinned `npx` download. `status` is observational and exits successfully with `ready:false` when
the project is not configured. `validate` uses the same result but exits nonzero when any landing
gate fails.

## Landing

```powershell
./Invoke-MultiAgentMergeQueue.ps1 -Action land -RepoPath C:\path\to\lane `
  -AllowRepositoryMutation -AllowNetworkPush
```

`land` requires both explicit authority switches because it rebases the lane and pushes a remote.
Readiness never implies mutation or network authority. The adapter delegates only `land`,
`reconcile`, and `land-history`. It cannot initialize or uninstall
the provider, delete worktrees, preview changes, use the emergency bypass, or run `promote`.

For this repository, the queue's project-local config should use the fast unified acceptance profile:

```javascript
export default {
  integrationBranch: "integration",
  productionBranch: "main",
  checkCommand: "pwsh -NoProfile -File ./scripts/tests/Test-CodeIntelProjectConformance.ps1 -Profile fast",
  checksRequired: true,
};
```

Other repositories replace `checkCommand` with their own unified acceptance entrypoint. The adapter
does not assume Python, Pon, Node, or any particular application language; only the selected queue
provider is Node-based.

## Limits

- The selected provider coordinates worktrees on one machine, not a distributed fleet.
- Local hooks prevent mistakes and workflow drift; they are not a security boundary against an
  adversarial process with shell access.
- A queue proves that the configured check ran, not that the check is sufficient. The project policy
  remains the acceptance authority.

Source: `2233admin/claude-code-merge-queue` revision
`e7a76958dbd3953b84f12abbc2e6bd755aafce53`, version `0.5.1`, MIT license.
