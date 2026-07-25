// ---------------------------------------------------------------------
// Enums. `as_str` always returns the schema's lowercase wire value;
// `parse` is its exact inverse.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "critical" => Some(Self::Critical),
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            "info" => Some(Self::Info),
            _ => None,
        }
    }

    /// Ascending rank for "severity desc" sorts: 0 is the most severe.
    pub(super) fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
            Self::Info => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DepartmentRunStatus {
    Assessed,
    NotAssessed,
    Disabled,
}

impl DepartmentRunStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Assessed => "assessed",
            Self::NotAssessed => "not_assessed",
            Self::Disabled => "disabled",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "assessed" => Some(Self::Assessed),
            "not_assessed" => Some(Self::NotAssessed),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applicable {
    Yes,
    No,
    Unknown,
}

impl Applicable {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingStatus {
    Confirmed,
    Suspected,
}

impl FindingStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Suspected => "suspected",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "confirmed" => Some(Self::Confirmed),
            "suspected" => Some(Self::Suspected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EstimatedEffort {
    Minutes,
    Hours,
    Days,
}

impl EstimatedEffort {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "minutes" => Some(Self::Minutes),
            "hours" => Some(Self::Hours),
            "days" => Some(Self::Days),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceKind {
    File,
    ModalitySignal,
    Command,
    ManualRead,
}

impl EvidenceKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::ModalitySignal => "modality_signal",
            Self::Command => "command",
            Self::ManualRead => "manual_read",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "modality_signal" => Some(Self::ModalitySignal),
            "command" => Some(Self::Command),
            "manual_read" => Some(Self::ManualRead),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Modality {
    Xray,
    Anatomy,
    Ct,
    Mri,
    Pet,
    Chart,
    Governance,
}

impl Modality {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Xray => "xray",
            Self::Anatomy => "anatomy",
            Self::Ct => "ct",
            Self::Mri => "mri",
            Self::Pet => "pet",
            Self::Chart => "chart",
            Self::Governance => "governance",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "xray" => Some(Self::Xray),
            "anatomy" => Some(Self::Anatomy),
            "ct" => Some(Self::Ct),
            "mri" => Some(Self::Mri),
            "pet" => Some(Self::Pet),
            "chart" => Some(Self::Chart),
            "governance" => Some(Self::Governance),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coverage {
    High,
    Medium,
    Low,
    NotAssessed,
}

impl Coverage {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::NotAssessed => "not_assessed",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            "not_assessed" => Some(Self::NotAssessed),
            _ => None,
        }
    }
}

/// The `scope.kind` discriminator (see `docs/audit-report.md`'s incremental
/// audits section): `full` sweeps the whole tree with no path restriction;
/// `diff` is bounded to `scope.files`, enforced by `AuditReport::validate()`
/// rule (j).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Full,
    Diff,
}

impl ScopeKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Diff => "diff",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "diff" => Some(Self::Diff),
            _ => None,
        }
    }
}
