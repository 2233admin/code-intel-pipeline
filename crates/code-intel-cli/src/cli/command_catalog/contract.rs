#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum CommandStability {
    Public,
    Compatibility,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum ControllerOwnership {
    AuthoritativeRun,
    CommittedEvidence,
    WorkspaceAdvisory,
    ProviderAdmin,
    Governance,
    AgentSession,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum CommandAuthority {
    Committed,
    Advisory,
    Administrative,
    Internal,
    Conditional(AuthorityCondition),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum AuthorityCondition {
    CommittedOrStaleAdvisory,
    CommittedWhenPresent,
}

/// Mirrors the closed `effect` vocabulary in
/// `code-intel-capability-envelope.v1.schema.json`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)]
pub(super) enum CommandEffect {
    RepoRead,
    LocalWrite,
    ProcessSpawn,
    Network,
    RepoMutation,
}

impl CommandEffect {
    #[cfg(test)]
    fn as_str(self) -> &'static str {
        match self {
            Self::RepoRead => "repo_read",
            Self::LocalWrite => "local_write",
            Self::ProcessSpawn => "process_spawn",
            Self::Network => "network",
            Self::RepoMutation => "repo_mutation",
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(super) struct CommandContract {
    pub(super) stability: CommandStability,
    pub(super) controller: ControllerOwnership,
    pub(super) authority: CommandAuthority,
    pub(super) effects: &'static [CommandEffect],
    pub(super) output_contract: OutputContract,
    pub(super) exit_contract: ExitContract,
    pub(super) retirement_condition: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutputContract {
    Stdout {
        identities: &'static [&'static str],
    },
    ArtifactFiles {
        identities: &'static [&'static str],
    },
    ArtifactFilesAndStdout {
        artifact_identities: &'static [&'static str],
        stdout_identities: &'static [&'static str],
    },
    StdoutWithOptionalArtifactFiles {
        artifact_identities: &'static [&'static str],
        stdout_identities: &'static [&'static str],
    },
}

impl OutputContract {
    #[cfg(test)]
    pub(super) fn assert_declared_formats(self) {
        match self {
            Self::Stdout { identities } | Self::ArtifactFiles { identities } => {
                assert_format_declarations(identities)
            }
            Self::ArtifactFilesAndStdout {
                artifact_identities,
                stdout_identities,
            }
            | Self::StdoutWithOptionalArtifactFiles {
                artifact_identities,
                stdout_identities,
            } => {
                assert_format_declarations(artifact_identities);
                assert_format_declarations(stdout_identities);
            }
        }
    }
}

#[cfg(test)]
fn assert_format_declarations(identities: &[&str]) {
    assert!(!identities.is_empty(), "output formats must be explicit");
    let mut seen = Vec::new();
    for identity in identities {
        assert!(
            identity.starts_with("code-")
                || identity.starts_with("text-format:")
                || identity.starts_with("artifact-format:"),
            "output identity must name a schema or a versioned format: {identity}"
        );
        assert!(
            !seen.contains(identity),
            "duplicate output identity: {identity}"
        );
        seen.push(*identity);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExitContract {
    Exact(&'static [i32]),
}

impl ExitContract {
    #[cfg(test)]
    pub(super) fn assert_concrete(self) {
        let Self::Exact(codes) = self;
        assert_eq!(codes.first(), Some(&0), "success exit 0 must be explicit");
        assert!(
            codes.windows(2).all(|pair| pair[0] < pair[1]),
            "exit codes must be sorted and unique: {codes:?}"
        );
    }

    #[cfg(test)]
    pub(super) fn codes(self) -> &'static [i32] {
        let Self::Exact(codes) = self;
        codes
    }
}

impl CommandContract {
    #[cfg(test)]
    pub(super) fn assert_complete(&self, spelling: &str) {
        let _classification = (self.stability, self.controller, self.authority);
        self.output_contract.assert_declared_formats();
        self.exit_contract.assert_concrete();
        assert!(
            !self.retirement_condition.is_empty(),
            "{spelling}: retirement condition"
        );
        let mut effects = Vec::new();
        for effect in self.effects {
            assert!(
                !effects.contains(&effect.as_str()),
                "{spelling}: duplicate effect"
            );
            effects.push(effect.as_str());
            assert!(matches!(
                effect.as_str(),
                "repo_read" | "local_write" | "process_spawn" | "network" | "repo_mutation"
            ));
        }
    }
}
