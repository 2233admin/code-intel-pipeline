mod command_catalog;
mod help_contract;
mod legacy;
mod primary;
mod project_query;

pub(crate) use command_catalog::RenderedOutcome;

/// The process-facing CLI seam. Callers provide argv and receive one rendered
/// result; parsing, execution, compatibility routing, and rendering stay
/// private to the CLI module.
///
/// During the Phase-2 migration, compatibility controllers still write some
/// output directly to the process streams. Those paths return
/// `CompatibilityAlreadyEmitted`, so their `RenderedOutcome` intentionally
/// carries empty stdout/stderr strings even though bytes were already emitted.
/// Later phases can remove this transitional side effect family by family.
pub(crate) fn run(raw: &[String]) -> RenderedOutcome {
    let result = command_catalog::parse_command(raw).and_then(command_catalog::execute_command);
    command_catalog::render_outcome(result)
}
