#[derive(Debug)]
pub(crate) enum AdapterError {
    InvalidOptions(String),
    Contract(String),
    Unavailable(String),
    Internal(String),
    Io(String),
}

pub(crate) struct AdapterOutput {
    pub(crate) artifacts: Vec<AdapterArtifact>,
    pub(crate) observed_effects: Vec<String>,
    pub(crate) domain_verdict: AdapterDomainVerdict,
    pub(crate) domain_failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdapterDomainVerdict {
    Pass,
    Fail,
    Unknown,
    NotApplicable,
}

impl AdapterDomainVerdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unknown => "unknown",
            Self::NotApplicable => "not_applicable",
        }
    }
}

pub(crate) struct AdapterArtifact {
    pub(crate) artifact_schema: String,
    pub(crate) artifact_type: String,
    pub(crate) relative_path: String,
    pub(crate) bytes: Vec<u8>,
}
