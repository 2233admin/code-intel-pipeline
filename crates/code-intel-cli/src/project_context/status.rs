impl super::ProjectContext {
    pub(crate) fn status(&self) -> Result<serde_json::Value, super::ProjectError> {
        if !self.artifact_root.is_dir() {
            return Ok(self.status_without_run(format!(
                "artifact root is not present: {}",
                self.artifact_root.display()
            )));
        }
        let freshness = match super::CommittedEvidenceController::freshness(
            crate::committed_evidence_controller::FreshnessRequest {
                artifact_root: self.artifact_root.clone(),
                repo: self.repo.clone(),
                repo_path: Some(self.repo_path.clone()),
            },
        ) {
            Ok(freshness) => freshness,
            Err(error) if is_unindexed(&error) => {
                return Ok(self.status_without_run(error_message(&error).to_string()));
            }
            Err(error) => return Err(super::ProjectError::from(error)),
        };
        Ok(self.status_with_run(freshness.value, &freshness.authority))
    }

    fn status_without_run(&self, reason: String) -> serde_json::Value {
        serde_json::json!({
            "schema": "code-intel-project-status.v1",
            "status": "needs_run",
            "project": self.project_identity(),
            "freshness": {
                "status": "unavailable",
                "recordedIdentity": serde_json::Value::Null,
                "currentIdentity": serde_json::Value::Null,
                "workingTreePolicy": serde_json::Value::Null,
                "scope": [],
            },
            "committedRun": serde_json::Value::Null,
            "reason": reason,
            "nextActions": [self.command_action(
                "analyze",
                "Analyze this project and publish the first committed run.",
                vec!["code-intel".into(), self.repo_path.display().to_string()],
            )],
        })
    }

    pub(super) fn status_with_run(
        &self,
        freshness: serde_json::Value,
        authority: &super::CommittedAuthority,
    ) -> serde_json::Value {
        let receipt = authority.receipt();
        let state = if freshness["status"] == "current" {
            "ready"
        } else {
            "stale"
        };
        serde_json::json!({
            "schema": "code-intel-project-status.v1",
            "status": state,
            "project": self.project_identity(),
            "freshness": freshness,
            "committedRun": {
                "repo": receipt.repo(),
                "run": receipt.run(),
                "runIdentity": receipt.run_identity(),
                "snapshotIdentity": receipt.snapshot_identity(),
                "authority": "committed",
            },
            "reason": serde_json::Value::Null,
            "nextActions": self.next_actions(state),
        })
    }

    fn project_identity(&self) -> serde_json::Value {
        serde_json::json!({
            "repo": self.repo,
            "path": self.repo_path,
        })
    }

    fn next_actions(&self, state: &str) -> serde_json::Value {
        let repo_path = self.repo_path.display().to_string();
        let artifact_root = self.artifact_root.display().to_string();
        let mut actions = Vec::new();
        if state == "stale" {
            actions.push(self.command_action(
                "refresh",
                "Refresh committed evidence before querying it as current.",
                vec!["code-intel".into(), repo_path.clone()],
            ));
        } else {
            actions.push(self.command_action(
                "context",
                "Read the bounded ranked code context for this project.",
                vec![
                    "code-intel".into(),
                    "query".into(),
                    repo_path.clone(),
                    "--kind".into(),
                    "evidence".into(),
                    "--type".into(),
                    "code_evidence.agent_slice".into(),
                    "--limit".into(),
                    "5".into(),
                    "--json".into(),
                ],
            ));
            actions.push(self.command_action(
                "query",
                "Query the verified committed artifacts with a bounded result.",
                vec![
                    "code-intel".into(),
                    "query".into(),
                    repo_path.clone(),
                    "--kind".into(),
                    "evidence".into(),
                    "--limit".into(),
                    "20".into(),
                    "--json".into(),
                ],
            ));
            actions.push(serde_json::json!({
                "id": "trace",
                "kind": "mcp_tool",
                "summary": "Trace one finding through its committed evidence chain.",
                "serverArgv": [
                    "code-intel", "serve", "--mcp", "--repo-path", repo_path.clone(),
                    "--repo", self.repo, "--artifact-root", artifact_root.clone(),
                ],
                "tool": "get_evidence",
            }));
        }
        actions.push(self.command_action(
            "impact",
            "Estimate blast radius and candidate tests from committed imports as advisory evidence.",
            vec![
                "code-intel".into(),
                "change".into(),
                "impact".into(),
                "--artifact-root".into(),
                artifact_root,
                "--repo".into(),
                self.repo.clone(),
                "--repo-path".into(),
                repo_path,
                "--changed".into(),
                "<path>".into(),
                "--staleness".into(),
                "advisory".into(),
            ],
        ));
        serde_json::Value::Array(actions)
    }

    fn command_action(&self, id: &str, summary: &str, argv: Vec<String>) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "kind": "command",
            "summary": summary,
            "argv": argv,
        })
    }
}

fn is_unindexed(error: &crate::committed_evidence::EvidenceError) -> bool {
    matches!(
        error,
        crate::committed_evidence::EvidenceError::Contract(message)
            if message.starts_with("no committed authoritative run is indexed for repository:")
    )
}

fn error_message(error: &crate::committed_evidence::EvidenceError) -> &str {
    match error {
        crate::committed_evidence::EvidenceError::Contract(message)
        | crate::committed_evidence::EvidenceError::HostIo(message) => message,
    }
}
