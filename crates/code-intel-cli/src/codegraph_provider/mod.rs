//! Explicit, advisory CodeGraph boundary. Rust owns input identity, cache
//! freshness and A04 admission; the installed engine owns all graph semantics.
mod parser;
mod process;
mod scope;
mod storage;
#[path = "../tool_path.rs"]
mod tool_path;

use std::fs;
use std::path::Path;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use parser::Request;
use process::Engine;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let request = match Request::parse(raw) {
        Ok(request) => request,
        Err(error) => {
            let mut result = envelope(raw.first().map(String::as_str).unwrap_or(""));
            reject(&mut result, &error);
            println!("{result}");
            return error.code;
        }
    };
    let (code, result) = run(&request);
    println!("{result}");
    code
}

pub(crate) fn explore(repo: &Path, query: &str, artifact_root: &Path) -> Result<Value, String> {
    let request = Request::parse(&[
        "explore".into(),
        "--repo".into(),
        repo.to_string_lossy().into_owned(),
        "--query".into(),
        query.into(),
        "--artifact-root".into(),
        artifact_root.to_string_lossy().into_owned(),
    ])
    .map_err(|error| error.message)?;
    let (code, result) = run(&request);
    if code == 0 {
        Ok(result)
    } else {
        Err(result.to_string())
    }
}

fn envelope(operation: &str) -> Value {
    json!({"schema":"code-intel-codegraph-result.v1","status":"rejected","operation":operation,
        "provider":{"id":"codegraph.cli","implementation":null,"executable":null,"launcher":null},
        "repository":null,"scope":["."],"snapshot":null,"cache":{"usable":false,"binding":"missing"},
        "authority":"advisory","completeness":"partial","domainVerdict":"unknown","engineeringFacts":[],
        "result":null,"rawOutput":null,"admission":null,"artifactRoot":null,"diagnostics":[]})
}

fn run(request: &Request) -> (i32, Value) {
    let mut result = envelope(&request.operation);
    match execute(request, &mut result) {
        Ok(()) => {
            result["status"] = json!("ok");
            result["exitCode"] = json!(0);
            (0, result)
        }
        Err(error) => {
            reject(&mut result, &error);
            (error.code, result)
        }
    }
}

fn reject(result: &mut Value, error: &Failure) {
    result["status"] = json!(match error.code {
        69 => "unavailable",
        70 | 74 => "failed",
        _ => "rejected",
    });
    result["result"] = Value::Null;
    result["admission"] = Value::Null;
    result["cache"]["usable"] = json!(false);
    if let Some(raw) = &error.raw {
        result["rawOutput"] = raw.clone();
    }
    result["diagnostics"] = json!([error.message]);
    result["exitCode"] = json!(error.code);
}

fn execute(request: &Request, result: &mut Value) -> Result<(), Failure> {
    let repo = fs::canonicalize(&request.repo)
        .map_err(|e| Failure::io(format!("resolve repository: {e}")))?;
    if !repo.is_dir() {
        return Err(Failure::usage("--repo must be an existing directory"));
    }
    result["repository"] = json!(repo);
    for path in request.file.iter().chain(request.changed.iter()) {
        storage::check_source_path(&repo, path)?;
    }
    let root = storage::resolve_root(request.artifact_root.as_deref(), &repo)?;
    result["artifactRoot"] = json!(root);
    // Missing providers have unavailable semantics before cache admission.
    let engine = Engine::discover(&repo)?;
    result["provider"] = engine.identity.clone();
    let cache = storage::cache_path(&repo)?;
    if request.operation == "index" {
        storage::initialize_cache(&cache)?;
    }
    let document = crate::snapshot::build_for_dag(&repo, "explicit_overlay", &[".".into()])
        .map_err(Failure::contract)?;
    let snapshot = &document["snapshot"];
    let identity = snapshot["identity"]
        .as_str()
        .ok_or_else(|| Failure::contract("snapshot builder returned no identity"))?;
    result["snapshot"] = snapshot.clone();
    let lease = crate::snapshot::begin_consumption(&repo, snapshot).map_err(Failure::contract)?;
    let inventory = lease.inventory_mirror_files();
    if inventory
        .keys()
        .any(|path| path.starts_with(".codegraph/") && path != ".codegraph/.gitignore")
    {
        return Err(Failure::contract("cache database/metadata appears in source snapshot; ignore .codegraph cache entries before use"));
    }
    if request.operation != "status" {
        scope::validate_configuration(&repo, &inventory)?;
        if request.operation == "node"
            && request
                .file
                .as_ref()
                .is_some_and(|file| !inventory.contains_key(file))
        {
            return Err(Failure::contract(
                "node --file is outside the hashed regular-file source snapshot",
            ));
        }
    }
    if !cache.is_dir() {
        if request.operation == "status" {
            result["diagnostics"] = json!(["No local CodeGraph cache; unusable until explicit index. No ancestor cache was queried."]);
            lease.verify_after(&repo).map_err(Failure::contract)?;
            return Ok(());
        }
        return Err(Failure::contract(
            "no local CodeGraph cache; explicit index is required",
        ));
    }
    let _lock = storage::CacheLock::acquire(&cache)?;
    // index may repair malformed/unbound caches; every other operation must
    // diagnose the marker rather than silently repairing it.
    let marker = if request.operation == "index" {
        None
    } else {
        match storage::read_marker(&cache) {
            Ok(marker) => marker,
            Err(error) if request.operation == "status" => {
                result["cache"] = json!({"usable":false,"binding":"invalid"});
                result["diagnostics"] = json!([error.message]);
                None
            }
            Err(error) => {
                result["cache"]["binding"] = json!("invalid");
                return Err(error);
            }
        }
    };
    let database = if cache.join("codegraph.db").is_file() {
        Some(storage::database_identity(&cache)?)
    } else {
        None
    };
    let compatible = marker.as_ref().is_some_and(|marker| {
        marker["repository"] == json!(repo)
            && marker["provider"] == engine.identity
            && marker["scope"] == json!(["."])
    });
    let database_matches = marker
        .as_ref()
        .is_some_and(|marker| Some(&marker["database"]) == database.as_ref());
    let current = compatible
        && database_matches
        && marker
            .as_ref()
            .is_some_and(|marker| marker["snapshot"] == *snapshot);
    if result["cache"]["binding"] != "invalid" {
        result["cache"] = json!({"usable":current,"binding":if database.is_none() { "missing" } else if marker.is_none() { "unbound" } else if current { "current" } else { "stale" },"boundSnapshot":marker.as_ref().map(|marker| &marker["snapshot"])});
    }
    if request.operation == "sync" && (!compatible || !database_matches) {
        return Err(Failure::contract("sync requires an existing bound index with unchanged provider/database identity; explicit index is required for unbound or externally modified caches"));
    }
    if !request.writes_index() && request.operation != "status" && !current {
        return Err(Failure::contract("CodeGraph cache is unbound, stale or missing; run explicit sync for source changes or index to bind/rebuild"));
    }
    if request.operation == "status" && database.is_none() {
        result["diagnostics"] =
            json!(["No local CodeGraph database; cache is unusable until explicit index."]);
        lease.verify_after(&repo).map_err(Failure::contract)?;
        return Ok(());
    }
    if request.operation == "status" && !current {
        result["diagnostics"].as_array_mut().unwrap().push(json!("Provider status is diagnostic only; this cache is unusable for Pipeline queries until explicit index/sync binds the exact source snapshot."));
    }
    if request.writes_index() {
        storage::invalidate(&cache)?;
    }
    let args = request.argv(&repo);
    let started = now()?;
    let initial = request.operation == "index" && database.is_none();
    let output = if initial {
        engine.initial_index(&repo)?
    } else {
        engine.invoke(&repo, &args)?
    };
    result["rawOutput"] = raw_output(
        &output,
        if request.json_output() {
            "json"
        } else {
            "text"
        },
    )?;
    if !output.status.success() {
        return Err(Failure::process("CodeGraph operation failed", &output));
    }
    lease.verify_after(&repo).map_err(Failure::contract)?;
    let completed = now()?.max(started);
    if request.writes_index() {
        let indexed_database = storage::database_identity(&cache)?;
        let indexed_files = engine.indexed_files(&repo)?;
        scope::validate_indexed_files(&repo, &inventory, &indexed_files)?;
        lease.verify_after(&repo).map_err(Failure::contract)?;
        if indexed_database != storage::database_identity(&cache)? {
            return Err(Failure::contract("CodeGraph database changed during indexed scope validation; no binding was published"));
        }
        let marker = json!({"schema":"code-intel-codegraph-snapshot-binding.v1","repository":repo,"scope":["."],"snapshot":snapshot,"provider":engine.identity,"database":indexed_database,"boundAt":completed});
        if let Err(error) = storage::publish_marker(&cache, &marker).and_then(|_| {
            lease.verify_after(&repo).map_err(Failure::contract)?;
            if marker["database"] != storage::database_identity(&cache)? {
                return Err(Failure::contract(
                    "CodeGraph database changed while publishing its binding",
                ));
            }
            Ok(())
        }) {
            storage::invalidate(&cache)?;
            return Err(error);
        }
        result["cache"] = json!({"usable":true,"binding":"current","boundSnapshot":snapshot});
    } else if database.as_ref() != Some(&storage::database_identity(&cache)?) {
        result["cache"]["binding"] = json!("stale");
        return Err(Failure::contract(
            "CodeGraph database changed while reading; explicit index/sync is required",
        ));
    }
    let stdout = result["rawOutput"]["stdout"].as_str().unwrap();
    let raw = if request.json_output() {
        crate::capability::reject_duplicate_json_keys_within(stdout, 64 * 1024 * 1024)
            .map_err(Failure::contract)?;
        let value: Value = serde_json::from_str(stdout).map_err(|e| {
            Failure::contract(format!(
                "provider promised JSON but emitted malformed/non-JSON output: {e}"
            ))
        })?;
        if !value.is_array() && !value.is_object() {
            return Err(Failure::contract(
                "provider JSON must be an object or array",
            ));
        }
        value
    } else {
        json!(stdout)
    };
    result["result"] = raw;
    result["exitCode"] = json!(0);
    if request.writes_index() || request.operation == "status" {
        return Ok(());
    }
    let provenance = json!({"collectionId":format!("codegraph-{identity}-{started}-{completed}-{}",std::process::id()),
        "command":serde_json::to_string(&engine.invocation(&args)).map_err(|e| Failure::contract(e.to_string()))?,"startedAt":started,"completedAt":completed});
    let uncertainty = json!(["Upstream language support is finite; unsupported source including PowerShell is not complete evidence.",
        "Static relationships, dynamic dispatch, confidence/provenance and upstream truncation remain partial; absence is not proof.",
        "Affected tests are candidates, not an authoritative test selection or delivery gate."]);
    let payload = json!({"schema":"code-intel-evidence-payload.v1","data":{"codegraph":{"schema":"code-intel-codegraph-observation.v1",
        "operation":request.operation,"repository":repo,"scope":["."],"snapshot":snapshot,"provider":engine.identity,
        "authority":"advisory","completeness":"partial","uncertainty":uncertainty,"result":result["result"],"rawOutput":result["rawOutput"],"provenance":provenance}}});
    let reference = storage::persist_payload(&root, identity, &payload)?;
    let provider = json!({"id":"codegraph.cli","implementation":engine.identity["implementation"]});
    let observation = json!({"schema":"code-intel-observed-evidence.v1","provider":provider,
        "source":{"endpointIdentity":format!("{}#.codegraph",repo.display())},"consumedSnapshotIdentity":identity,
        "observedAt":completed,"completeness":"partial","claimedComplete":false,"payload":reference,"provenance":provenance,"failure":{"kind":"none"}});
    let admission_request = json!({"schema":"code-intel-evidence-admissibility-request.v1","expectedSnapshotIdentity":identity,
        "policy":{"evaluatedAt":completed,"maxAgeSeconds":300},"observation":observation});
    let admission = crate::admissibility::validate_for_consumer(&admission_request, &root)
        .map_err(Failure::contract)?;
    if admission.result()["domainVerdict"] != "unknown" {
        return Err(Failure::contract(
            "partial CodeGraph evidence unexpectedly gained fact authority",
        ));
    }
    lease.verify_after(&repo).map_err(Failure::contract)?;
    if database.as_ref() != Some(&storage::database_identity(&cache)?) {
        return Err(Failure::contract(
            "CodeGraph database changed before evidence publication",
        ));
    }
    result["admission"] = admission.result().clone();
    result["uncertainty"] = uncertainty;
    Ok(())
}

fn now() -> Result<u64, Failure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|e| Failure::new(70, format!("system clock precedes Unix epoch: {e}")))
}

fn raw_output(output: &Output, format: &str) -> Result<Value, Failure> {
    let decoded = std::str::from_utf8(&output.stdout)
        .and_then(|stdout| std::str::from_utf8(&output.stderr).map(|stderr| (stdout, stderr)));
    let (stdout, stderr) = decoded.map_err(|error| Failure {
        code: 65, message: format!("provider output is not UTF-8: {error}"),
        raw: Some(json!({"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr),"stdoutBytes":output.stdout,"stderrBytes":output.stderr,"format":"bytes","exitCode":output.status.code()})),
    })?;
    Ok(json!({"stdout":stdout,"stderr":stderr,"format":format,"exitCode":output.status.code()}))
}

pub(super) struct Failure {
    code: i32,
    message: String,
    raw: Option<Value>,
}
impl Failure {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            raw: None,
        }
    }
    fn usage(message: impl Into<String>) -> Self {
        Self::new(64, message)
    }
    fn contract(message: impl Into<String>) -> Self {
        Self::new(65, message)
    }
    fn unavailable(message: impl Into<String>) -> Self {
        Self::new(69, message)
    }
    fn io(message: impl Into<String>) -> Self {
        Self::new(74, message)
    }
    fn process(message: impl Into<String>, output: &Output) -> Self {
        Self {
            code: 70,
            message: message.into(),
            raw: Some(raw_output(output, "text").unwrap_or_else(|error| error.raw.unwrap())),
        }
    }
}
