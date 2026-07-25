# Optional Compete project score

This adapter turns a completed [lbj96347/compete](https://github.com/lbj96347/compete) research run into an advisory Code Intel score. It does not affect Sentrux gates, hospital score, discharge state, or default pipeline execution.

Prepare an Agent task:

```powershell
& "$env:CODE_INTEL_HOME/Invoke-CompeteProjectScore.ps1" `
  -Action prepare `
  -RepoPath <repo-path> `
  -ArtifactRoot <artifact-directory> `
  -CompeteRoot <compete-clone-or-skill-root>
```

The command writes `competitive-intelligence-request.json` and `competitive-intelligence-prompt.md`. An Orca-managed Claude/Codex terminal can consume the prompt; the Agent must have web research access because Orca coordinates execution but does not perform competitive research itself.

After the Agent writes the normal `compete` datasets, normalize its existing six-axis scoring logic:

```powershell
& "$env:CODE_INTEL_HOME/Invoke-CompeteProjectScore.ps1" `
  -Action score `
  -RepoPath <repo-path> `
  -ArtifactRoot <artifact-directory> `
  -CompeteRoot <compete-clone-or-skill-root> `
  -CompeteDataPath <directory-with-compete-json-datasets>
```

The result is `competitive-score.json` with the six upstream axes and their arithmetic mean. Review the InsightKit report and its confidence/provenance before acting on the score.
