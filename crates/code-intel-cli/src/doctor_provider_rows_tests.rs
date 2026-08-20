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
