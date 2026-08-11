use serde_json::{json, Value};

use crate::change_impact;
use crate::committed_evidence::EvidenceError;
use crate::evidence_query::QueryError;
use crate::run_error;

#[derive(Debug)]
pub(crate) struct ProjectError {
    exit_code: i32,
    message: String,
}

impl ProjectError {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: 64,
            message: message.into(),
        }
    }

    pub(crate) fn contract(message: impl Into<String>) -> Self {
        Self {
            exit_code: 65,
            message: message.into(),
        }
    }

    pub(crate) fn host_io(message: impl Into<String>) -> Self {
        Self {
            exit_code: 74,
            message: message.into(),
        }
    }

    pub(super) fn from_run(error: run_error::RunError) -> Self {
        Self {
            exit_code: error.exit_code,
            message: error.message,
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self.exit_code {
            64 => "usage",
            65 => "contract",
            74 => "host_io",
            _ => "execution",
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn to_value(&self) -> Value {
        json!({
            "schema": "code-intel-project-error.v1",
            "outcome": "error",
            "kind": self.kind(),
            "exitCode": self.exit_code,
            "diagnostic": self.message,
        })
    }
}

impl From<EvidenceError> for ProjectError {
    fn from(error: EvidenceError) -> Self {
        match error {
            EvidenceError::Contract(message) => Self::contract(message),
            EvidenceError::HostIo(message) => Self::host_io(message),
        }
    }
}

impl From<QueryError> for ProjectError {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::Contract(message) => Self::contract(message),
            QueryError::HostIo(message) => Self::host_io(message),
        }
    }
}

impl From<change_impact::ImpactError> for ProjectError {
    fn from(error: change_impact::ImpactError) -> Self {
        match error {
            change_impact::ImpactError::Contract(message) => Self::contract(message),
            change_impact::ImpactError::HostIo(message) => Self::host_io(message),
        }
    }
}
