use super::*;

pub(super) fn aggregate(condition: &str, runs: &[ScoredRun]) -> Value {
    let selected = runs
        .iter()
        .filter(|run| run.condition == condition)
        .collect::<Vec<_>>();
    let quality = selected
        .iter()
        .copied()
        .filter(|run| run.status == "completed" || run.status == "domain_unknown")
        .collect::<Vec<_>>();
    let attempted = selected.len();
    let completed = selected
        .iter()
        .filter(|run| run.status == "completed")
        .count();
    let unknown = selected
        .iter()
        .filter(|run| run.status == "domain_unknown")
        .count();
    let unavailable = selected
        .iter()
        .filter(|run| run.status == "unavailable")
        .count();
    let failed = selected
        .iter()
        .filter(|run| run.status == "process_failed")
        .count();
    json!({
        "condition":condition,
        "attempted":attempted,
        "completed":completed,
        "domainUnknown":unknown,
        "unavailable":unavailable,
        "processFailed":failed,
        "availabilityRate":ratio(completed + unknown, attempted),
        "honestUnknownRate":ratio(unknown, attempted),
        "unavailableRate":ratio(unavailable, attempted),
        "processFailureRate":ratio(failed, attempted),
        "quality":if quality.is_empty() { Value::Null } else { json!({
            "rootCauseRecallAtK":mean(&quality, |run| run.root_cause_recall),
            "relevantEntityPrecisionAtK":mean(&quality, |run| run.evidence_precision),
            "meanReciprocalRank":mean(&quality, |run| run.reciprocal_rank),
            "diagnosisRootCauseRecall":mean(&quality, |run| run.diagnosis_root_cause_recall),
            "diagnosisExactRate":mean(&quality, |run| run.diagnosis_exact),
            "attestationSuccessRate":mean(&quality, |run| run.attestation_success),
            "unsupportedClaimRate":mean(&quality, |run| run.unsupported_claim_rate)
        })},
        "cost":if selected.is_empty() { Value::Null } else { json!({
            "meanWallTimeMs":mean(&selected, |run| run.wall_time_ms),
            "meanInputTokens":mean(&selected, |run| run.input_tokens),
            "meanOutputTokens":mean(&selected, |run| run.output_tokens),
            "meanToolCalls":mean(&selected, |run| run.tool_calls),
            "meanArtifactBytes":mean(&selected, |run| run.artifact_bytes)
        })}
    })
}

pub(super) fn paired_comparison(condition: &str, runs: &[ScoredRun]) -> Value {
    let mut deltas = Vec::new();
    for candidate in runs.iter().filter(|run| run.condition == condition) {
        if !quality_observed(candidate) {
            continue;
        }
        if let Some(control) = runs.iter().find(|run| {
            run.condition == "C0"
                && run.case_id == candidate.case_id
                && run.repetition == candidate.repetition
                && quality_observed(run)
        }) {
            deltas.push((
                candidate.root_cause_recall - control.root_cause_recall,
                candidate.evidence_precision - control.evidence_precision,
                candidate.reciprocal_rank - control.reciprocal_rank,
                candidate.diagnosis_root_cause_recall - control.diagnosis_root_cause_recall,
                candidate.diagnosis_exact - control.diagnosis_exact,
                candidate.attestation_success - control.attestation_success,
                candidate.unsupported_claim_rate - control.unsupported_claim_rate,
                candidate.wall_time_ms - control.wall_time_ms,
                candidate.input_tokens - control.input_tokens,
                candidate.output_tokens - control.output_tokens,
                candidate.tool_calls - control.tool_calls,
            ));
        }
    }
    let mean_delta = |index: usize| {
        if deltas.is_empty() {
            Value::Null
        } else {
            json!(
                deltas
                    .iter()
                    .map(|values| tuple_at(values, index))
                    .sum::<f64>()
                    / deltas.len() as f64
            )
        }
    };
    json!({
        "baseline":"C0",
        "condition":condition,
        "pairedRunCount":deltas.len(),
        "rootCauseRecallAtKDelta":mean_delta(0),
        "relevantEntityPrecisionAtKDelta":mean_delta(1),
        "meanReciprocalRankDelta":mean_delta(2),
        "diagnosisRootCauseRecallDelta":mean_delta(3),
        "diagnosisExactRateDelta":mean_delta(4),
        "attestationSuccessRateDelta":mean_delta(5),
        "unsupportedClaimRateDelta":mean_delta(6),
        "wallTimeMsDelta":mean_delta(7),
        "inputTokensDelta":mean_delta(8),
        "outputTokensDelta":mean_delta(9),
        "toolCallsDelta":mean_delta(10)
    })
}

pub(super) fn tuple_at(
    values: &(f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64),
    index: usize,
) -> f64 {
    match index {
        0 => values.0,
        1 => values.1,
        2 => values.2,
        3 => values.3,
        4 => values.4,
        5 => values.5,
        6 => values.6,
        7 => values.7,
        8 => values.8,
        9 => values.9,
        _ => values.10,
    }
}

pub(super) fn quality_observed(run: &ScoredRun) -> bool {
    run.status == "completed" || run.status == "domain_unknown"
}

pub(super) fn mean(runs: &[&ScoredRun], metric: impl Fn(&ScoredRun) -> f64) -> f64 {
    runs.iter().map(|run| metric(run)).sum::<f64>() / runs.len() as f64
}

pub(super) fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

pub(super) fn required_string<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{context}.{field} must be a non-empty string"))
}

pub(super) fn required_digest<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    let digest = required_string(value, field, context)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{context}.{field} must be 64 lowercase hex"));
    }
    Ok(digest)
}

pub(super) fn required_u64(value: &Value, field: &str, context: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{context}.{field} must be an unsigned integer"))
}

pub(super) fn required_bool(value: &Value, field: &str, context: &str) -> Result<bool, String> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{context}.{field} must be boolean"))
}

pub(super) fn string_array<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<Vec<&'a str>, String> {
    let values = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}.{field} must be an array"))?;
    values
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format!("{context}.{field} must contain non-empty strings"))
        })
        .collect()
}

pub(super) fn string_set(
    value: &Value,
    field: &str,
    context: &str,
) -> Result<BTreeSet<String>, String> {
    Ok(string_array(value, field, context)?
        .into_iter()
        .map(str::to_string)
        .collect())
}

pub(super) fn require_schema(value: &Value, schema: &str, context: &str) -> Result<(), String> {
    if value.get("schema").and_then(Value::as_str) != Some(schema) {
        return Err(format!("{context} schema must be {schema}"));
    }
    Ok(())
}

pub(super) fn document_digest(value: &Value) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| format!("serialize benchmark input for digest: {error}"))
}

pub(super) fn validate_artifact_entity_ref(reference: &str) -> Result<(), String> {
    let digest = reference
        .strip_prefix("artifact://sha256/")
        .ok_or_else(|| {
            "ExternalAttestation.evidenceRefs must use artifact://sha256/<digest>".to_string()
        })?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("ExternalAttestation.evidenceRefs must use artifact://sha256/<digest>".into());
    }
    Ok(())
}

pub(super) fn require_exact_fields(
    value: &Value,
    context: &str,
    expected: &[&str],
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{context} fields differ from the closed v1 schema"));
    }
    Ok(())
}

pub(super) fn render_markdown(report: &Value) -> String {
    let variants = report["variants"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|variant| {
            format!(
                "| {} | {} | {} | {} | {} |",
                variant["condition"].as_str().unwrap_or("unknown"),
                variant["availabilityRate"].as_f64().unwrap_or(0.0),
                variant["quality"]["rootCauseRecallAtK"]
                    .as_f64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".into()),
                variant["quality"]["relevantEntityPrecisionAtK"]
                    .as_f64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".into()),
                variant["quality"]["attestationSuccessRate"]
                    .as_f64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".into())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# Tool Effectiveness Benchmark\n\n- Verdict: {}\n- Corpus: {}\n- Cases: {}\n- Repetitions: {}\n\n| Condition | Availability | Root-cause recall@K | Relevant-EntityRef precision@K | Authority attestation success |\n| --- | ---: | ---: | ---: | ---: |\n{}\n",
        report["verdict"].as_str().unwrap_or("unknown"),
        report["corpus"]["id"].as_str().unwrap_or("unknown"),
        report["corpus"]["caseCount"].as_u64().unwrap_or(0),
        report["corpus"]["repetitions"].as_u64().unwrap_or(0),
        variants
    )
}
