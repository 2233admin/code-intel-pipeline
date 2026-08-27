# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root — shared vocabulary and terms agents must use.
- **`docs/adr/`** — read ADRs that touch the area you're about to work in.
- **`docs/project-management-support.md`** — project-management intake and wiki boundary, when work may become an issue or wiki note.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill creates them lazily when terms or decisions actually get resolved.

## File structure

single-context repo:

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-rust-cli-reads-artifacts.md
│   └── 0009-atomic-capability-execution-model.md
└── crates/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal — either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling` rather than inventing a parallel vocabulary).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0009 (atomic capability execution model) — but worth reopening because…_

## Obsidian/LLM Wiki

An Obsidian/LLM wiki may mirror or index these docs, artifact summaries, and handoff notes. It is a project-management knowledge surface, not artifact authority.

If wiki content conflicts with repo-local docs or artifact reports, prefer `CONTEXT.md`, ADRs, `summary.md`, `hospital.md`, `understanding.md`, and machine-readable artifact files until the source docs are deliberately updated.
