#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunError {
    pub(crate) exit_code: i32,
    pub(crate) message: String,
}

impl RunError {
    pub(crate) fn contract(message: impl Into<String>) -> Self {
        Self {
            exit_code: 65,
            message: message.into(),
        }
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self {
            exit_code: 74,
            message: message.into(),
        }
    }
}
