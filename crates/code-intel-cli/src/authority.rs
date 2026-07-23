use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

const KINDS: [&str; 6] = [
    "observed_evidence",
    "engineering_fact",
    "derived_engineering_model",
    "proposal",
    "adoption_decision",
    "committed_engineering_plan",
];
const ACTORS: [&str; 5] = [
    "deterministic_pipeline",
    "human",
    "llm",
    "provider",
    "recommender",
];
const TRUSTED_APPROVERS: [(&str, &str); 1] = [("code-intel-maintainers", "repository_governance")];
const ATTESTATION_SCHEME: &str = "repository-governed-sha256-v1";
const EDGES: [(&str, &str, bool); 7] = [
    ("observed_evidence", "engineering_fact", false),
    ("observed_evidence", "proposal", false),
    ("engineering_fact", "derived_engineering_model", false),
    ("derived_engineering_model", "proposal", false),
    ("proposal", "adoption_decision", true),
    ("adoption_decision", "committed_engineering_plan", true),
    ("proposal", "committed_engineering_plan", true),
];

pub(crate) fn policy_document() -> Value {
    json!({
        "schema":"code-intel-authority-transition-policy.v1",
        "artifactKinds":KINDS,
        "actorKinds":ACTORS,
        "edges":EDGES.iter().map(|(from,to,event)| json!({"from":from,"to":to,"authorityEventRequired":event})).collect::<Vec<_>>(),
        "restrictedActors":{"llm":["observed_evidence","proposal"],"provider":["observed_evidence","proposal"],"recommender":["observed_evidence","proposal"]},
        "trustedApprovers":TRUSTED_APPROVERS.iter().map(|(id,role)| json!({"id":id,"role":role})).collect::<Vec<_>>(),
        "attestation":{"scheme":ATTESTATION_SCHEME,"meaning":"content-bound repository sign-off, not cryptographic identity authentication"},
        "rule":"protected transitions are owned by an explicit approved authority event; source output alone has no commitment authority"
    })
}

pub(crate) fn evaluate_batch(request: &Value) -> Result<Value, String> {
    validate_batch(request)?;
    let evaluated_at = request["evaluatedAt"].as_u64().unwrap();
    let known = string_set(&request["knownEvidenceIds"], "knownEvidenceIds")?;
    let consumed = string_set(
        &request["consumedAuthorityEventIds"],
        "consumedAuthorityEventIds",
    )?;
    let branches = request["branches"].as_array().unwrap();
    let duplicate_branches = duplicates(branches.iter().filter_map(|b| b["branchId"].as_str()));
    let duplicate_outputs = duplicates(
        branches
            .iter()
            .filter_map(|b| b.pointer("/transition/outputId").and_then(Value::as_str)),
    );
    let duplicate_events = duplicates(branches.iter().filter_map(|b| {
        b.pointer("/transition/authorityEvent/id")
            .and_then(Value::as_str)
    }));

    let results = branches
        .iter()
        .enumerate()
        .map(|(index, branch)| {
            evaluate_branch(
                branch,
                index,
                evaluated_at,
                &known,
                &consumed,
                &duplicate_branches,
                &duplicate_outputs,
                &duplicate_events,
            )
        })
        .collect::<Vec<_>>();
    let accepted = results.iter().filter(|r| r["status"] == "accepted").count();
    let rejected = results.len() - accepted;
    let mut consumed_events = consumed.clone();
    consumed_events.extend(
        results
            .iter()
            .filter(|result| result["status"] == "accepted")
            .filter_map(|result| result["authorityEventId"].as_str())
            .map(str::to_string),
    );
    Ok(json!({
        "schema":"code-intel-authority-transition-result.v1",
        "status":"completed",
        "summary":{"accepted":accepted,"rejected":rejected},
        "consumedAuthorityEventIds":consumed_events,
        "branches":results
    }))
}

#[allow(clippy::too_many_arguments)]
fn evaluate_branch(
    branch: &Value,
    index: usize,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
    duplicate_branches: &BTreeSet<String>,
    duplicate_outputs: &BTreeSet<String>,
    duplicate_events: &BTreeSet<String>,
) -> Value {
    let branch_id = branch["branchId"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("invalid-branch-{index}"));
    let from = branch
        .pointer("/current/kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    let to = branch
        .pointer("/transition/to")
        .and_then(Value::as_str)
        .unwrap_or("");
    let output_id = branch
        .pointer("/transition/outputId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let event_id = branch
        .pointer("/transition/authorityEvent/id")
        .and_then(Value::as_str);
    let authority_event = branch.pointer("/transition/authorityEvent");
    let decision = validate_branch(branch, evaluated_at, known, consumed).and_then(|_| {
        if duplicate_branches.contains(&branch_id) {
            Err("duplicate branchId".to_string())
        } else if duplicate_outputs.contains(output_id) {
            Err("duplicate transition outputId".to_string())
        } else if event_id.is_some_and(|id| duplicate_events.contains(id)) {
            Err("duplicate authority event use".to_string())
        } else {
            Ok(())
        }
    });
    match decision {
        Ok(()) => json!({
            "branchId":branch_id,"status":"accepted","from":from,"to":to,
            "outputId":output_id,"authorityEventId":event_id,
            "authorityEvent":authority_event,
            "effectiveAuthority":if event_id.is_some() { "authority_event" } else { "deterministic_policy" },
            "diagnostics":[]
        }),
        Err(message) => json!({
            "branchId":branch_id,"status":"rejected","from":from,"to":to,
            "outputId":null,"authorityEventId":null,"authorityEvent":null,"effectiveAuthority":null,"diagnostics":[message]
        }),
    }
}

fn validate_batch(request: &Value) -> Result<(), String> {
    exact(
        request,
        &[
            "schema",
            "evaluatedAt",
            "knownEvidenceIds",
            "consumedAuthorityEventIds",
            "branches",
        ],
        "authority batch",
    )?;
    if request["schema"] != "code-intel-authority-transition-batch.v1" {
        return Err("authority batch schema is invalid".to_string());
    }
    request["evaluatedAt"]
        .as_u64()
        .ok_or("evaluatedAt must be a non-negative integer")?;
    string_set(&request["knownEvidenceIds"], "knownEvidenceIds")?;
    string_set(
        &request["consumedAuthorityEventIds"],
        "consumedAuthorityEventIds",
    )?;
    let branches = request["branches"]
        .as_array()
        .ok_or("branches must be an array")?;
    if branches.is_empty() {
        return Err("branches must not be empty".to_string());
    }
    Ok(())
}

fn validate_branch(
    branch: &Value,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
) -> Result<(), String> {
    exact(
        branch,
        &["branchId", "source", "current", "transition"],
        "transition branch",
    )?;
    nonempty(&branch["branchId"], "branchId")?;
    let source = &branch["source"];
    exact(source, &["kind", "id"], "source")?;
    let actor = source["kind"].as_str().ok_or("source kind is invalid")?;
    if !ACTORS.contains(&actor) {
        return Err("source kind is unknown".to_string());
    }
    nonempty(&source["id"], "source id")?;
    let current = &branch["current"];
    exact(current, &["kind", "id"], "current artifact")?;
    let from = current["kind"].as_str().ok_or("current kind is invalid")?;
    if !KINDS.contains(&from) {
        return Err("current kind is unknown".to_string());
    }
    nonempty(&current["id"], "current id")?;
    let transition = &branch["transition"];
    let fields = transition
        .as_object()
        .ok_or("transition must be an object")?;
    if !fields.keys().all(|key| {
        matches!(
            key.as_str(),
            "to" | "outputId" | "evidenceIds" | "authorityEvent"
        )
    }) || !["to", "outputId", "evidenceIds"]
        .iter()
        .all(|key| fields.contains_key(*key))
    {
        return Err("transition fields are invalid".to_string());
    }
    let to = transition["to"]
        .as_str()
        .ok_or("transition target is invalid")?;
    if !KINDS.contains(&to) {
        return Err("transition target is unknown".to_string());
    }
    nonempty(&transition["outputId"], "transition outputId")?;
    let evidence = string_set(&transition["evidenceIds"], "transition evidenceIds")?;
    if evidence.is_empty() {
        return Err("transition requires evidence".to_string());
    }
    if !evidence.is_subset(known) {
        return Err("transition references unknown evidence".to_string());
    }
    let (_, _, event_required) = EDGES
        .iter()
        .find(|(a, b, _)| *a == from && *b == to)
        .ok_or("transition edge is not allowed")?;
    if matches!(actor, "llm" | "provider" | "recommender")
        && !matches!(
            to,
            "observed_evidence" | "proposal" | "adoption_decision" | "committed_engineering_plan"
        )
    {
        return Err("source actor cannot create facts or derived models".to_string());
    }
    let event = transition.get("authorityEvent");
    if *event_required {
        validate_event(
            event.ok_or("protected transition requires authority event")?,
            evaluated_at,
            known,
            &evidence,
            consumed,
        )?;
    } else if event.is_some() {
        return Err("unprotected transition must not consume an authority event".to_string());
    }
    Ok(())
}

fn validate_event(
    event: &Value,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    transition_evidence: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
) -> Result<(), String> {
    validate_authority_event(event, evaluated_at, known, transition_evidence, consumed).map(|_| ())
}

pub(crate) fn validate_authority_event(
    event: &Value,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    required_evidence: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
) -> Result<String, String> {
    let mut event_fields = vec![
        "schema",
        "id",
        "decision",
        "approver",
        "evidenceIds",
        "issuedAt",
        "expiresAt",
    ];
    if event.get("attestation").is_some() {
        event_fields.push("attestation");
    }
    exact(event, &event_fields, "authority event")?;
    if event["schema"] != "code-intel-authority-event.v1" || event["decision"] != "approved" {
        return Err("authority event must be explicitly approved".to_string());
    }
    let id = event["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("authority event id is invalid")?;
    if consumed.contains(id) {
        return Err("authority event replay is rejected".to_string());
    }
    let approver = &event["approver"];
    exact(approver, &["id", "role"], "authority approver")?;
    nonempty(&approver["id"], "approver id")?;
    nonempty(&approver["role"], "approver role")?;
    let event_evidence = string_set(&event["evidenceIds"], "authority event evidenceIds")?;
    if event_evidence.is_empty()
        || !event_evidence.is_subset(known)
        || !required_evidence.is_subset(&event_evidence)
    {
        return Err("authority event evidence is unknown or incomplete".to_string());
    }
    let issued = event["issuedAt"]
        .as_u64()
        .ok_or("authority event issuedAt is invalid")?;
    let expires = event["expiresAt"]
        .as_u64()
        .ok_or("authority event expiresAt is invalid")?;
    if issued > evaluated_at || expires < evaluated_at || expires < issued {
        return Err("authority event is future-dated or expired".to_string());
    }
    if event.get("attestation").is_some() {
        validate_repository_attestation(event)?;
    }
    Ok(id.to_string())
}

/// Requires the backward-compatible v1 event extension used for repository-owned sign-off.
/// The digest detects content changes; trust comes only from the checked-in id/role allow-list.
pub(crate) fn validate_signed_authority_event(
    event: &Value,
    evaluated_at: u64,
    known: &BTreeSet<String>,
    required_evidence: &BTreeSet<String>,
    consumed: &BTreeSet<String>,
) -> Result<String, String> {
    let id = validate_authority_event(event, evaluated_at, known, required_evidence, consumed)?;
    if event.get("attestation").is_none() {
        return Err("repository sign-off attestation is required".to_string());
    }
    Ok(id)
}

pub(crate) fn authority_event_digest(event: &Value) -> Result<String, String> {
    let mut evidence = event["evidenceIds"]
        .as_array()
        .ok_or("authority event evidenceIds must be an array")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "authority event evidenceIds contains an invalid id".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    evidence.sort();
    let payload = json!({
        "schema":event["schema"],
        "id":event["id"],
        "decision":event["decision"],
        "approver":event["approver"],
        "evidenceIds":evidence,
        "issuedAt":event["issuedAt"],
        "expiresAt":event["expiresAt"]
    });
    Ok(sha256_hex(&serde_json::to_vec(&payload).unwrap()))
}

fn validate_repository_attestation(event: &Value) -> Result<(), String> {
    let approver_id = event["approver"]["id"]
        .as_str()
        .ok_or("approver id is invalid")?;
    let approver_role = event["approver"]["role"]
        .as_str()
        .ok_or("approver role is invalid")?;
    if !TRUSTED_APPROVERS.contains(&(approver_id, approver_role)) {
        return Err("authority event approver is not trusted by repository policy".to_string());
    }
    let attestation = &event["attestation"];
    exact(
        attestation,
        &["scheme", "digest"],
        "authority event attestation",
    )?;
    if attestation["scheme"] != ATTESTATION_SCHEME {
        return Err("authority event attestation scheme is invalid".to_string());
    }
    let digest = attestation["digest"]
        .as_str()
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or("authority event attestation digest is invalid")?;
    if digest != authority_event_digest(event)? {
        return Err("authority event attestation content digest mismatch".to_string());
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = bytes.to_vec();
    let bits = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value);
        }
    }
    h.iter().map(|value| format!("{value:08x}")).collect()
}

fn exact(value: &Value, expected: &[&str], label: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} fields are invalid"))
    }
}

fn string_set(value: &Value, label: &str) -> Result<BTreeSet<String>, String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?;
    let mut result = BTreeSet::new();
    for value in values {
        let item = value
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{label} contains an invalid id"))?;
        if !result.insert(item.to_string()) {
            return Err(format!("{label} contains duplicate ids"));
        }
    }
    Ok(result)
}

fn duplicates<'a>(values: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value.to_string()).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(value, count)| (count > 1).then_some(value))
        .collect()
}

fn nonempty(value: &Value, label: &str) -> Result<(), String> {
    if value.as_str().is_some_and(|value| !value.is_empty()) {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}
