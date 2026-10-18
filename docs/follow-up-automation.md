# Follow-up automation

This is a zero-effect advisory artifact (`follow-up-automation.json`), not proof that a skill ran or a pull request was created. The compiled primary entry (`code-intel`) does not emit it yet; primary-entry support is future work.

## Proactive skill suggestions

`followUpAutomation.proactiveSkillSuggestions.enabled` defaults to `true`. An actionable local-tool failure, an unclassified failed step, or blocking Sentrux debt proposes `/investigate`. Provider quota, provider availability, configuration, and missing-graph failures are not mislabeled as code bugs. Suggestions are proposal-only and always carry `effects: []`.

Disable or change the suggestion in `pipeline.config.json`:

```json
{
  "followUpAutomation": {
    "proactiveSkillSuggestions": {
      "enabled": false,
      "bugSkill": "/investigate"
    }
  }
}
```

The compiled primary entry does not yet read `followUpAutomation` from
`pipeline.config.json` or accept its flags. Primary-entry support is future work.

## Automatic pull requests

Automatic PR mode defaults to `ask`. When an actionable problem exists, the Pipeline writes `automatic-pr-consent.request.json`, prints the question, and records `consentStatus: pending`. Ordinary scanning and reporting still complete; only `automatic_pr_execution` remains unauthorized.

The choices are:

- `keep_disabled`: no PR may be created;
- `enable_once_for_snapshot`: enter the exact-proposal flow. The executor runs only after proposal-specific consent, C07 replay, snapshot, HEAD, repository-mutation, and network checks all pass.

`enabled` means the operator requested the execution path; it is not sufficient authority by itself. The execution atom remains fail-closed until it receives scoped authorization artifacts and both runtime effect switches. `disabled` emits neither a consent request nor an external effect.

The core Pipeline never calls `gh pr create` from the advisory path. It does not listen to Codex, Claude, or OpenCode chat messages directly; a host that wants chat-triggered suggestions must submit normalized bug evidence to the Pipeline.
The automatic-PR question is a feature-flow opt-in, not authority to publish a particular pull
request. A concrete draft PR requires a separately hashed canonical proposal and replay-valid C07
Decision Record as documented in `automatic-pull-request-beta.md`.
