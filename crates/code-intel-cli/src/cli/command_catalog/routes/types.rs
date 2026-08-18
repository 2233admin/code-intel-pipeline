//! The route-catalog entry types. Split out of `routes.rs` (issue #155's
//! god-file remediation, same directory-module convention as
//! `change_risk/`/`change_agenda/`): four small type definitions on their
//! own do not need to sit in the same file as the ~700-line `COMMAND_ROUTES`
//! table and its resolver functions. Visibility stays `pub(in
//! crate::cli::command_catalog)` rather than the `pub(super)` these carried
//! in `routes.rs` -- `super` from here means `routes`, one level shallower
//! than before, so the explicit path keeps the reach identical: still
//! `cli::command_catalog` (and its `tests` submodule, which pattern-matches
//! `CommandRoute` and reads these fields) and no wider.

use super::super::contract::CommandContract;
use super::super::{CompatibilityRoute, LegacyRouteId};

pub(in crate::cli::command_catalog) struct RawRoute {
    pub(in crate::cli::command_catalog) command: &'static str,
    pub(in crate::cli::command_catalog) subcommand: Option<&'static str>,
    pub(in crate::cli::command_catalog) argument_offset: usize,
    pub(in crate::cli::command_catalog) id: CompatibilityRoute,
    #[allow(dead_code)]
    pub(in crate::cli::command_catalog) contract: CommandContract,
}

pub(in crate::cli::command_catalog) struct LegacyRoute {
    pub(in crate::cli::command_catalog) command: &'static str,
    pub(in crate::cli::command_catalog) aliases: &'static [&'static str],
    pub(in crate::cli::command_catalog) id: LegacyRouteId,
    #[allow(dead_code)]
    pub(in crate::cli::command_catalog) contract: CommandContract,
}

pub(in crate::cli::command_catalog) struct VersionRoute {
    pub(in crate::cli::command_catalog) command: &'static str,
    pub(in crate::cli::command_catalog) aliases: &'static [&'static str],
    pub(in crate::cli::command_catalog) contract: CommandContract,
}

pub(in crate::cli::command_catalog) enum CommandRoute {
    Version(VersionRoute),
    RunAlias(CommandContract),
    ProjectStatus(CommandContract),
    ProjectQuery(CommandContract),
    Primary(CommandContract),
    Raw(RawRoute),
    Legacy(LegacyRoute),
}
