mod request;
mod validation;

use std::fs;
use std::io::{self, Read};

use serde_json::{json, Value};

use crate::capability::sha256_hex;

const PACKET_SCHEMA: &str = "code-intel-pr-evidence-packet.v1";

/// Assemble an advisory merge-review packet from already-collected evidence.
///
/// This boundary validates the supplied claim contract and binds it to one
/// snapshot. It deliberately does not discover, re-read, or authenticate the
/// source artifacts: a later committed-artifact adapter owns that work.
pub(crate) fn assemble(request: &Value) -> Result<Value, String> {
    let binding_payload = request::normalize(request)?;
    let digest = sha256_hex(
        &serde_json::to_vec(&json!({
            "schema": PACKET_SCHEMA,
            "subject": binding_payload["subject"],
            "claims": binding_payload["claims"],
        }))
        .map_err(|error| format!("serialize packet binding: {error}"))?,
    );
    let claims = binding_payload["claims"].clone();
    let decision = decide(claims.as_array().expect("claims are an array"));

    Ok(json!({
        "schema": PACKET_SCHEMA,
        "packetId": format!("pr-evidence-packet-v1:{digest}"),
        "binding": {"algorithm": "sha256", "sha256": digest},
        "subject": binding_payload["subject"],
        "claims": claims,
        "decision": decision,
    }))
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let (request, out) = match parse_cli(raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}");
            return 64;
        }
    };
    let result = (|| -> Result<(), String> {
        let bytes = read_request(&request)?;
        let packet = assemble(&request::parse_json(&bytes)?)?;
        let rendered = serde_json::to_string_pretty(&packet)
            .map_err(|error| format!("serialize PR evidence packet: {error}"))?;
        if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create output directory: {error}"))?;
        }
        fs::write(&out, rendered.as_bytes())
            .map_err(|error| format!("write PR evidence packet: {error}"))?;
        println!("{rendered}");
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            65
        }
    }
}

fn parse_cli(raw: &[String]) -> Result<(String, std::path::PathBuf), String> {
    if raw.len() != 5 || raw.first().map(String::as_str) != Some("evidence") {
        return Err(
            "usage: pr evidence --request <packet-request.json|-> --out <packet.json>".into(),
        );
    }
    let request = option(raw, "--request")?;
    let out = option(raw, "--out")?;
    if out == "-" {
        return Err("--out must name a file".into());
    }
    Ok((request.to_string(), out.into()))
}

fn option<'a>(raw: &'a [String], name: &str) -> Result<&'a str, String> {
    let positions = raw
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (value == name).then_some(index))
        .collect::<Vec<_>>();
    if positions.len() != 1 {
        return Err(format!("{name} must appear exactly once"));
    }
    raw.get(positions[0] + 1)
        .map(String::as_str)
        .filter(|value| !value.is_empty() && !value.starts_with("--"))
        .ok_or_else(|| format!("{name} requires a value"))
}

fn read_request(request: &str) -> Result<Vec<u8>, String> {
    if request == "-" {
        let mut bytes = Vec::new();
        io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read PR evidence request from stdin: {error}"))?;
        Ok(bytes)
    } else {
        fs::read(request).map_err(|error| format!("read PR evidence request: {error}"))
    }
}

fn decide(claims: &[Value]) -> Value {
    let gates = claims
        .iter()
        .filter(|claim| claim["authority"].as_str() == Some("gate"))
        .collect::<Vec<_>>();
    let gate_failures = gates
        .iter()
        .filter(|claim| claim["status"].as_str() == Some("fail"))
        .collect::<Vec<_>>();
    if !gate_failures.is_empty() {
        return decision(
            "blocked",
            "failed",
            gate_failures
                .iter()
                .map(|claim| reason(claim, "gate_failed"))
                .collect(),
            vec!["resolve failed gate claims and regenerate this advisory packet"],
        );
    }
    if gates.is_empty() {
        return decision(
            "manual_review",
            "unknown",
            vec![json!({
                "claimId": Value::Null,
                "code": "missing_gate_evidence",
                "summary": "no gate-authority claim was supplied"
            })],
            vec!["supply current gate evidence and obtain required human approval before merge"],
        );
    }

    let review_claims = claims
        .iter()
        .filter(|claim| {
            claim["status"].as_str() == Some("unknown") || claim["status"].as_str() == Some("fail")
        })
        .collect::<Vec<_>>();
    if !review_claims.is_empty() {
        let hard_gate_status = if gates
            .iter()
            .any(|claim| claim["status"].as_str() == Some("unknown"))
        {
            "unknown"
        } else {
            "passed"
        };
        let reasons = review_claims
            .iter()
            .map(|claim| {
                let availability = claim["availability"].as_str().expect("validated");
                let status = claim["status"].as_str().expect("validated");
                let authority = claim["authority"].as_str().expect("validated");
                let code = if availability == "stale" {
                    "evidence_stale"
                } else if availability == "unavailable" {
                    "evidence_unavailable"
                } else if status == "unknown" {
                    "claim_unknown"
                } else if authority == "advisory" {
                    "advisory_failed"
                } else {
                    "observation_failed"
                };
                reason(claim, code)
            })
            .collect();
        return decision(
            "manual_review",
            hard_gate_status,
            reasons,
            vec!["inspect listed claims, refresh unavailable evidence, and obtain required human approval before merge"],
        );
    }
    decision(
        "ready_for_human_merge_review",
        "passed",
        Vec::new(),
        vec!["obtain required human approval and configured CI status checks before merge"],
    )
}

fn decision(
    state: &str,
    hard_gate_status: &str,
    reasons: Vec<Value>,
    next_actions: Vec<&str>,
) -> Value {
    json!({
        "authority": "advisory",
        "state": state,
        "hardGateStatus": hard_gate_status,
        "reasons": reasons,
        "nextActions": next_actions,
    })
}

fn reason(claim: &Value, code: &str) -> Value {
    json!({
        "claimId": claim["id"],
        "code": code,
        "summary": claim["summary"],
    })
}
