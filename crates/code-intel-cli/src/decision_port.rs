use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const MAX_MESSAGE_BYTES: u64 = 128 * 1024;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let (value, exit_code) = match run_cli(raw) {
        Ok(value) => {
            let exit_code = match value["status"].as_str() {
                Some("resolved") => 0,
                Some("pending") => 10,
                Some("timeout") => 11,
                Some("cancelled") => 12,
                _ => 70,
            };
            (value, exit_code)
        }
        Err(error) => {
            eprintln!("decision request-response: {error}");
            (
                json!({
                    "schema":"code-intel-decision-exchange-result.v1",
                    "status":"rejected",
                    "correlationId":null,
                    "gapId":null,
                    "acceptedAnswer":null,
                    "authorityProvenance":null,
                    "branches":[],
                    "effects":[],
                    "diagnostics":[error.to_string()],
                }),
                65,
            )
        }
    };
    match serde_json::to_string_pretty(&value) {
        Ok(text) => {
            println!("{text}");
            exit_code
        }
        Err(error) => {
            eprintln!("decision request-response: serialize result: {error}");
            70
        }
    }
}

fn run_cli(raw: &[String]) -> Result<Value, DecisionPortError> {
    if raw.first().map(String::as_str) != Some("request-response") {
        return Err(DecisionPortError(
            "expected request-response subcommand".into(),
        ));
    }
    let mut request_path = None;
    let mut response_path = None;
    let mut cancellation_path = None;
    let mut now = None;
    let mut branches = Vec::new();
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        let value = raw
            .get(index + 1)
            .ok_or_else(|| DecisionPortError(format!("{flag} requires a value")))?;
        match flag {
            "--request" if request_path.is_none() => request_path = Some(value.clone()),
            "--response" if response_path.is_none() => response_path = Some(value.clone()),
            "--cancel" if cancellation_path.is_none() => cancellation_path = Some(value.clone()),
            "--now" if now.is_none() => {
                now =
                    Some(value.parse::<u64>().map_err(|_| {
                        DecisionPortError("--now must be an unsigned integer".into())
                    })?)
            }
            "--branch" => branches.push(value.clone()),
            _ => {
                return Err(DecisionPortError(format!(
                    "unknown or duplicate option {flag}"
                )))
            }
        }
        index += 2;
    }
    if response_path.is_some() && cancellation_path.is_some() {
        return Err(DecisionPortError(
            "--response and --cancel are mutually exclusive".into(),
        ));
    }
    let request_path =
        request_path.ok_or_else(|| DecisionPortError("--request is required".into()))?;
    let now = now.ok_or_else(|| DecisionPortError("--now is required".into()))?;
    let request = read_cli_message(&request_path)?;
    let mut port = NativeStructuredDecisionPort::default();
    if let Some(path) = response_path {
        port.supply_response(read_cli_message(&path)?)?;
    }
    if let Some(path) = cancellation_path {
        port.supply_cancellation(read_cli_message(&path)?)?;
    }
    let branch_refs = branches.iter().map(String::as_str).collect::<Vec<_>>();
    DecisionExchange::default().advance(&request, &mut port, now, &branch_refs)
}

fn read_cli_message(path: &str) -> Result<Value, DecisionPortError> {
    if path != "-" {
        return read_message(Path::new(path));
    }
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_MESSAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| DecisionPortError(format!("read stdin message: {error}")))?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(DecisionPortError("stdin message exceeds size limit".into()));
    }
    parse_message(&bytes, "stdin")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecisionPortError(String);

impl fmt::Display for DecisionPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DecisionPortError {}

#[derive(Debug, Clone)]
pub(crate) enum PortPoll {
    Pending,
    Response(Value),
    Cancelled(Value),
}

pub(crate) trait DecisionRequestResponsePort {
    fn submit(&mut self, request: &Value) -> Result<(), DecisionPortError>;
    fn poll(&mut self, correlation_id: &str) -> Result<PortPoll, DecisionPortError>;
}

#[derive(Debug, Clone)]
struct EvidenceRef {
    expires_at: u64,
}

#[derive(Debug, Clone)]
struct ParsedRequest {
    correlation_id: String,
    gap_id: String,
    option_ids: BTreeSet<String>,
    evidence_refs: Vec<EvidenceRef>,
    authority_kind: String,
    actor_ids: BTreeSet<String>,
    issued_at: u64,
    expires_at: u64,
    affected_branches: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub(crate) struct DecisionExchange {
    pending: BTreeMap<String, Value>,
    terminal: BTreeSet<String>,
}

impl DecisionExchange {
    pub(crate) fn advance<P: DecisionRequestResponsePort>(
        &mut self,
        request: &Value,
        port: &mut P,
        now: u64,
        branches: &[&str],
    ) -> Result<Value, DecisionPortError> {
        let parsed = parse_request(request)?;
        let branch_ids = validate_branches(branches, &parsed.affected_branches)?;
        if now < parsed.issued_at {
            return Err(DecisionPortError(
                "processing clock precedes request issue time".into(),
            ));
        }
        if self.terminal.contains(&parsed.correlation_id) {
            return Err(DecisionPortError(format!(
                "response replay for terminal correlation {}",
                parsed.correlation_id
            )));
        }
        match self.pending.get(&parsed.correlation_id) {
            Some(existing) if existing != request => {
                return Err(DecisionPortError(format!(
                    "correlation {} was reused for a different request",
                    parsed.correlation_id
                )))
            }
            Some(_) => {}
            None => {
                port.submit(request)?;
                self.pending
                    .insert(parsed.correlation_id.clone(), request.clone());
            }
        }

        match port.poll(&parsed.correlation_id)? {
            PortPoll::Pending if now >= parsed.expires_at => {
                self.finish(&parsed.correlation_id);
                Ok(result(
                    &parsed,
                    &branch_ids,
                    "timeout",
                    "blocked_timeout",
                    None,
                    None,
                ))
            }
            PortPoll::Pending => Ok(result(
                &parsed,
                &branch_ids,
                "pending",
                "blocked_pending_response",
                None,
                None,
            )),
            PortPoll::Response(response) => {
                if now >= parsed.expires_at {
                    self.finish(&parsed.correlation_id);
                    return Err(DecisionPortError(
                        "expired request cannot accept a response".into(),
                    ));
                }
                let (answer, provenance) = validate_response(&response, &parsed, now)?;
                self.finish(&parsed.correlation_id);
                Ok(result(
                    &parsed,
                    &branch_ids,
                    "resolved",
                    "ready",
                    Some(answer),
                    Some(provenance),
                ))
            }
            PortPoll::Cancelled(cancellation) => {
                if now >= parsed.expires_at {
                    self.finish(&parsed.correlation_id);
                    return Err(DecisionPortError(
                        "expired request cannot accept a cancellation".into(),
                    ));
                }
                let provenance = validate_cancellation(&cancellation, &parsed, now)?;
                self.finish(&parsed.correlation_id);
                Ok(result(
                    &parsed,
                    &branch_ids,
                    "cancelled",
                    "blocked_cancelled",
                    None,
                    Some(provenance),
                ))
            }
        }
    }

    fn finish(&mut self, correlation_id: &str) {
        self.pending.remove(correlation_id);
        self.terminal.insert(correlation_id.to_string());
    }
}

fn result(
    request: &ParsedRequest,
    branches: &BTreeSet<String>,
    status: &str,
    affected_status: &str,
    answer: Option<Value>,
    provenance: Option<Value>,
) -> Value {
    let branch_results = branches
        .iter()
        .map(|branch| {
            json!({
                "branchId": branch,
                "status": if request.affected_branches.contains(branch) {
                    affected_status
                } else {
                    "continues"
                },
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema": "code-intel-decision-exchange-result.v1",
        "status": status,
        "correlationId": request.correlation_id,
        "gapId": request.gap_id,
        "acceptedAnswer": answer,
        "authorityProvenance": provenance,
        "branches": branch_results,
        "effects": [],
        "diagnostics": [],
    })
}

fn parse_request(value: &Value) -> Result<ParsedRequest, DecisionPortError> {
    exact_object(
        value,
        "decision request",
        &[
            "schema",
            "correlationId",
            "gapId",
            "question",
            "recommendation",
            "evidenceRefs",
            "options",
            "authorityNeeded",
            "issuedAt",
            "expiresAt",
            "affectedBranches",
        ],
    )?;
    if value["schema"] != "code-intel-decision-request.v1" {
        return Err(DecisionPortError(
            "decision request schema is invalid".into(),
        ));
    }
    let correlation_id = identifier(&value["correlationId"], "correlationId")?.to_string();
    let gap_id = nonempty_string(&value["gapId"], "gapId")?.to_string();
    nonempty_string(&value["question"], "question")?;

    exact_object(
        &value["recommendation"],
        "recommendation",
        &["optionId", "rationale"],
    )?;
    let recommended_option = nonempty_string(
        &value["recommendation"]["optionId"],
        "recommendation.optionId",
    )?;
    nonempty_string(
        &value["recommendation"]["rationale"],
        "recommendation.rationale",
    )?;

    let option_values = value["options"]
        .as_array()
        .ok_or_else(|| DecisionPortError("options must be an array".into()))?;
    if option_values.len() < 2 {
        return Err(DecisionPortError(
            "decision request requires at least two options".into(),
        ));
    }
    let mut option_ids = BTreeSet::new();
    for option in option_values {
        exact_object(option, "option", &["id", "label", "consequence"])?;
        let id = nonempty_string(&option["id"], "option.id")?;
        nonempty_string(&option["label"], "option.label")?;
        nonempty_string(&option["consequence"], "option.consequence")?;
        if !option_ids.insert(id.to_string()) {
            return Err(DecisionPortError(format!("duplicate option id {id}")));
        }
    }
    if !option_ids.contains(recommended_option) {
        return Err(DecisionPortError(
            "recommendation names an unknown option".into(),
        ));
    }

    let evidence_values = value["evidenceRefs"]
        .as_array()
        .ok_or_else(|| DecisionPortError("evidenceRefs must be an array".into()))?;
    if evidence_values.is_empty() {
        return Err(DecisionPortError("evidenceRefs must not be empty".into()));
    }
    let mut evidence_ids = BTreeSet::new();
    let mut evidence_refs = Vec::new();
    for evidence in evidence_values {
        exact_object(
            evidence,
            "evidence ref",
            &["refId", "sha256", "observedAt", "expiresAt"],
        )?;
        let ref_id = nonempty_string(&evidence["refId"], "evidence ref.refId")?;
        if !evidence_ids.insert(ref_id.to_string()) {
            return Err(DecisionPortError(format!(
                "duplicate evidence ref {ref_id}"
            )));
        }
        let sha256 = nonempty_string(&evidence["sha256"], "evidence ref.sha256")?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DecisionPortError(
                "evidence ref.sha256 must be lowercase hexadecimal".into(),
            ));
        }
        let observed_at = integer(&evidence["observedAt"], "evidence ref.observedAt")?;
        let expires_at = integer(&evidence["expiresAt"], "evidence ref.expiresAt")?;
        if observed_at >= expires_at {
            return Err(DecisionPortError(
                "evidence ref expiry must follow observation".into(),
            ));
        }
        evidence_refs.push(EvidenceRef { expires_at });
    }

    exact_object(
        &value["authorityNeeded"],
        "authorityNeeded",
        &["kind", "actorIds"],
    )?;
    let authority_kind =
        nonempty_string(&value["authorityNeeded"]["kind"], "authorityNeeded.kind")?.to_string();
    let actor_ids = string_set(
        &value["authorityNeeded"]["actorIds"],
        "authorityNeeded.actorIds",
    )?;
    let issued_at = integer(&value["issuedAt"], "issuedAt")?;
    let expires_at = integer(&value["expiresAt"], "expiresAt")?;
    if issued_at >= expires_at {
        return Err(DecisionPortError(
            "request expiry must follow issue time".into(),
        ));
    }
    let affected_branches = string_set(&value["affectedBranches"], "affectedBranches")?;

    Ok(ParsedRequest {
        correlation_id,
        gap_id,
        option_ids,
        evidence_refs,
        authority_kind,
        actor_ids,
        issued_at,
        expires_at,
        affected_branches,
    })
}

fn validate_response(
    response: &Value,
    request: &ParsedRequest,
    now: u64,
) -> Result<(Value, Value), DecisionPortError> {
    exact_object(
        response,
        "decision response",
        &[
            "schema",
            "correlationId",
            "gapId",
            "answer",
            "actorProvenance",
            "timestamp",
        ],
    )?;
    if response["schema"] != "code-intel-decision-response.v1" {
        return Err(DecisionPortError(
            "decision response schema is invalid".into(),
        ));
    }
    validate_binding(response, request, now)?;
    let answer = &response["answer"];
    let answer_object = answer
        .as_object()
        .ok_or_else(|| DecisionPortError("answer must be an object".into()))?;
    match answer_object.get("kind").and_then(Value::as_str) {
        Some("choice") => {
            exact_object(answer, "choice answer", &["kind", "optionId"])?;
            let option_id = nonempty_string(&answer["optionId"], "answer.optionId")?;
            if !request.option_ids.contains(option_id) {
                return Err(DecisionPortError("answer selects an unknown option".into()));
            }
        }
        Some("free-form") => {
            exact_object(answer, "free-form answer", &["kind", "text"])?;
            nonempty_string(&answer["text"], "answer.text")?;
        }
        _ => return Err(DecisionPortError("answer kind is invalid".into())),
    }
    Ok((answer.clone(), response["actorProvenance"].clone()))
}

fn validate_cancellation(
    cancellation: &Value,
    request: &ParsedRequest,
    now: u64,
) -> Result<Value, DecisionPortError> {
    exact_object(
        cancellation,
        "decision cancellation",
        &[
            "schema",
            "correlationId",
            "gapId",
            "actorProvenance",
            "timestamp",
            "reason",
        ],
    )?;
    if cancellation["schema"] != "code-intel-decision-cancellation.v1" {
        return Err(DecisionPortError(
            "decision cancellation schema is invalid".into(),
        ));
    }
    nonempty_string(&cancellation["reason"], "cancellation.reason")?;
    validate_binding(cancellation, request, now)?;
    Ok(cancellation["actorProvenance"].clone())
}

fn validate_binding(
    message: &Value,
    request: &ParsedRequest,
    now: u64,
) -> Result<(), DecisionPortError> {
    if message["correlationId"].as_str() != Some(&request.correlation_id) {
        return Err(DecisionPortError("response correlation mismatch".into()));
    }
    if message["gapId"].as_str() != Some(&request.gap_id) {
        return Err(DecisionPortError("response gap mismatch".into()));
    }
    exact_object(
        &message["actorProvenance"],
        "actor provenance",
        &["actorId", "authorityKind", "source"],
    )?;
    let actor_id = nonempty_string(
        &message["actorProvenance"]["actorId"],
        "actorProvenance.actorId",
    )?;
    let authority_kind = nonempty_string(
        &message["actorProvenance"]["authorityKind"],
        "actorProvenance.authorityKind",
    )?;
    nonempty_string(
        &message["actorProvenance"]["source"],
        "actorProvenance.source",
    )?;
    if !request.actor_ids.contains(actor_id) || authority_kind != request.authority_kind {
        return Err(DecisionPortError(
            "response actor lacks the requested authority".into(),
        ));
    }
    let timestamp = integer(&message["timestamp"], "timestamp")?;
    if timestamp > now {
        return Err(DecisionPortError(
            "response timestamp is in the future".into(),
        ));
    }
    if timestamp < request.issued_at || timestamp > request.expires_at {
        return Err(DecisionPortError(
            "response timestamp is outside the request lifetime".into(),
        ));
    }
    if request
        .evidence_refs
        .iter()
        .any(|evidence| timestamp > evidence.expires_at)
    {
        return Err(DecisionPortError(
            "response is bound to stale evidence".into(),
        ));
    }
    Ok(())
}

fn validate_branches(
    branches: &[&str],
    affected: &BTreeSet<String>,
) -> Result<BTreeSet<String>, DecisionPortError> {
    let mut result = BTreeSet::new();
    for branch in branches {
        if branch.trim().is_empty() || !result.insert((*branch).to_string()) {
            return Err(DecisionPortError(
                "branch list must contain unique non-empty ids".into(),
            ));
        }
    }
    if let Some(missing) = affected.iter().find(|branch| !result.contains(*branch)) {
        return Err(DecisionPortError(format!(
            "affected branch {missing} is absent from the DAG branch set"
        )));
    }
    Ok(result)
}

#[derive(Debug, Default)]
pub(crate) struct InMemoryDecisionPort {
    requests: BTreeMap<String, Value>,
    events: VecDeque<PortPoll>,
}

impl InMemoryDecisionPort {
    pub(crate) fn supply_response(&mut self, response: Value) -> Result<(), DecisionPortError> {
        message_correlation(&response)?;
        self.events.push_back(PortPoll::Response(response));
        Ok(())
    }

    pub(crate) fn supply_cancellation(
        &mut self,
        cancellation: Value,
    ) -> Result<(), DecisionPortError> {
        message_correlation(&cancellation)?;
        self.events.push_back(PortPoll::Cancelled(cancellation));
        Ok(())
    }
}

impl DecisionRequestResponsePort for InMemoryDecisionPort {
    fn submit(&mut self, request: &Value) -> Result<(), DecisionPortError> {
        store_request(&mut self.requests, request)
    }

    fn poll(&mut self, _correlation_id: &str) -> Result<PortPoll, DecisionPortError> {
        Ok(self.events.pop_front().unwrap_or(PortPoll::Pending))
    }
}

#[derive(Debug, Default)]
pub(crate) struct NativeStructuredDecisionPort {
    requests: BTreeMap<String, Value>,
    events: VecDeque<PortPoll>,
}

impl NativeStructuredDecisionPort {
    pub(crate) fn supply_response(&mut self, response: Value) -> Result<(), DecisionPortError> {
        message_correlation(&response)?;
        self.events.push_back(PortPoll::Response(response));
        Ok(())
    }

    pub(crate) fn supply_cancellation(
        &mut self,
        cancellation: Value,
    ) -> Result<(), DecisionPortError> {
        message_correlation(&cancellation)?;
        self.events.push_back(PortPoll::Cancelled(cancellation));
        Ok(())
    }

    pub(crate) fn submitted(&self) -> impl Iterator<Item = &Value> {
        self.requests.values()
    }
}

impl DecisionRequestResponsePort for NativeStructuredDecisionPort {
    fn submit(&mut self, request: &Value) -> Result<(), DecisionPortError> {
        store_request(&mut self.requests, request)
    }

    fn poll(&mut self, _correlation_id: &str) -> Result<PortPoll, DecisionPortError> {
        Ok(self.events.pop_front().unwrap_or(PortPoll::Pending))
    }
}

#[derive(Debug, Default)]
pub(crate) struct PlainTextDecisionPort {
    requests: BTreeMap<String, Value>,
    outbox: Vec<String>,
    events: VecDeque<PortPoll>,
}

impl PlainTextDecisionPort {
    pub(crate) fn supply_line(&mut self, line: &str) -> Result<(), DecisionPortError> {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() != 8 || parts.iter().any(|part| part.trim().is_empty()) {
            return Err(DecisionPortError(
                "plain-text reply must contain eight non-empty tab-separated fields".into(),
            ));
        }
        let timestamp = parts[7]
            .parse::<u64>()
            .map_err(|_| DecisionPortError("plain-text timestamp is invalid".into()))?;
        let provenance = json!({
            "actorId": parts[4],
            "authorityKind": parts[5],
            "source": parts[6],
        });
        let event = match parts[0] {
            "choice" => PortPoll::Response(json!({
                "schema": "code-intel-decision-response.v1",
                "correlationId": parts[1],
                "gapId": parts[2],
                "answer": {"kind": "choice", "optionId": parts[3]},
                "actorProvenance": provenance,
                "timestamp": timestamp,
            })),
            "free-form" => PortPoll::Response(json!({
                "schema": "code-intel-decision-response.v1",
                "correlationId": parts[1],
                "gapId": parts[2],
                "answer": {"kind": "free-form", "text": parts[3]},
                "actorProvenance": provenance,
                "timestamp": timestamp,
            })),
            "cancel" => PortPoll::Cancelled(json!({
                "schema": "code-intel-decision-cancellation.v1",
                "correlationId": parts[1],
                "gapId": parts[2],
                "actorProvenance": provenance,
                "timestamp": timestamp,
                "reason": parts[3],
            })),
            _ => return Err(DecisionPortError("plain-text reply kind is invalid".into())),
        };
        self.events.push_back(event);
        Ok(())
    }

    pub(crate) fn outbox(&self) -> &[String] {
        &self.outbox
    }
}

impl DecisionRequestResponsePort for PlainTextDecisionPort {
    fn submit(&mut self, request: &Value) -> Result<(), DecisionPortError> {
        let parsed = parse_request(request)?;
        store_request(&mut self.requests, request)?;
        if !self.outbox.iter().any(|message| {
            message.starts_with(&format!("DECISION REQUEST {}\n", parsed.correlation_id))
        }) {
            let request_json = serde_json::to_string_pretty(request).map_err(|error| {
                DecisionPortError(format!("render plain-text request: {error}"))
            })?;
            self.outbox.push(format!(
                "DECISION REQUEST {}\n{}",
                parsed.correlation_id, request_json
            ));
        }
        Ok(())
    }

    fn poll(&mut self, _correlation_id: &str) -> Result<PortPoll, DecisionPortError> {
        Ok(self.events.pop_front().unwrap_or(PortPoll::Pending))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FileDecisionPort {
    root: PathBuf,
}

impl FileDecisionPort {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, correlation_id: &str, suffix: &str) -> Result<PathBuf, DecisionPortError> {
        if correlation_id.is_empty()
            || !correlation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(DecisionPortError(
                "correlation id is not portable for file transport".into(),
            ));
        }
        Ok(self.root.join(format!("{correlation_id}.{suffix}.json")))
    }
}

impl DecisionRequestResponsePort for FileDecisionPort {
    fn submit(&mut self, request: &Value) -> Result<(), DecisionPortError> {
        let parsed = parse_request(request)?;
        fs::create_dir_all(&self.root)
            .map_err(|error| DecisionPortError(format!("create file-port root: {error}")))?;
        let path = self.path(&parsed.correlation_id, "request")?;
        if path.exists() {
            let existing = read_message(&path)?;
            if existing != *request {
                return Err(DecisionPortError(
                    "file port correlation already contains a different request".into(),
                ));
            }
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(request)
            .map_err(|error| DecisionPortError(format!("serialize request: {error}")))?;
        fs::write(path, bytes)
            .map_err(|error| DecisionPortError(format!("write file-port request: {error}")))
    }

    fn poll(&mut self, correlation_id: &str) -> Result<PortPoll, DecisionPortError> {
        let response_path = self.path(correlation_id, "response")?;
        let cancel_path = self.path(correlation_id, "cancel")?;
        match (response_path.is_file(), cancel_path.is_file()) {
            (true, true) => Err(DecisionPortError(
                "file port contains both response and cancellation".into(),
            )),
            (true, false) => Ok(PortPoll::Response(read_message(&response_path)?)),
            (false, true) => Ok(PortPoll::Cancelled(read_message(&cancel_path)?)),
            (false, false) => Ok(PortPoll::Pending),
        }
    }
}

fn read_message(path: &Path) -> Result<Value, DecisionPortError> {
    let metadata = fs::metadata(path)
        .map_err(|error| DecisionPortError(format!("inspect port message: {error}")))?;
    if !metadata.is_file() || metadata.len() > MAX_MESSAGE_BYTES {
        return Err(DecisionPortError(
            "port message must be a bounded regular file".into(),
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| DecisionPortError(format!("read port message: {error}")))?;
    parse_message(&bytes, "port")
}

fn parse_message(bytes: &[u8], source: &str) -> Result<Value, DecisionPortError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| DecisionPortError(format!("{source} message is not UTF-8: {error}")))?;
    crate::capability::reject_duplicate_json_keys(text)
        .map_err(|error| DecisionPortError(format!("{source} message {error}")))?;
    serde_json::from_str(text)
        .map_err(|error| DecisionPortError(format!("parse {source} message: {error}")))
}

fn store_request(
    requests: &mut BTreeMap<String, Value>,
    request: &Value,
) -> Result<(), DecisionPortError> {
    let parsed = parse_request(request)?;
    match requests.get(&parsed.correlation_id) {
        Some(existing) if existing != request => Err(DecisionPortError(
            "port correlation already contains a different request".into(),
        )),
        Some(_) => Ok(()),
        None => {
            requests.insert(parsed.correlation_id, request.clone());
            Ok(())
        }
    }
}

fn message_correlation(message: &Value) -> Result<String, DecisionPortError> {
    identifier(&message["correlationId"], "message.correlationId").map(str::to_string)
}

fn exact_object(value: &Value, context: &str, keys: &[&str]) -> Result<(), DecisionPortError> {
    let object = value
        .as_object()
        .ok_or_else(|| DecisionPortError(format!("{context} must be an object")))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(DecisionPortError(format!("{context} has invalid fields")));
    }
    Ok(())
}

fn nonempty_string<'a>(value: &'a Value, context: &str) -> Result<&'a str, DecisionPortError> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| DecisionPortError(format!("{context} must be a non-empty string")))
}

fn identifier<'a>(value: &'a Value, context: &str) -> Result<&'a str, DecisionPortError> {
    let value = nonempty_string(value, context)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(DecisionPortError(format!("{context} is not portable")));
    }
    Ok(value)
}

fn integer(value: &Value, context: &str) -> Result<u64, DecisionPortError> {
    value
        .as_u64()
        .ok_or_else(|| DecisionPortError(format!("{context} must be an unsigned integer")))
}

fn string_set(value: &Value, context: &str) -> Result<BTreeSet<String>, DecisionPortError> {
    let values = value
        .as_array()
        .ok_or_else(|| DecisionPortError(format!("{context} must be an array")))?;
    if values.is_empty() {
        return Err(DecisionPortError(format!("{context} must not be empty")));
    }
    let mut result = BTreeSet::new();
    for value in values {
        let item = nonempty_string(value, context)?.to_string();
        if !result.insert(item.clone()) {
            return Err(DecisionPortError(format!(
                "{context} contains duplicate {item}"
            )));
        }
    }
    Ok(result)
}
