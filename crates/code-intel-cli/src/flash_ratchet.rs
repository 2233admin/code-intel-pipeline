// Measurement method from huashu-flash (MIT, Copyright 2026 花叔 Huashu):
// failed attempts stay out of the percentiles, the ratchet tolerance is five
// percent, metrics only improve by getting smaller, and a paired reduction
// requires an alternating collection order.
// https://github.com/alchaincyf/huashu-flash

use crate::adapter_contract::{AdapterArtifact, AdapterDomainVerdict, AdapterError, AdapterOutput};
use crate::artifact_ref::VerifiedArtifact;
use serde_json::{json, Value};

const SCHEMA: &str = "code-intel-flash-ratchet.v1";
const SAMPLES_SCHEMA: &str = "code-intel-flash-samples.v1";
const CEILING_SCHEMA: &str = "code-intel-flash-ratchet-ceiling.v1";
const MIN_SAMPLES: usize = 10;

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &std::path::Path,
) -> Result<AdapterOutput, AdapterError> {
    if !request["options"]
        .as_object()
        .is_some_and(|options| options.is_empty())
    {
        return Err(AdapterError::InvalidOptions(
            "measurement.flash-ratchet accepts no options".into(),
        ));
    }
    let (samples, ceiling) = inputs(request, verified_inputs)?;
    let report = evaluate(samples, ceiling)?;
    let verdict = if report["ratchet"]["state"] == "regressed" {
        AdapterDomainVerdict::Fail
    } else {
        AdapterDomainVerdict::Pass
    };
    let failure = (verdict == AdapterDomainVerdict::Fail).then(|| {
        format!(
            "p75 {} is worse than ceiling {} by more than the recorded tolerance",
            report["primary"]["p75"], report["ratchet"]["submitted"]
        )
    });
    let bytes = serde_json::to_vec(&report).map_err(|error| {
        AdapterError::Internal(format!("serialize flash ratchet report: {error}"))
    })?;
    publish(out, "flash-ratchet.json", &bytes)?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: SCHEMA.into(),
            artifact_type: "measurement.flash-ratchet-report".into(),
            relative_path: "flash-ratchet.json".into(),
            bytes,
        }],
        observed_effects: vec!["local_write".into()],
        domain_verdict: verdict,
        domain_failure: failure,
    })
}

fn inputs<'a>(
    request: &Value,
    verified: &'a [VerifiedArtifact],
) -> Result<(&'a VerifiedArtifact, Option<&'a VerifiedArtifact>), AdapterError> {
    let refs = request["inputs"].as_array().ok_or_else(|| {
        AdapterError::Contract("capability request inputs must be an array".into())
    })?;
    if refs.len() != verified.len() || !(1..=2).contains(&verified.len()) {
        return Err(AdapterError::Contract(
            "measurement.flash-ratchet requires samples and an optional ceiling".into(),
        ));
    }
    let mut samples = None;
    let mut ceiling = None;
    for artifact in verified {
        match artifact.artifact_schema() {
            SAMPLES_SCHEMA => samples = Some(artifact),
            CEILING_SCHEMA => ceiling = Some(artifact),
            other => {
                return Err(AdapterError::Contract(format!(
                    "unexpected input schema {other}"
                )))
            }
        }
    }
    let samples = samples.ok_or_else(|| {
        AdapterError::Contract("measurement.flash-ratchet requires a sample artifact".into())
    })?;
    Ok((samples, ceiling))
}

fn evaluate(
    samples: &VerifiedArtifact,
    ceiling: Option<&VerifiedArtifact>,
) -> Result<Value, AdapterError> {
    let body: Value = serde_json::from_slice(samples.bytes())
        .map_err(|error| AdapterError::Contract(format!("parse flash samples: {error}")))?;
    require_present(
        &body,
        &["schema", "operation", "metric", "primary"],
        "samples",
    )?;
    if body.as_object().is_some_and(|object| object.len() > 5) {
        return Err(AdapterError::Contract(
            "samples has an unknown field".into(),
        ));
    }
    if body["schema"] != SAMPLES_SCHEMA {
        return Err(AdapterError::Contract("sample schema mismatch".into()));
    }
    let operation = text(&body, "operation")?;
    let metric = text(&body, "metric")?;
    let primary = group(&body["primary"], "primary")?;
    let paired = match body.get("paired") {
        Some(value) => Some(paired(value, &primary)?),
        None => None,
    };
    let ratchet = ratchet(operation, metric, primary.p75, ceiling)?;
    Ok(json!({
        "schema": SCHEMA,
        "authority": "derived_measurement_no_publish_authority",
        "operation": operation,
        "metric": metric,
        "direction": "lower",
        "primary": primary.summary(),
        "paired": paired,
        "ratchet": ratchet,
        "limitations": [
            "The pipeline did not run the operation or collect the samples.",
            "A passing ratchet does not authorize publication or deployment."
        ]
    }))
}

struct Group {
    ok: Vec<f64>,
    failed: usize,
    p75: f64,
}
impl Group {
    fn summary(&self) -> Value {
        json!({
            "samples": self.ok.len(),
            "failed": self.failed,
            "p50": percentile(&self.ok, 50),
            "p75": percentile(&self.ok, 75),
            "p95": percentile(&self.ok, 95),
        })
    }
}

fn group(value: &Value, label: &str) -> Result<Group, AdapterError> {
    let attempts = value
        .as_array()
        .ok_or_else(|| AdapterError::Contract(format!("{label} must be an array of attempts")))?;
    let mut ok = Vec::new();
    let mut failed = 0;
    for attempt in attempts {
        match attempt.get("ok") {
            Some(Value::Bool(true)) => {
                let sample = attempt
                    .get("value")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite());
                let sample = sample.ok_or_else(|| {
                    AdapterError::Contract(format!("{label} ok attempt needs a finite value"))
                })?;
                ok.push(sample);
            }
            Some(Value::Bool(false)) => {
                text(attempt, "reason").map_err(|_| {
                    AdapterError::Contract(format!("{label} failed attempt needs a reason"))
                })?;
                failed += 1;
            }
            _ => {
                return Err(AdapterError::Contract(format!(
                    "{label} attempt needs boolean ok"
                )))
            }
        }
    }
    if ok.len() < MIN_SAMPLES {
        return Err(AdapterError::Contract(format!(
            "{label} needs at least {MIN_SAMPLES} finite samples, found {}",
            ok.len()
        )));
    }
    ok.sort_by(f64::total_cmp);
    let p75 = percentile(&ok, 75);
    Ok(Group { ok, failed, p75 })
}

fn paired(value: &Value, primary: &Group) -> Result<Value, AdapterError> {
    require_present(value, &["attempts", "order"], "paired")?;
    let other = group(&value["attempts"], "paired")?;
    let order = value["order"]
        .as_array()
        .ok_or_else(|| AdapterError::Contract("paired order must be an array".into()))?;
    let expected = primary.ok.len() + other.ok.len() + other.failed + primary.failed;
    if order.len() != expected {
        return Err(AdapterError::Contract(
            "paired order must name every attempt".into(),
        ));
    }
    if !alternates(order) {
        return Err(AdapterError::Contract(
            "paired order must strictly alternate".into(),
        ));
    }
    let reduction = (primary.p75 - other.p75) / primary.p75;
    Ok(json!({
        "summary": other.summary(),
        "p75Reduction": round1(reduction),
    }))
}

fn alternates(order: &[Value]) -> bool {
    let tags: Vec<&str> = order.iter().filter_map(Value::as_str).collect();
    if tags.len() != order.len() || tags.len() < 2 {
        return false;
    }
    let (first, second) = (tags[0], tags[1]);
    (first == "A" && second == "B" || first == "B" && second == "A")
        && tags.windows(2).all(|pair| pair[0] != pair[1])
}

fn ratchet(
    operation: &str,
    metric: &str,
    p75: f64,
    prior: Option<&VerifiedArtifact>,
) -> Result<Value, AdapterError> {
    let Some(prior) = prior else {
        return Ok(json!({
            "state": "initialized",
            "tolerance": 0.05,
            "ceiling": ceiling(operation, metric, p75),
        }));
    };
    let body: Value = serde_json::from_slice(prior.bytes())
        .map_err(|error| AdapterError::Contract(format!("parse ratchet ceiling: {error}")))?;
    require_present(
        &body,
        &[
            "schema",
            "operation",
            "metric",
            "direction",
            "tolerance",
            "p75",
        ],
        "ceiling",
    )?;
    if body["schema"] != CEILING_SCHEMA
        || body["operation"] != operation
        || body["metric"] != metric
        || body["direction"] != "lower"
    {
        return Err(AdapterError::Contract(
            "ceiling does not match the submitted operation and metric".into(),
        ));
    }
    let tolerance = body["tolerance"]
        .as_f64()
        .filter(|value| *value == 0.05)
        .ok_or_else(|| {
            AdapterError::Contract("ceiling tolerance must be the recorded 0.05".into())
        })?;
    let limit = body["p75"]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| AdapterError::Contract("ceiling p75 must be finite".into()))?;
    let state = if p75 > limit * (1.0 + tolerance) {
        "regressed"
    } else if p75 < limit {
        "tightened"
    } else {
        "held"
    };
    let next = if state == "tightened" { p75 } else { limit };
    Ok(json!({
        "state": state,
        "tolerance": tolerance,
        "submitted": limit,
        "ceiling": ceiling(operation, metric, next),
    }))
}

fn ceiling(operation: &str, metric: &str, p75: f64) -> Value {
    json!({
        "schema": CEILING_SCHEMA,
        "operation": operation,
        "metric": metric,
        "direction": "lower",
        "tolerance": 0.05,
        "p75": p75,
    })
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    // statistics.quantiles(method="inclusive"), the method bench.py uses.
    if sorted.len() == 1 {
        return round1(sorted[0]);
    }
    let rank = (sorted.len() - 1) as f64 * (p as f64 / 100.0);
    let below = rank.floor() as usize;
    let above = rank.ceil() as usize;
    let value = sorted[below] + (sorted[above] - sorted[below]) * (rank - below as f64);
    round1(value)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn require_present(value: &Value, fields: &[&str], label: &str) -> Result<(), AdapterError> {
    let object = value
        .as_object()
        .ok_or_else(|| AdapterError::Contract(format!("{label} must be an object")))?;
    if fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(AdapterError::Contract(format!(
            "{label} is missing one of {}",
            fields.join(", ")
        )));
    }
    Ok(())
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, AdapterError> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| AdapterError::Contract(format!("{field} must be a non-empty string")))
}

fn publish(out: &std::path::Path, name: &str, bytes: &[u8]) -> Result<(), AdapterError> {
    std::fs::create_dir_all(out)
        .map_err(|error| AdapterError::Internal(format!("create staging dir: {error}")))?;
    std::fs::write(out.join(name), bytes)
        .map_err(|error| AdapterError::Internal(format!("write {name}: {error}")))
}
