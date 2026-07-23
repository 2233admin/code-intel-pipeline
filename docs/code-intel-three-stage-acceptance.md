# Code Intel three-stage acceptance

`Invoke-CodeIntelAcceptance.ps1` is the language-neutral acceptance entry point used before an
Agent hands off work, before a queue lands work, and before a human promotes an integration branch.
It does not merge, push, publish, or promote anything. It only returns an acceptance decision.

## Fixed stage mapping

| Stage | Required evidence | Project profile |
| --- | --- | --- |
| `agent` | one or more explicit targeted checks, then project conformance | `fast` |
| `land` | project conformance only | `fast` |
| `promote` | project conformance only | `full` |

Project suites remain owned by `Test-CodeIntelProjectConformance.ps1` and
`orchestration/code-intel-project-conformance-policy.v1.json`. The three-stage entry point delegates
to that command; it does not duplicate suite definitions or test implementations.

The stage contract and targeted-command allowlist live in
`orchestration/code-intel-acceptance-policy.v1.json`. The adapter also enforces the fixed mapping
independently, so replacing the policy with a weakened mapping blocks the request.

## Agent targeted checks

Each targeted check is an explicit JSON object. PowerShell test files must be repository-relative;
they are resolved and containment-checked before any command runs:

```powershell
$target = @{
    id = "acceptance-contract"
    kind = "pwsh"
    file = "test-code-intel-acceptance.ps1"
    args = @()
} | ConvertTo-Json -Compress

./Invoke-CodeIntelAcceptance.ps1 -Stage agent -TargetCheckJson $target -Json
```

Language tools use `kind=process` and a policy-allowed executable. Arguments are passed as an argv
array without shell evaluation:

```powershell
$target = @{
    id = "rust-graph"
    kind = "process"
    executable = "cargo"
    args = @("test", "-p", "code-intel", "--test", "graph_adapter")
} | ConvertTo-Json -Compress

./Invoke-CodeIntelAcceptance.ps1 -Stage agent -TargetCheckJson $target -Json
```

For multiple checks, pass a PowerShell string array to `-TargetCheckJson`. All specifications are
validated before the first command executes. Duplicate IDs, repository escapes, shell tokens,
shell executables, inline interpreter evaluation, unsupported properties, and unknown executables
block the complete request.

A change with no applicable focused test may explicitly declare why:

```powershell
./Invoke-CodeIntelAcceptance.ps1 `
    -Stage agent `
    -SkipTargetedChecksReason "documentation-only change; no executable behavior changed" `
    -Json
```

The fast profile still runs, but the overall status is `skipped-with-reason`, not `pass`. Omitting
both targeted checks and a skip reason is `blocked`.

## Machine result and exits

The result schema is `orchestration/schemas/code-intel-acceptance-result.v1.schema.json`.

| Status | Exit | Meaning |
| --- | ---: | --- |
| `pass` | 0 | every required check passed |
| `fail` | 1 | an executed acceptance check rejected the change |
| `blocked` | 2 | the request, policy, command, or child result was unsafe or malformed |
| `skipped-with-reason` | 3 | evidence was explicitly skipped; this is not acceptance |

Consumers must treat every nonzero exit and every status other than `pass` as fail closed.

## Land and promote

```powershell
./Invoke-CodeIntelAcceptance.ps1 -Stage land -Json
./Invoke-CodeIntelAcceptance.ps1 -Stage promote -Json
```

`land` maps to the usable `fast` profile. `promote` maps to `full`; with the current project policy,
full remains intentionally rejected until every release mechanism is implemented. Target overrides
are forbidden at both stages, preventing an operator or Agent from substituting a narrower test for
the project-wide profile.

## Contract test

```powershell
./test-code-intel-acceptance.ps1
```

The test uses an isolated repository-shaped fixture whose path contains spaces. It proves all three
stage mappings, target and project failure propagation, explicit-skip behavior, pre-execution input
validation, shell-token rejection, inline-evaluation rejection, and the four-status result contract.
