/// A run failure: the process exit code, the operator-facing message, and —
/// when the failure happened late enough that a real run manifest already
/// exists — a machine-readable envelope to print on stdout.
///
/// `report` exists because publication failures land *after* the DAG has done
/// the entire analysis. Exiting with only a stderr line throws that manifest
/// away and leaves the caller nothing to parse; carrying it here lets the CLI
/// seam emit it without every intermediate layer having to thread a second
/// return value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunError {
    pub(crate) exit_code: i32,
    pub(crate) message: String,
    pub(crate) report: Option<serde_json::Value>,
}

impl RunError {
    pub(crate) fn new(exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
            report: None,
        }
    }

    pub(crate) fn contract(message: impl Into<String>) -> Self {
        Self::new(65, message)
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self::new(74, message)
    }

    /// The requested publication name is already taken under the authority
    /// root. Distinct from `contract` (65) because the caller's arguments were
    /// well-formed and the run itself succeeded — only the destination was
    /// occupied — and distinct from `io` (74) because nothing is wrong with the
    /// host. 73 is `EX_CANTCREAT` in the sysexits vocabulary the rest of these
    /// codes follow: cannot create the output.
    pub(crate) fn publication_collision(message: impl Into<String>) -> Self {
        Self::new(73, message)
    }

    pub(crate) fn with_report(mut self, report: serde_json::Value) -> Self {
        self.report = Some(report);
        self
    }
}
