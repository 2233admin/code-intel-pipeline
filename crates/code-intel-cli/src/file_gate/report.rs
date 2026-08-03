//! The per-run decision record: [`GateReport`], the fail-closed identity
//! check (issue #152 requirement 3), and the JSON contract both `sentrux
//! scan` and `sentrux dsm` embed verbatim (requirement 1, requirement 4).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::rules::{Decision, GateDecision, GATE_ORDER};

impl GateDecision {
    fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "decision": self.decision.as_str(),
            "gate": self.gate,
            "source": self.source,
        })
    }
}

/// The full per-candidate decision record for one [`super::evaluate`] run.
pub(crate) struct GateReport {
    pub(crate) candidates: i64,
    /// Sorted relative paths with `decision == Included`.
    pub(crate) included: Vec<String>,
    /// One entry per candidate, in candidate-sorted order.
    pub(crate) decisions: Vec<GateDecision>,
}

impl GateReport {
    /// `candidates == included + sum(excluded by gate)` (issue #152
    /// requirement 3), checked structurally: every candidate must have
    /// produced exactly one decision, and the decisions must partition
    /// exactly into the `included` list and the excluded remainder. A
    /// mismatch is a coding defect in this module (a candidate silently
    /// skipped, one double-recorded), and it fails the caller's command
    /// outright rather than emitting a quietly wrong count.
    pub(crate) fn verify_identity(&self) -> Result<(), String> {
        if self.decisions.len() as i64 != self.candidates {
            return Err(format!(
                "sentrux file-gate identity violated: {} candidates walked but {} decisions recorded",
                self.candidates,
                self.decisions.len()
            ));
        }
        let included_count = self
            .decisions
            .iter()
            .filter(|decision| decision.decision == Decision::Included)
            .count() as i64;
        let excluded_count = self.decisions.len() as i64 - included_count;
        if included_count != self.included.len() as i64 {
            return Err(format!(
                "sentrux file-gate identity violated: {included_count} decisions marked included but {} paths in the included set",
                self.included.len()
            ));
        }
        if self.candidates != included_count + excluded_count {
            return Err(format!(
                "sentrux file-gate identity violated: candidates {} != included {included_count} + excluded {excluded_count}",
                self.candidates
            ));
        }
        Ok(())
    }

    /// Per-gate rollup in [`GATE_ORDER`], each carrying the `source` layer
    /// that produced it (issue #152 requirement 5). Gates with zero matches
    /// this run are omitted rather than printed as a hollow `0`.
    pub(crate) fn by_gate(&self) -> Vec<Value> {
        let mut counts: BTreeMap<&'static str, (i64, &'static str, &'static str)> = BTreeMap::new();
        for decision in &self.decisions {
            let entry = counts.entry(decision.gate).or_insert((
                0,
                decision.source,
                decision.decision.as_str(),
            ));
            entry.0 += 1;
        }
        GATE_ORDER
            .iter()
            .filter_map(|gate| {
                counts.get(gate).map(|(files, source, decision)| {
                    json!({
                        "gate": gate,
                        "source": source,
                        "decision": decision,
                        "files": files,
                    })
                })
            })
            .collect()
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "schema": "code-intel-file-gate.v1",
            "gate_order": GATE_ORDER,
            "candidates": self.candidates,
            "included": self.included.len() as i64,
            "excluded": self.candidates - self.included.len() as i64,
            "by_gate": self.by_gate(),
            "decisions": self.decisions.iter().map(GateDecision::to_json).collect::<Vec<_>>(),
        })
    }
}
