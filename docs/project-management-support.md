# Project Management Support

Project Management Support is Code Intel Pipeline's agent-intake layer for turning repository evidence into trackable work and durable project knowledge. It internalizes the useful setup concepts from `mattpocock/skills`: issue tracker choice, triage label vocabulary, and domain documentation layout.

It is not scanner runtime. Do not install `mattpocock/skills`, Linear clients, Obsidian plugins, or wiki tooling to run Code Intel Pipeline. Scanner-owned artifact runs remain produced only by `run-code-intel.ps1` and `invoke-code-intel.ps1`.

## Surfaces

- Work control plane: where actionable work lives. Selection is local-first and project-specific: reuse an existing Git-backed Work-OS or repo-native task graph before creating a new external SaaS project. Linear and GitHub Projects are optional projections, not prerequisites.
- Triage labels: stable states agents can apply or report against when moving work through evaluation, reporter follow-up, agent-ready, human-ready, and wontfix states.
- Domain docs: the authoritative repo-local knowledge surface agents read before planning or coding.
- Obsidian/LLM wiki: a knowledge view by default; when a project already has a reviewed Git-backed Work-OS issue contract, it may also own task state. It still never replaces scanner artifacts as technical evidence.

## Control-Plane Selection

Choose exactly one writable task-state authority for an initiative:

1. follow an explicit user selection;
2. otherwise reuse the project's existing Git-backed Work-OS or repo-native task graph;
3. use Linear, GitHub Projects, or another hosted tracker only when explicitly requested or already bound as the project authority.

Do not mirror mutable status bidirectionally between trackers. Other systems may index links and reviewed summaries, but they are read-only projections unless a deliberate migration changes the authority.

## Hosted Tracker Boundary

Hosted tracker support means Code Intel artifacts can be referenced from external issues and those issues can carry triage state when selected. It does not mean the scanner writes to any tracker by default.

Do not store API keys, OAuth tokens, workspace IDs, or user secrets in this repository. A helper that writes external issues must require explicit user authorization, read credentials from user-scoped environment or an approved secret store, and record local evidence before external writes.

## Obsidian/LLM Wiki Boundary

Obsidian/LLM wiki support means repo docs and artifact summaries can be mirrored, indexed, linked, or summarized for project management. The wiki is a knowledge surface, not artifact authority.

Use the wiki to find context; use artifact runs for current scanner evidence. If wiki notes conflict with `summary.md`, `hospital.md`, `understanding.md`, `CONTEXT.md`, or ADRs, treat repo-local artifacts and docs as authoritative until deliberately updated.

## Required Files

- `docs/agents/issue-tracker.md`: issue tracker selection and external-action boundary.
- `docs/agents/triage-labels.md`: canonical triage roles and Linear label/status mapping.
- `docs/agents/domain.md`: domain documentation layout and wiki consumption rules.
- `CONTEXT.md`: shared vocabulary for these concepts.
- `docs/adr/0006-project-management-support-as-agent-intake.md`: decision record.

## Agent Flow

1. Read `docs/agents/domain.md`, then the relevant `CONTEXT.md` and ADRs.
2. Run Code Intel Pipeline when fresh repository evidence is needed.
3. Convert verified artifact evidence into the selected control plane only when requested or when the active workflow explicitly requires it.
4. Preserve artifact links and verification commands in the issue or note.
5. Never let task or wiki state replace scanner evidence, artifact data contract fields, or safety checks.
