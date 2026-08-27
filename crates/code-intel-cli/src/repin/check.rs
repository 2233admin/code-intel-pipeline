//! Check-only entry point, split out of `repin/mod.rs` to keep that file
//! under the god-file threshold (issue #367 added this alongside
//! `code-intel verify`; see `super::run_raw` for the CLI-facing sibling this
//! composes without invoking).

use std::path::Path;

use serde_json::{json, Value};

use crate::declared_pins;

use super::{render_human, scan, RepinReport};

/// Check-only outcome of a repin scan: the exact fixpoint scan and
/// declared-pin audit `run_raw` performs before any `--write`, exposed as
/// data instead of printed CLI output. `code-intel verify` (issue #367)
/// composes this rather than invoking `run_raw`'s argv-and-print path or
/// reimplementing the scan. Never touches disk -- no `flush` call and no
/// `declared_pins::resync` call exist on this path.
pub(crate) struct CheckOutcome {
    pub(crate) ok: bool,
    pub(crate) json: Value,
    pub(crate) human: String,
}

pub(crate) fn run_check(repo: &Path) -> Result<CheckOutcome, String> {
    let report: RepinReport = scan(repo, &[])?;
    let declared = declared_pins::audit(repo)?;
    let declared_dirty = declared
        .iter()
        .any(declared_pins::PinFinding::needs_attention);
    let ok = report.is_clean() && !declared_dirty;
    let mut json = report.to_json(false);
    json["declaredPins"] = Value::Array(
        declared
            .iter()
            .filter(|finding| finding.needs_attention())
            .map(|finding| {
                let (state, actual) = match &finding.state {
                    declared_pins::PinState::Stale { actual } => ("stale", Some(actual.clone())),
                    declared_pins::PinState::SourceMissing => ("source_missing", None),
                    declared_pins::PinState::Ambiguous => ("ambiguous", None),
                    declared_pins::PinState::Fresh => ("fresh", None),
                };
                json!({
                    "record": finding.pin.record,
                    "path": finding.pin.path,
                    "declared": finding.pin.declared,
                    "state": state,
                    "actual": actual,
                })
            })
            .collect(),
    );
    // Same composition order `run_raw` prints in: declared-pin findings first,
    // then the pin-staleness summary (which can say "clean" on its own even
    // while a declared pin is stale -- exactly what `run_raw`'s own output
    // does today).
    let human = format!(
        "{}{}",
        declared_pins::render_findings(&declared, false),
        render_human(&report, false)
    );
    Ok(CheckOutcome { ok, json, human })
}
