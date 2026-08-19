//! Provider presence/readiness/conformance rows, derived from a doctor
//! bootstrap document alone.
//!
//! This lives in its own module because it now has two callers with very
//! different shapes: the DAG doctor node, which publishes the rows inside a
//! `code-intel-doctor-observation.v1`, and `doctor bootstrap
//! --require-provider-conformance`, which only wants the verdict. Restating
//! the predicate in the second caller would have been the cheaper edit and
//! the wrong one — two copies of a governance rule drift, and the drift is
//! invisible until a machine disagrees with CI about whether an installation
//! is sound.
//!
//! The rows are also the reason CI's post-install doctor step exists at all.
//! `bootstrap` reports `ok` while a *present* external provider is broken,
//! because the built-in engines make an external one optional. The strict
//! reading — present must mean conforming — was reachable only through the
//! doctor node, which CI runs before installing anything, so the branch every
//! installed developer machine takes had no coverage.

use serde_json::{json, Value};

/// Local JSON accessors. `doctor_adapter` has identical two-line helpers, but
/// importing them would create a module import cycle between these two files,
/// and `.sentrux/rules.toml` gates `max_cycles = 0` for good reasons.
fn string<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or_default()
}

fn boolean(value: &Value, field: &str) -> bool {
    value[field].as_bool().unwrap_or(false)
}

pub(crate) fn provider_rows(raw: &Value) -> Vec<Value> {
    let empty = Vec::new();
    let tools = raw
        .pointer("/checks/tools")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let tool_present = |name: &str| {
        tools
            .iter()
            .any(|tool| string(tool, "name") == name && boolean(tool, "found"))
    };
    let sentrux_core = raw
        .pointer("/checks/sentrux/core/found")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sentrux_pro = raw
        .pointer("/checks/sentrux/pro/found")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sentrux_builtin = raw
        .pointer("/checks/sentrux/builtin/found")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // The built-in engine makes an external Sentrux optional, but a PRESENT
    // external installation must still conform: a broken overlay on PATH is
    // exactly the verdict-drift hazard this doctor exists to surface.
    let external_sentrux = tool_present("sentrux");
    let sentrux_ready = (sentrux_builtin && !external_sentrux) || (sentrux_core && sentrux_pro);
    let graph_source = raw
        .pointer("/checks/graphProvider/sourceFound")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let graph_cargo = raw
        .pointer("/checks/graphProvider/cargoFound")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let graph_binary = raw
        .pointer("/checks/graphProvider/binaryFound")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut rows = vec![
        json!({"id":"repowise","presence":if tool_present("repowise") {"present"} else {"missing"},"readiness":if tool_present("repowise") {"ready"} else {"unavailable"},"conformance":"not_evaluated","admissibility":"not_evaluated"}),
        json!({"id":"sentrux","presence":if sentrux_builtin || tool_present("sentrux") {"present"} else {"missing"},"readiness":if sentrux_ready {"ready"} else {"unavailable"},"conformance":if sentrux_ready {"conforming"} else if tool_present("sentrux") {"nonconforming"} else {"not_evaluated"},"admissibility":"not_evaluated"}),
        json!({"id":"graph.code-intel","presence":if graph_source && graph_cargo {"present"} else {"missing"},"readiness":if graph_source && graph_cargo && graph_binary {"ready"} else {"unavailable"},"conformance":if graph_source && graph_cargo {"conforming"} else {"not_evaluated"},"admissibility":"not_evaluated"}),
    ];
    // weco (#300): optional external perf-optimize engine, PATH presence plus
    // BYOK auth. The human-readable reason for "unavailable" lives in
    // `checks.weco.reason` (doctor_bootstrap::mod.rs), not here — this row
    // feeds the `providerObservation` schema
    // (orchestration/schemas/code-intel-doctor-observation.v1.schema.json),
    // which sets `additionalProperties: false` and has no `reason` property.
    // presence + readiness already disambiguate the two "unavailable" causes
    // for any caller that wants to derive one.
    let weco_present = tool_present("weco");
    let weco_byok = raw
        .pointer("/checks/weco/byokConfigured")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let weco_ready = weco_present && weco_byok;
    rows.push(json!({
        "id":"weco",
        "presence":if weco_present {"present"} else {"missing"},
        "readiness":if weco_ready {"ready"} else {"unavailable"},
        "conformance":"not_evaluated",
        "admissibility":"not_evaluated"
    }));
    // Assistance candidates the catalog routes to. Presence is the only claim
    // made: nothing here reads the plugin's contents, so a present one stays
    // `not_evaluated` rather than `conforming`, and an absent one can never
    // make `--require-provider-conformance` fail. An uninstalled routing
    // target is a fact the operator should see, not a broken installation.
    rows.extend(
        raw.pointer("/checks/assistancePlugins")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .map(|plugin| {
                let found = boolean(plugin, "found");
                json!({
                    "id": format!("assistance:{}", string(plugin, "candidateId")),
                    "presence": if found {"present"} else {"missing"},
                    "readiness": if found {"ready"} else {"unavailable"},
                    "conformance": "not_evaluated",
                    "admissibility": "not_evaluated"
                })
            }),
    );
    rows
}

/// Provider ids a doctor bootstrap document reports as `nonconforming`.
pub(crate) fn nonconforming_providers(raw: &Value) -> Vec<String> {
    provider_rows(raw)
        .into_iter()
        .filter(|provider| provider["conformance"] == "nonconforming")
        .map(|provider| provider["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bootstrap(builtin: bool, external: bool, core: bool, pro: bool) -> Value {
        json!({
            "checks": {
                "tools": [
                    {"name": "sentrux", "required": false, "found": external},
                ],
                "sentrux": {
                    "builtin": {"found": builtin},
                    "core": {"found": core},
                    "pro": {"found": pro},
                },
            }
        })
    }

    #[test]
    fn a_present_but_broken_overlay_is_nonconforming_even_with_the_builtin_engine() {
        // The exact shape a stale installed shim produces: the built-in
        // engine is fine, `sentrux` resolves on PATH, and both probes fail.
        let raw = bootstrap(true, true, false, false);
        assert_eq!(nonconforming_providers(&raw), vec!["sentrux".to_string()]);
    }

    #[test]
    fn builtin_alone_and_a_working_overlay_are_both_conformant() {
        // CI before the install step: no external sentrux anywhere.
        assert!(nonconforming_providers(&bootstrap(true, false, false, false)).is_empty());
        // An installed machine with a working overlay.
        assert!(nonconforming_providers(&bootstrap(true, true, true, true)).is_empty());
    }

    #[test]
    fn an_absent_provider_is_not_evaluated_rather_than_nonconforming() {
        let raw = bootstrap(false, false, false, false);
        let rows = provider_rows(&raw);
        let sentrux = rows
            .iter()
            .find(|row| row["id"] == "sentrux")
            .expect("sentrux row");
        assert_eq!(sentrux["presence"], "missing");
        assert_eq!(sentrux["conformance"], "not_evaluated");
        assert!(nonconforming_providers(&raw).is_empty());
    }

    fn bootstrap_weco(tool_found: bool, byok_configured: bool) -> Value {
        json!({
            "checks": {
                "tools": [
                    {"name": "weco", "required": false, "found": tool_found},
                ],
                "weco": {"byokConfigured": byok_configured},
            }
        })
    }

    fn weco_row(raw: &Value) -> Value {
        provider_rows(raw)
            .into_iter()
            .find(|row| row["id"] == "weco")
            .expect("weco row")
    }

    #[test]
    fn weco_reports_missing_when_not_on_path() {
        let row = weco_row(&bootstrap_weco(false, false));
        assert_eq!(row["presence"], "missing");
        assert_eq!(row["readiness"], "unavailable");
    }

    /// Installed-but-unauthenticated must stay distinguishable from
    /// not-installed via presence+readiness alone (#300) — an operator who
    /// ran `pipx install weco` needs `presence:"present"` to tell them the
    /// binary was found, even though the row itself carries no free-text
    /// reason (see the `reason` comment above `provider_rows` — that string
    /// lives in `checks.weco.reason`, not this schema-constrained row).
    #[test]
    fn weco_reports_present_but_unauthenticated_distinctly_from_missing() {
        let row = weco_row(&bootstrap_weco(true, false));
        assert_eq!(row["presence"], "present");
        assert_eq!(row["readiness"], "unavailable");
    }

    #[test]
    fn weco_is_ready_when_present_and_authenticated() {
        let row = weco_row(&bootstrap_weco(true, true));
        assert_eq!(row["presence"], "present");
        assert_eq!(row["readiness"], "ready");
    }

    /// #300 code review: the providerObservation schema
    /// (code-intel-doctor-observation.v1.schema.json) sets
    /// `additionalProperties: false` on exactly
    /// `[id,presence,readiness,conformance,admissibility]` — a `reason` field
    /// here would fail schema validation even though nothing in
    /// `crates/code-intel-cli/tests/` currently exercises that check.
    #[test]
    fn weco_row_carries_no_field_outside_the_provider_observation_schema() {
        let row = weco_row(&bootstrap_weco(true, false));
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "admissibility",
                "conformance",
                "id",
                "presence",
                "readiness"
            ]
        );
    }
}
