# Automatic pull request beta atom

`Invoke-CodeIntelAutomaticPullRequest.ps1` is an optional execution atom. Ordinary Code Intel
scans, GitHub solution research, Hospital diagnosis, and workflow recommendations never invoke it.
Finding a repairable problem is a proposal, not permission to edit a repository or create a pull
request.

## Consent and effect boundary

The atom is disabled unless every execution supplies all of the following:

1. a canonical `code-intel-auto-pr-proposal.v1` document containing the exact PR fields;
2. a committed C07 Decision Record whose evidence binds that proposal file's SHA-256 and whose
   replay still returns `status=replay`, `questionRequired=false`;
3. a one-time `code-intel-auto-pr-authorization.v1` document bound to that Decision Record;
4. the expected Git HEAD and snapshot identity as separate command arguments;
5. `-AllowRepositoryMutation` and `-AllowNetworkPrCreate`.

The authorization is valid for exactly one current repository snapshot, one `owner/name`, one base
branch, and one head branch. It expires, permits draft PRs only, and is consumed after successful
creation. A receipt keyed by a canonical semantic digest of the exact proposal fields under the Git common directory
prevents either the same consent from being rewrapped or the same proposal from being approved twice
under different Decision Records. Missing permission, drift,
expiry, concurrent execution, replay, a non-draft request, or malformed authorization fails before
`gh pr create` runs.

Once the network call starts, the authorization is consumed even if `gh` fails or its output cannot
prove which PR was created. That result is `execution_indeterminate`; an operator must inspect the
repository's pull requests instead of retrying the same authorization and risking a duplicate PR.

The snapshot identity is SHA-256 over UTF-8 text containing the v1 domain marker, lowercase HEAD,
and the sorted output lines of `git status --porcelain=v1 --untracked-files=all`. This binds consent
to tracked and untracked working-tree state without relying on a machine-specific path.

## Example

For the full proposal → decision → C07 record → executor sequence, prefer
`Invoke-CodeIntelAutomaticPullRequestFlow.ps1`. The lower-level executor example below remains useful
for hosts that already own the decision artifacts.

```powershell
./Invoke-CodeIntelAutomaticPullRequest.ps1 `
  -RepoPath C:\src\project `
  -AuthorizationPath C:\secure\auto-pr-authorization.json `
  -ProposalPath C:\secure\auto-pr-proposal.json `
  -DecisionStore C:\secure\decision-store `
  -DecisionReplayQueryPath C:\secure\auto-pr-replay-query.json `
  -CodeIntelCommand C:\tools\code-intel.exe `
  -ExpectedHead <40-lowercase-hex> `
  -ExpectedSnapshotIdentity <64-lowercase-hex> `
  -AllowRepositoryMutation `
  -AllowNetworkPrCreate `
  -Json
```

The Pipeline's initial question only asks whether to enter the automatic-PR proposal flow. It never
authorizes a concrete PR. After a fix branch and exact proposal exist, the caller must hash the
canonical proposal into a new decision request, record the response through C07, and supply a replay
query at execution. A bare “yes”, the initial feature opt-in, a workflow recommendation, GitHub
research evidence, or a successful test run is not execution authority.

C07 proves content binding and replay validity, not cryptographic human identity. Deployments whose
threat model includes a malicious local writer must obtain the Decision Response through a trusted UI
or add a verifiable digital-signature layer.

Machine contracts are defined by:

- `orchestration/schemas/code-intel-auto-pr-authorization.v1.schema.json`;
- `orchestration/schemas/code-intel-auto-pr-proposal.v1.schema.json`;
- `orchestration/schemas/code-intel-auto-pr-execution-result.v1.schema.json`;
- `orchestration/schemas/code-intel-auto-pr-execution-receipt.v1.schema.json`.
