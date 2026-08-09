# Spec workflow adapter recommendation

The former PowerShell “OpenSpec detector” has been replaced by the Rust-native, tool-neutral workflow adapter recommender. The filename remains as a documentation compatibility link; it no longer describes a detector implementation.

The v1 projection still carries matt-flow's idea→ship and gstack delivery candidates for existing consumers. They are compatibility alternatives, not v2 selection inputs or runtime dependencies.

## What is decided

The recommender chooses from governed capabilities, not repository age and not a tool-name phrase. OpenSpec OPSX and spec-kit are both valid for brownfield work:

| Adapter | Pinned provenance | Distinct capability fit |
| --- | --- | --- |
| OpenSpec OPSX | 1.8.0, `d57889664cab4f2f061d236ec3ff82a5578701bb`, MIT | change deltas, continuous governance, proposal → specs → design → tasks |
| spec-kit | 0.16.1, `ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6`, MIT | constitution, clarification, checklists, convergence, composed specify/plan/implement flow |
| lightweight local | pipeline-owned | bounded work that does not need either governance system |

An existing `openspec/`, `.specify/`, or `specs/` root is configuration evidence. It is not an adoption decision. Two active normative roots are a conflict that requires an explicit authority decision. A manual override is accepted only with a recorded reason.

## Structured actions

The catalog distinguishes:

- normative entry actions, such as OpenSpec explore/propose/apply-change/archive/sync and spec-kit specify/plan/implement/analyze/checklist;
- setup actions, such as `openspec init` and `specify init`;
- maintenance actions, such as OpenSpec update.

Generated OpenSpec actions depend on the installed host profile. The proposal marks an entry available only when the corresponding generated action is observed; it never prints an unsupported invocation as callable. Setup and maintenance remain separately authorized effects.

## Activation boundary

Agent hosts map phrases such as “定案” to `plan`, “明确 apply 请求” to `implement`, “验证” to `verify`, “开 PR” to `ship`, and “复盘” to `observe`. The Rust core receives those closed intents and has no multilingual phrase parser. `ship` and `observe` are explicit handoffs until the separate shipping control loop and outcome ledger exist on the active branch.

Use the compiled CLI through A01. The retained PowerShell command is a thin compatibility forwarder only:

```powershell
.\legacy\Invoke-WorkflowRecommendation.ps1 -RepoPath <path> -Auto -Json
```

The result remains a zero-effect proposal. It cannot initialize a workflow, execute a generated action, create a PR, grant merge authority, or claim that an intervention improved anything.
