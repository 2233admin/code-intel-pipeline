use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::artifact_ref;
use crate::capability_inventory::{self, AdapterError};

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_JSON_DEPTH: usize = 128;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let parsed = match parse_cli(raw) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let outcome = execute_cli(
        &parsed.capability,
        &parsed.request,
        &parsed.out,
        parsed.artifact_root.as_deref(),
        parsed.manifest.as_deref(),
    );
    if let Some(result) = outcome.result {
        if let Err(message) = validate_result_envelope(&result) {
            eprintln!("executor refused to emit an invalid result envelope: {message}");
            return 70;
        }
        println!(
            "{}",
            serde_json::to_string(&result).expect("result envelope serializes")
        );
    }
    if let Some(diagnostic) = outcome.stderr {
        eprintln!("{diagnostic}");
    }
    outcome.exit_code
}

struct ExecCli {
    capability: String,
    request: PathBuf,
    out: PathBuf,
    manifest: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
}

fn parse_cli(raw: &[String]) -> Result<ExecCli, String> {
    if raw.len() < 2 || raw[0] != "exec" || raw[1].starts_with('-') {
        return Err(
            "usage: capability exec <id> --request <request.json|-> --out <staging-dir> [--artifact-root <directory>]"
                .to_string(),
        );
    }
    let capability = raw[1].clone();
    let mut request = None;
    let mut out = None;
    let mut manifest = None;
    let mut artifact_root = None;
    let mut index = 2;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--request" | "--out" | "--manifest" | "--artifact-root"
        ) {
            return Err(format!(
                "unknown or conflicting capability argument: {flag}"
            ));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        let slot = match flag {
            "--request" => &mut request,
            "--out" => &mut out,
            "--manifest" => &mut manifest,
            "--artifact-root" => &mut artifact_root,
            _ => unreachable!(),
        };
        if slot.replace(PathBuf::from(value)).is_some() {
            return Err(format!("duplicate capability argument: {flag}"));
        }
        index += 2;
    }
    Ok(ExecCli {
        capability,
        request: request.ok_or("capability exec requires exactly one --request")?,
        out: out.ok_or("capability exec requires exactly one --out")?,
        manifest,
        artifact_root,
    })
}

struct CliOutcome {
    result: Option<Value>,
    stderr: Option<String>,
    exit_code: i32,
}

fn execute_cli(
    cli_capability: &str,
    request_file: &Path,
    out_dir: &Path,
    artifact_root: Option<&Path>,
    manifest: Option<&Path>,
) -> CliOutcome {
    let request = match read_one_request(request_file) {
        Ok(request) => request,
        Err((code, message)) => return pre_envelope(code, &message),
    };
    if let Err(message) = validate_request(&request) {
        return failure_from(&request, 64, &message);
    }
    let registry = match load_registry(manifest) {
        Ok(registry) => registry,
        Err(RegistryError::Unavailable(message)) => return failure_from(&request, 69, &message),
        Err(RegistryError::Invalid(message)) => return failure_from(&request, 65, &message),
    };
    let (declaration, adapter) = match find_declaration(&registry, cli_capability) {
        Ok(Some(value)) => value,
        Ok(None) => {
            return failure_from(&request, 64, "CLI capability has no registered declaration")
        }
        Err(message) => return failure_from(&request, 65, &message),
    };
    if request["capability"].as_str() != Some(cli_capability) {
        return failure_from_declaration(
            &request,
            &declaration,
            64,
            "CLI capability differs from request capability",
        );
    }
    if let Err(message) = validate_declaration(&declaration) {
        return failure_from_declaration(&request, &declaration, 65, &message);
    }
    if let Err(message) = cohere(&declaration, &request) {
        return failure_from_declaration(&request, &declaration, 64, &message);
    }
    let verified_inputs = match artifact_ref::verify_inputs(
        &request["inputs"],
        artifact_root,
        request["snapshot"]["identity"]
            .as_str()
            .expect("validated snapshot identity"),
    ) {
        Ok(verified) => verified,
        Err(error) => {
            let exit = if matches!(error, artifact_ref::ArtifactError::Io(_)) {
                74
            } else {
                65
            };
            return failure_from_declaration(&request, &declaration, exit, error.message());
        }
    };
    match capability_inventory::execute(&adapter, &request, &verified_inputs, out_dir) {
        Ok(output) => {
            let domain_verdict = output.domain_verdict.as_str();
            let domain_failure = output.domain_failure.clone();
            let artifacts = output
                .artifacts
                .into_iter()
                .map(|artifact| {
                    json!({
                        "schema":"code-intel-artifact-ref.v1",
                        "artifactSchema":artifact.artifact_schema,
                        "type":artifact.artifact_type,
                        "path":artifact.relative_path,
                        "sha256":sha256_hex(&artifact.bytes),
                        "consumedSnapshotIdentity":request["snapshot"]["identity"]
                    })
                })
                .collect();
            let (exit_code, verdict, diagnostics) = match (domain_verdict, domain_failure) {
                ("fail", Some(message)) => (10, "fail", vec![message]),
                ("fail", None) => (
                    10,
                    "fail",
                    vec!["adapter returned a domain fail verdict without a diagnostic".into()],
                ),
                ("pass" | "unknown" | "not_applicable", None) => (0, "pass", vec![]),
                (_, Some(message)) => {
                    return failure_from_declaration(
                        &request,
                        &declaration,
                        70,
                        &format!(
                            "adapter returned a domain failure diagnostic for {domain_verdict}: {message}"
                        ),
                    )
                }
                _ => unreachable!("AdapterDomainVerdict is closed"),
            };
            let result = base_result(
                &request,
                &declaration,
                exit_code,
                "completed",
                verdict,
                domain_verdict,
                artifacts,
                output.observed_effects,
                diagnostics,
            );
            if let Err(message) = validate_result(&result, &request, &declaration) {
                return failure_from_declaration(
                    &request,
                    &declaration,
                    70,
                    &format!("executor produced invalid result: {message}"),
                );
            }
            CliOutcome {
                result: Some(result),
                stderr: None,
                exit_code,
            }
        }
        Err(AdapterError::InvalidOptions(message)) => {
            failure_from_declaration(&request, &declaration, 64, &message)
        }
        Err(AdapterError::Contract(message)) => {
            failure_from_declaration(&request, &declaration, 65, &message)
        }
        Err(AdapterError::Unavailable(message)) => {
            failure_from_declaration(&request, &declaration, 69, &message)
        }
        Err(AdapterError::Internal(message)) => {
            failure_from_declaration(&request, &declaration, 70, &message)
        }
        Err(AdapterError::Io(message)) => {
            failure_from_declaration(&request, &declaration, 74, &message)
        }
    }
}

fn pre_envelope(exit_code: i32, message: &str) -> CliOutcome {
    CliOutcome {
        result: None,
        stderr: Some(message.to_string()),
        exit_code,
    }
}

fn read_one_request(path: &Path) -> Result<Value, (i32, String)> {
    let bytes = if path == Path::new("-") {
        read_limited(io::stdin())
            .map_err(|err| (74, format!("cannot read stdin request: {err}")))?
    } else {
        let file = fs::File::open(path).map_err(|err| {
            (
                74,
                format!("cannot read request file {}: {err}", path.display()),
            )
        })?;
        read_limited(file).map_err(|err| {
            (
                74,
                format!("cannot read request file {}: {err}", path.display()),
            )
        })?
    };
    if bytes.len() > MAX_JSON_BYTES {
        return Err((64, format!("JSON input exceeds {MAX_JSON_BYTES} bytes")));
    }
    let text = String::from_utf8(bytes)
        .map_err(|err| (64, format!("request is not valid UTF-8: {err}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    reject_duplicate_json_keys(text).map_err(|message| (64, message))?;
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let first = stream
        .next()
        .ok_or_else(|| (64, "expected exactly one request JSON document".to_string()))?
        .map_err(|err| (64, format!("invalid request JSON: {err}")))?;
    if stream.next().is_some() {
        return Err((
            64,
            "expected exactly one request JSON document; found additional input".to_string(),
        ));
    }
    Ok(first)
}

enum RegistryError {
    Unavailable(String),
    Invalid(String),
}

fn load_registry(explicit: Option<&Path>) -> Result<Value, RegistryError> {
    let path = discover_manifest(explicit).ok_or_else(|| {
        RegistryError::Unavailable("cannot locate orchestration/integrations.json".to_string())
    })?;
    let file = fs::File::open(&path).map_err(|err| {
        RegistryError::Unavailable(format!("cannot read registry {}: {err}", path.display()))
    })?;
    let bytes = read_limited(file).map_err(|err| {
        RegistryError::Unavailable(format!("cannot read registry {}: {err}", path.display()))
    })?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(RegistryError::Invalid(format!(
            "JSON input exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    let text = String::from_utf8(bytes).map_err(|err| {
        RegistryError::Invalid(format!(
            "registry {} is not valid UTF-8: {err}",
            path.display()
        ))
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    reject_duplicate_json_keys(text).map_err(RegistryError::Invalid)?;
    serde_json::from_str(text).map_err(|err| {
        RegistryError::Invalid(format!("invalid registry {}: {err}", path.display()))
    })
}

fn read_limited(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_JSON_BYTES as u64) + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn discover_manifest(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return path.is_file().then(|| path.to_path_buf());
    }
    if let Some(path) = env::var_os("CODE_INTEL_INTEGRATIONS_MANIFEST") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    if let Some(home) = env::var_os("CODE_INTEL_HOME") {
        let path = PathBuf::from(home)
            .join("orchestration")
            .join("integrations.json");
        return path.is_file().then_some(path);
    }
    let mut candidates = vec![];
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("orchestration").join("integrations.json"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("orchestration")
            .join("integrations.json"),
    );
    candidates.into_iter().find(|path| path.is_file())
}

fn find_declaration(registry: &Value, capability: &str) -> Result<Option<(Value, String)>, String> {
    let integrations = registry
        .get("integrations")
        .and_then(Value::as_array)
        .ok_or("registry integrations must be an array")?;
    let mut all_ids = BTreeSet::new();
    let mut found = None;
    for entry in integrations {
        let Some(declaration) = entry.get("capabilityDeclaration") else {
            continue;
        };
        let id = declaration
            .get("id")
            .and_then(Value::as_str)
            .ok_or("registered capability declaration lacks id")?;
        if !all_ids.insert(id) {
            return Err(format!("duplicate registered capability declaration: {id}"));
        }
        if id == capability {
            found = Some((
                declaration.clone(),
                entry
                    .get("runtimeAdapter")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ));
        }
    }
    Ok(found)
}

pub(crate) fn reject_duplicate_json_keys(text: &str) -> Result<(), String> {
    if text.len() > MAX_JSON_BYTES {
        return Err(format!("JSON input exceeds {MAX_JSON_BYTES} bytes"));
    }
    JsonKeyScanner {
        bytes: text.as_bytes(),
        pos: 0,
    }
    .scan_document()
}

struct JsonKeyScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonKeyScanner<'_> {
    fn scan_document(&mut self) -> Result<(), String> {
        self.ws();
        self.value(0)?;
        self.ws();
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err("invalid trailing JSON input".to_string())
        }
    }
    fn value(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_JSON_DEPTH {
            return Err(format!("JSON nesting exceeds {MAX_JSON_DEPTH}"));
        }
        self.ws();
        match self.bytes.get(self.pos).copied() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(|_| ()),
            Some(_) => {
                while self.pos < self.bytes.len()
                    && !matches!(
                        self.bytes[self.pos],
                        b',' | b']' | b'}' | b' ' | b'\t' | b'\r' | b'\n'
                    )
                {
                    self.pos += 1;
                }
                Ok(())
            }
            None => Err("unexpected end of JSON".to_string()),
        }
    }
    fn object(&mut self, depth: usize) -> Result<(), String> {
        self.pos += 1;
        self.ws();
        let mut keys = BTreeSet::new();
        if self.take(b'}') {
            return Ok(());
        }
        loop {
            self.ws();
            let key = self.string()?;
            if !keys.insert(key.clone()) {
                return Err(format!("duplicate JSON object key: {key}"));
            }
            self.ws();
            if !self.take(b':') {
                return Err("invalid JSON object separator".to_string());
            }
            self.value(depth)?;
            self.ws();
            if self.take(b'}') {
                return Ok(());
            }
            if !self.take(b',') {
                return Err("invalid JSON object delimiter".to_string());
            }
        }
    }
    fn array(&mut self, depth: usize) -> Result<(), String> {
        self.pos += 1;
        self.ws();
        if self.take(b']') {
            return Ok(());
        }
        loop {
            self.value(depth)?;
            self.ws();
            if self.take(b']') {
                return Ok(());
            }
            if !self.take(b',') {
                return Err("invalid JSON array delimiter".to_string());
            }
        }
    }
    fn string(&mut self) -> Result<String, String> {
        let start = self.pos;
        if !self.take(b'"') {
            return Err("expected JSON string".to_string());
        }
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\\' => {
                    self.pos += 1;
                    if self.pos >= self.bytes.len() {
                        return Err("unterminated JSON escape".to_string());
                    }
                    self.pos += 1;
                }
                b'"' => {
                    self.pos += 1;
                    return serde_json::from_slice(&self.bytes[start..self.pos])
                        .map_err(|e| format!("invalid JSON string: {e}"));
                }
                _ => self.pos += 1,
            }
        }
        Err("unterminated JSON string".to_string())
    }
    fn ws(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.pos += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

fn validate_request(request: &Value) -> Result<(), String> {
    let object = request.as_object().ok_or("request must be a JSON object")?;
    require_exact_keys(
        object,
        &[
            "schema",
            "capability",
            "contractVersion",
            "implementation",
            "snapshot",
            "options",
            "inputs",
            "effectPolicy",
        ],
        "request",
    )?;
    if request["schema"] != "code-intel-capability-request.v1" || request["contractVersion"] != 1 {
        return Err("request must use the v1 schema and contract".to_string());
    }
    require_id(&request["capability"], "request.capability")?;
    validate_implementation(&request["implementation"], "request.implementation")?;
    validate_snapshot(&request["snapshot"])?;
    if !request["options"].is_object() || !request["inputs"].is_array() {
        return Err("request options/inputs have invalid types".to_string());
    }
    for artifact in request["inputs"].as_array().expect("checked array") {
        validate_artifact_ref_shape(artifact)?;
    }
    let policy = request["effectPolicy"]
        .as_object()
        .ok_or("request.effectPolicy must be an object")?;
    require_exact_keys(policy, &["allowedEffects"], "request.effectPolicy")?;
    validate_effects(
        &request["effectPolicy"]["allowedEffects"],
        "request.effectPolicy.allowedEffects",
    )
}

fn validate_declaration(declaration: &Value) -> Result<(), String> {
    let object = declaration
        .as_object()
        .ok_or("declaration must be an object")?;
    require_exact_keys(
        object,
        &[
            "schema",
            "id",
            "contractVersion",
            "implementation",
            "determinism",
            "allowedEffects",
            "dependencies",
        ],
        "declaration",
    )?;
    if declaration["schema"] != "code-intel-capability-declaration.v1"
        || declaration["contractVersion"] != 1
    {
        return Err("registered declaration is not v1".to_string());
    }
    require_id(&declaration["id"], "declaration.id")?;
    validate_implementation(&declaration["implementation"], "declaration.implementation")?;
    if !matches!(
        declaration["determinism"].as_str(),
        Some("deterministic" | "external_nondeterministic")
    ) {
        return Err("declaration determinism is invalid".to_string());
    }
    validate_effects(&declaration["allowedEffects"], "declaration.allowedEffects")?;
    if !declaration["dependencies"].as_array().is_some_and(|v| {
        let mut seen = BTreeSet::new();
        v.iter().all(|id| {
            id.as_str()
                .is_some_and(|id| valid_id(id) && seen.insert(id))
        })
    }) {
        return Err("declaration dependencies are invalid".to_string());
    }
    Ok(())
}

fn cohere(declaration: &Value, request: &Value) -> Result<(), String> {
    if request["capability"] != declaration["id"] {
        return Err("request capability differs from declaration id".to_string());
    }
    if request["implementation"] != declaration["implementation"] {
        return Err("request implementation differs from declaration".to_string());
    }
    if !string_set(&request["effectPolicy"]["allowedEffects"])
        .is_subset(&string_set(&declaration["allowedEffects"]))
    {
        return Err("request effects exceed declaration".to_string());
    }
    Ok(())
}

fn validate_result(result: &Value, request: &Value, declaration: &Value) -> Result<(), String> {
    validate_result_envelope(result)?;
    for artifact in result["artifacts"]
        .as_array()
        .expect("validated result artifacts")
    {
        if artifact["consumedSnapshotIdentity"] != result["snapshotIdentity"] {
            return Err("result artifact snapshot coherence failure".to_string());
        }
    }
    if result["capability"] != request["capability"]
        || result["implementation"] != request["implementation"]
        || result["snapshotIdentity"] != request["snapshot"]["identity"]
        || result["determinism"] != declaration["determinism"]
        || result["declaredEffects"] != request["effectPolicy"]["allowedEffects"]
    {
        return Err("result coherence failure".to_string());
    }
    if !string_set(&result["observedEffects"]).is_subset(&string_set(&result["declaredEffects"])) {
        return Err("observed undeclared effect".to_string());
    }
    Ok(())
}

fn validate_result_envelope(result: &Value) -> Result<(), String> {
    let object = result.as_object().ok_or("result must be an object")?;
    require_exact_keys(
        object,
        &[
            "schema",
            "capability",
            "implementation",
            "snapshotIdentity",
            "status",
            "verdict",
            "domainVerdict",
            "exitCode",
            "determinism",
            "declaredEffects",
            "observedEffects",
            "cache",
            "artifacts",
            "diagnostics",
            "provenance",
        ],
        "result",
    )?;
    if result["schema"] != "code-intel-capability-result.v1" {
        return Err("result schema is invalid".to_string());
    }
    require_id(&result["capability"], "result.capability")?;
    validate_implementation(&result["implementation"], "result.implementation")?;
    if !result["snapshotIdentity"].as_str().is_some_and(is_digest) {
        return Err("result snapshotIdentity is invalid".to_string());
    }
    if !matches!(
        result["determinism"].as_str(),
        Some("deterministic" | "external_nondeterministic")
    ) {
        return Err("result determinism is invalid".to_string());
    }
    validate_effects(&result["declaredEffects"], "result.declaredEffects")?;
    validate_effects(&result["observedEffects"], "result.observedEffects")?;
    let cache = result["cache"]
        .as_object()
        .ok_or("result cache must be an object")?;
    require_exact_keys(cache, &["key", "hit"], "result.cache")?;
    if !result["cache"]["key"].is_null() && !result["cache"]["key"].as_str().is_some_and(is_digest)
    {
        return Err("result cache key is invalid".to_string());
    }
    if !result["cache"]["hit"].is_boolean() {
        return Err("result cache hit is invalid".to_string());
    }
    let artifacts = result["artifacts"]
        .as_array()
        .ok_or("result artifacts must be an array")?;
    for artifact in artifacts {
        validate_artifact_ref_shape(artifact)?;
        let path = artifact["path"].as_str().unwrap_or("");
        if Path::new(path).is_absolute()
            || Path::new(path)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err("result artifact path escapes output boundary".to_string());
        }
    }
    if !result["diagnostics"]
        .as_array()
        .is_some_and(|items| items.iter().all(Value::is_string))
    {
        return Err("result diagnostics are invalid".to_string());
    }
    let provenance = result["provenance"]
        .as_object()
        .ok_or("result provenance must be an object")?;
    let allowed: BTreeSet<&str> = [
        "attemptId",
        "generatedAt",
        "provider",
        "model",
        "configurationDigest",
    ]
    .into_iter()
    .collect();
    if provenance.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err("result provenance contains an unknown field".to_string());
    }
    if !provenance
        .get("attemptId")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty())
        || !provenance
            .get("generatedAt")
            .and_then(Value::as_str)
            .is_some_and(is_rfc3339_utc)
    {
        return Err("result provenance required fields are invalid".to_string());
    }
    for key in ["provider", "model"] {
        if provenance
            .get(key)
            .is_some_and(|v| !v.as_str().is_some_and(|v| !v.is_empty()))
        {
            return Err(format!("result provenance {key} is invalid"));
        }
    }
    if provenance
        .get("configurationDigest")
        .is_some_and(|v| !v.as_str().is_some_and(is_digest))
    {
        return Err("result provenance configurationDigest is invalid".to_string());
    }
    let exit = result["exitCode"]
        .as_i64()
        .ok_or("result exitCode missing")?;
    if !matches!(
        result["domainVerdict"].as_str(),
        Some("pass" | "fail" | "unknown" | "not_applicable")
    ) {
        return Err("result domainVerdict is invalid".to_string());
    }
    if !matches!(
        (
            result["status"].as_str(),
            result["verdict"].as_str(),
            result["domainVerdict"].as_str(),
            exit
        ),
        (
            Some("completed"),
            Some("pass" | "not_applicable"),
            Some("pass" | "unknown" | "not_applicable"),
            0
        ) | (Some("completed"), Some("fail"), Some("fail"), 10)
            | (Some("blocked"), Some("unknown"), Some("unknown"), 20)
            | (
                Some("failed"),
                Some("unknown"),
                Some("unknown"),
                64 | 65 | 69 | 70 | 74
            )
    ) {
        return Err("illegal status/verdict/domainVerdict/exitCode".to_string());
    }
    Ok(())
}

fn is_rfc3339_utc(value: &str) -> bool {
    if !(value.len() == 20
        && value.ends_with('Z')
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':'))
    {
        return false;
    }
    let parse =
        |range: std::ops::Range<usize>| value.get(range).and_then(|part| part.parse::<u32>().ok());
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        parse(0..4),
        parse(5..7),
        parse(8..10),
        parse(11..13),
        parse(14..16),
        parse(17..19),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=max_day).contains(&day)
}

fn base_result(
    request: &Value,
    declaration: &Value,
    exit: i32,
    status: &str,
    verdict: &str,
    domain_verdict: &str,
    artifacts: Vec<Value>,
    observed: Vec<String>,
    diagnostics: Vec<String>,
) -> Value {
    json!({"schema":"code-intel-capability-result.v1","capability":request["capability"],"implementation":request["implementation"],"snapshotIdentity":request["snapshot"]["identity"],"status":status,"verdict":verdict,"domainVerdict":domain_verdict,"exitCode":exit,"determinism":declaration["determinism"],"declaredEffects":request["effectPolicy"]["allowedEffects"],"observedEffects":observed,"cache":{"key":null,"hit":false},"artifacts":artifacts,"diagnostics":diagnostics,"provenance":{"attemptId":format!("capability-{}-{}",now_seconds(),std::process::id()),"generatedAt":rfc3339_now()}})
}

fn failure_from(request: &Value, exit: i32, message: &str) -> CliOutcome {
    failure_with_determinism(request, exit, message, "deterministic")
}

fn failure_from_declaration(
    request: &Value,
    declaration: &Value,
    exit: i32,
    message: &str,
) -> CliOutcome {
    let determinism = declaration
        .get("determinism")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "deterministic" | "external_nondeterministic"))
        .unwrap_or("deterministic");
    failure_with_determinism(request, exit, message, determinism)
}

fn failure_with_determinism(
    request: &Value,
    exit: i32,
    message: &str,
    determinism: &str,
) -> CliOutcome {
    let capability = request
        .get("capability")
        .and_then(Value::as_str)
        .filter(|v| valid_id(v))
        .unwrap_or("invalid.request");
    let implementation = request
        .get("implementation")
        .filter(|v| validate_implementation(v, "implementation").is_ok())
        .cloned()
        .unwrap_or_else(|| json!({"id":"invalid.request","version":"1","toolchainDigests":[]}));
    let snapshot = request
        .pointer("/snapshot/identity")
        .and_then(Value::as_str)
        .filter(|v| is_digest(v))
        .unwrap_or(ZERO_DIGEST);
    let effects = request
        .pointer("/effectPolicy/allowedEffects")
        .filter(|v| validate_effects(v, "effects").is_ok())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let result = json!({"schema":"code-intel-capability-result.v1","capability":capability,"implementation":implementation,"snapshotIdentity":snapshot,"status":"failed","verdict":"unknown","domainVerdict":"unknown","exitCode":exit,"determinism":determinism,"declaredEffects":effects,"observedEffects":[],"cache":{"key":null,"hit":false},"artifacts":[],"diagnostics":[message],"provenance":{"attemptId":format!("capability-{}-{}",now_seconds(),std::process::id()),"generatedAt":rfc3339_now()}});
    CliOutcome {
        result: Some(result),
        stderr: Some(message.to_string()),
        exit_code: exit,
    }
}

fn validate_implementation(value: &Value, name: &str) -> Result<(), String> {
    let o = value
        .as_object()
        .ok_or_else(|| format!("{name} must be an object"))?;
    require_exact_keys(o, &["id", "version", "toolchainDigests"], name)?;
    if o.get("id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .is_none()
        || o.get("version")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .is_none()
    {
        return Err(format!("{name} id/version invalid"));
    }
    validate_digests(
        &value["toolchainDigests"],
        &format!("{name}.toolchainDigests"),
    )
}
pub(crate) fn validate_snapshot(value: &Value) -> Result<(), String> {
    let o = value.as_object().ok_or("snapshot must be an object")?;
    require_exact_keys(
        o,
        &[
            "identity",
            "repoIdentity",
            "head",
            "workingTreePolicy",
            "scope",
            "inputDigest",
        ],
        "snapshot",
    )?;
    if !["identity", "inputDigest"]
        .iter()
        .all(|k| o.get(*k).and_then(Value::as_str).is_some_and(is_digest))
    {
        return Err("snapshot digest invalid".to_string());
    }
    if !["repoIdentity", "head"].iter().all(|k| {
        o.get(*k)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    }) {
        return Err("snapshot identity fields invalid".to_string());
    }
    if !matches!(
        o.get("workingTreePolicy").and_then(Value::as_str),
        Some("head_only" | "explicit_overlay")
    ) {
        return Err("snapshot workingTreePolicy invalid".to_string());
    }
    if !o.get("scope").and_then(Value::as_array).is_some_and(|v| {
        let mut seen = BTreeSet::new();
        v.iter()
            .all(|s| s.as_str().is_some_and(|s| !s.is_empty() && seen.insert(s)))
    }) {
        return Err("snapshot scope invalid".to_string());
    }
    Ok(())
}

pub(crate) fn validate_artifact_ref_shape(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("input Artifact Ref must be an object")?;
    require_exact_keys(
        object,
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
        "input Artifact Ref",
    )?;
    if value["schema"] != "code-intel-artifact-ref.v1" {
        return Err("input Artifact Ref schema is invalid".to_string());
    }
    for key in ["artifactSchema", "type", "path"] {
        if !object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
        {
            return Err(format!("input Artifact Ref {key} is invalid"));
        }
    }
    if !value["sha256"].as_str().is_some_and(is_digest) {
        return Err("input Artifact Ref sha256 is invalid".to_string());
    }
    if !value["consumedSnapshotIdentity"].is_null()
        && !value["consumedSnapshotIdentity"]
            .as_str()
            .is_some_and(is_digest)
    {
        return Err("input Artifact Ref consumedSnapshotIdentity is invalid".to_string());
    }
    Ok(())
}
fn require_exact_keys(o: &Map<String, Value>, keys: &[&str], name: &str) -> Result<(), String> {
    let a: BTreeSet<&str> = o.keys().map(String::as_str).collect();
    let e: BTreeSet<&str> = keys.iter().copied().collect();
    if a == e {
        Ok(())
    } else {
        Err(format!("{name} fields differ from v1 schema"))
    }
}
fn validate_effects(v: &Value, name: &str) -> Result<(), String> {
    let a = v
        .as_array()
        .ok_or_else(|| format!("{name} must be array"))?;
    let mut s = BTreeSet::new();
    if a.iter().all(|v| {
        v.as_str().is_some_and(|e| {
            matches!(
                e,
                "repo_read" | "local_write" | "process_spawn" | "network" | "repo_mutation"
            ) && s.insert(e)
        })
    }) {
        Ok(())
    } else {
        Err(format!("{name} invalid"))
    }
}
fn validate_digests(v: &Value, name: &str) -> Result<(), String> {
    let a = v
        .as_array()
        .ok_or_else(|| format!("{name} must be array"))?;
    let mut s = BTreeSet::new();
    if a.iter()
        .all(|v| v.as_str().is_some_and(|d| is_digest(d) && s.insert(d)))
    {
        Ok(())
    } else {
        Err(format!("{name} invalid"))
    }
}
fn require_id(v: &Value, name: &str) -> Result<(), String> {
    if v.as_str().is_some_and(valid_id) {
        Ok(())
    } else {
        Err(format!("{name} invalid"))
    }
}
fn valid_id(v: &str) -> bool {
    !v.is_empty()
        && v.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || (i > 0 && matches!(b, b'.' | b'_' | b'-'))
        })
}
fn is_digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn string_set(v: &Value) -> BTreeSet<&str> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}
fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0)
}
fn rfc3339_now() -> String {
    let s = now_seconds() as i64;
    let days = s.div_euclid(86400);
    let ds = s.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        ds / 3600,
        (ds % 3600) / 60,
        ds % 60
    )
}
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
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
        data.push(0)
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
            w[i] = u32::from_be_bytes(word.try_into().unwrap())
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1)
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
            a = t1.wrapping_add(t2)
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value)
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha256_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn duplicate_key_scanner_rejects_nested_duplicates() {
        assert!(reject_duplicate_json_keys(r#"{"outer":{"key":1,"key":2}}"#).is_err());
        assert!(
            reject_duplicate_json_keys(r#"{"outer":{"key":1},"other":[true,null,"x"]}"#).is_ok()
        );
    }

    #[test]
    fn json_scanner_enforces_size_and_depth_before_deserialization() {
        assert!(reject_duplicate_json_keys(&" ".repeat(MAX_JSON_BYTES + 1))
            .unwrap_err()
            .contains("exceeds"));
        let deeply_nested = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_DEPTH + 2),
            "]".repeat(MAX_JSON_DEPTH + 2)
        );
        assert!(reject_duplicate_json_keys(&deeply_nested)
            .unwrap_err()
            .contains("nesting"));
    }

    #[test]
    fn rfc3339_validator_checks_calendar_and_clock_ranges() {
        assert!(is_rfc3339_utc("2024-02-29T23:59:59Z"));
        assert!(!is_rfc3339_utc("2023-02-29T00:00:00Z"));
        assert!(!is_rfc3339_utc("2026-02-30T00:00:00Z"));
        assert!(!is_rfc3339_utc("2026-13-01T00:00:00Z"));
        assert!(!is_rfc3339_utc("2026-01-01T24:00:00Z"));
    }

    #[test]
    fn result_validator_rejects_every_nested_contract_family() {
        let implementation = json!({"id":"inventory.rg.compat","version":"1.0.0","toolchainDigests":["a".repeat(64)]});
        let request = json!({"schema":"code-intel-capability-request.v1","capability":"inventory.rg","contractVersion":1,"implementation":implementation,"snapshot":{"identity":"b".repeat(64),"repoIdentity":"fixture","head":"head","workingTreePolicy":"head_only","scope":["."],"inputDigest":"c".repeat(64)},"options":{},"inputs":[],"effectPolicy":{"allowedEffects":["repo_read","local_write"]}});
        let declaration = json!({"schema":"code-intel-capability-declaration.v1","id":"inventory.rg","contractVersion":1,"implementation":request["implementation"],"determinism":"deterministic","allowedEffects":["repo_read","local_write"],"dependencies":[]});
        let artifact = json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":"inventory.v1","type":"inventory.files","path":"files.txt","sha256":"d".repeat(64),"consumedSnapshotIdentity":"b".repeat(64)});
        let result = base_result(
            &request,
            &declaration,
            0,
            "completed",
            "pass",
            "pass",
            vec![artifact],
            vec!["repo_read".into(), "local_write".into()],
            vec![],
        );
        assert!(validate_result(&result, &request, &declaration).is_ok());
        let mutations: Vec<Box<dyn Fn(&mut Value)>> = vec![
            Box::new(|v| {
                v.as_object_mut()
                    .unwrap()
                    .insert("extra".into(), json!(true));
            }),
            Box::new(|v| v["schema"] = json!("wrong")),
            Box::new(|v| v["cache"]["hit"] = json!("false")),
            Box::new(|v| v["artifacts"][0]["path"] = json!("../escape")),
            Box::new(|v| v["artifacts"][0]["consumedSnapshotIdentity"] = json!("e".repeat(64))),
            Box::new(|v| v["diagnostics"] = json!([1])),
            Box::new(|v| v["provenance"]["generatedAt"] = json!("not-a-time")),
            Box::new(|v| v["provenance"]["extra"] = json!(true)),
        ];
        for mutate in mutations {
            let mut candidate = result.clone();
            mutate(&mut candidate);
            assert!(validate_result(&candidate, &request, &declaration).is_err());
        }
    }
}
