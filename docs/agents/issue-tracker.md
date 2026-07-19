# Work Control Plane

Code Intel Pipeline does not require a SaaS issue tracker. The active control plane is selected per initiative, with this precedence:

1. an explicit user choice;
2. an existing Git-backed project Work-OS or repo-native task graph;
3. a hosted tracker already bound to the project;
4. a new hosted tracker only after explicit authorization.

Exactly one system owns mutable task state. GitHub, Gitea, Linear, and an LLM Wiki may link to one another, but must not all become bidirectional status authorities.

Record each initiative's selected task authority in its reviewed project record or ADR. The pipeline repository remains authority for code, contracts, tests, and generated scanner evidence.

## Hosted Tracker Usage

- Create hosted issues from verified plans, PRDs, hospital reports, or surgery plans only when the user requested external issue creation or the hosted tracker is already selected as authority.
- Link Code Intel artifact files instead of copying scanner output wholesale.
- Include verification commands and stop conditions in each issue.
- Keep labels/statuses aligned with `docs/agents/triage-labels.md` when the selected tracker uses them.

## External Action Boundary

The scanner does not write to hosted trackers. Code Intel Pipeline has no Linear runtime dependency and no GitHub Projects or Gitea Projects runtime dependency.

Do not store Linear API keys, other tracker credentials, OAuth tokens, workspace IDs, or user secrets in this repository. Hosted-tracker automation must read credentials from user-scoped environment or an approved secret store and require explicit authorization before creating or updating external issues.

## Pull Requests

External pull requests are not the default triage request surface. Treat pull requests as code-review or upstream-evidence inputs unless a workflow explicitly opts into PR triage.

## Local Work-OS Boundary

- Store logical project and issue identities in reviewed Markdown; keep machine-specific paths in local, ignored bindings.
- Use Git branches or pull requests as the promotion ledger.
- Keep boards, canvases, and indexes derived from issue notes; they are views, not alternate state stores.
- Never broad-stage a dirty vault. Stage only the exact project files owned by the active task.
