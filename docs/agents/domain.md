# Domain Docs

Code Intel Pipeline uses a single-context domain layout:

- `CONTEXT.md`: shared vocabulary and terms agents must use.
- `docs/adr/`: accepted architectural and workflow decisions.
- `docs/project-management-support.md`: project-management intake and wiki boundary.
- `docs/agents/issue-tracker.md`: issue tracker selection.
- `docs/agents/triage-labels.md`: triage state vocabulary.

## Before Work

Before planning or coding, read the relevant repo-local domain docs first:

1. `CONTEXT.md`
2. ADRs in `docs/adr/` that touch the workflow
3. `docs/project-management-support.md` when work may become a Linear issue or wiki note

Proceed silently when optional files are missing in a target repository. Do not create domain docs unless the task requires it.

## Obsidian/LLM Wiki

An Obsidian/LLM wiki may mirror or index these docs, artifact summaries, and handoff notes. It is a project-management knowledge surface, not artifact authority.

If wiki content conflicts with repo-local docs or artifact reports, prefer `CONTEXT.md`, ADRs, `summary.md`, `hospital.md`, `understanding.md`, and machine-readable artifact files until the source docs are deliberately updated.

Use glossary vocabulary from `CONTEXT.md` in issue titles, wiki notes, code comments, and test names. If the needed term is missing, record the gap rather than inventing a parallel vocabulary.
