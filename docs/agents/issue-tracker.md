# Issue Tracker

Code Intel Pipeline uses Linear as the preferred project-management issue tracker for agent-planned work.

GitHub remains the source-code host for this repository. GitHub Issues, pull requests, and upstream repositories may still be evidence for `GitHub Solution Research`, but they are not the default project-management queue.

## Linear Usage

- Create Linear issues from verified plans, PRDs, hospital reports, or surgery plans only when the user requested external issue creation.
- Link Code Intel artifact files instead of copying scanner output wholesale.
- Include verification commands and stop conditions in each issue.
- Keep labels/statuses aligned with `docs/agents/triage-labels.md`.

## External Action Boundary

The scanner does not write to Linear. Code Intel Pipeline has no Linear runtime dependency.

Do not store Linear API keys, OAuth tokens, workspace IDs, or user secrets in this repository. Future Linear automation must read credentials from user-scoped environment or an approved secret store and require explicit authorization before creating or updating external issues.

## Pull Requests

External pull requests are not the default triage request surface. Treat pull requests as code-review or upstream-evidence inputs unless a workflow explicitly opts into PR triage.
