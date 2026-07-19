# ADR 0008: Select the Project Control Plane Locally First

- Status: accepted
- Date: 2026-07-13

## Context

The project-management intake documentation treated Linear as the preferred default. That assumption is unsafe in an environment that already has Git-backed local project management, cross-device vault synchronization, worktree isolation, and long-lived LLM Wiki memory. It can create duplicate projects, split mutable task state across systems, and make an external quota or service availability look like a pipeline blocker.

Scanner evidence has a different authority boundary from task coordination. Reports, contracts, repository docs, and tests describe technical truth. A task system describes intent, ownership, dependencies, and review state. Neither should silently replace the other.

## Decision

Project-control-plane discovery is local-first:

1. honor an explicit user selection;
2. discover and reuse an existing Git-backed Work-OS or repo-native task graph;
3. treat hosted trackers as optional selected authorities or read-only projections;
4. maintain exactly one writable source of mutable task state per initiative.

For the trust-boundary initiative, the existing LLM Wiki Work-OS project owns the Wayfinder map, PRD, ticket state, and blocked-by graph. Its Git repository provides review history and cross-device transport. The code-intel-pipeline repository owns implementation, tests, ADRs, artifact contracts, and generated evidence.

Machine-specific paths remain in local bindings. Shared records use logical identities and repository-relative links. Agent execution may use isolated worktrees, but no runtime database or agent session becomes task authority.

## Consequences

- Linear availability, quota, or authentication cannot block planning or implementation.
- Cross-device recovery uses Git fetch/checkout plus the selected Work-OS records.
- LLM Wiki may be authoritative for task state while remaining non-authoritative for scanner measurements.
- Bidirectional status mirroring between Work-OS, Linear, and GitHub Projects is prohibited without a deliberate migration.
- Existing tools are reused before introducing a new coordination format, daemon, or dependency.
