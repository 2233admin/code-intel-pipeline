pub(super) const HELP_COMMAND: &str = "help";
pub(super) const HELP_ALIASES: &[&str] = &["--help", "-h"];

pub(super) fn is_help_spelling(spelling: &str) -> bool {
    spelling == HELP_COMMAND || HELP_ALIASES.contains(&spelling)
}

pub(super) fn is_help_flag(spelling: &str) -> bool {
    HELP_ALIASES.contains(&spelling)
}
