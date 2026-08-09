//! Read-only import boundary for Diaphora binary-diff result databases.
//!
//! Diaphora is an AGPL-3.0 IDA plugin. This module never installs, invokes, or
//! embeds it: it only validates the SQLite result database the operator already
//! produced and publishes bounded, path-free evidence for later review.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MATCH_KINDS: [&str; 4] = ["best", "partial", "unreliable", "multimatch"];
const UNMATCHED_KINDS: [&str; 2] = ["primary", "secondary"];
const SAMPLE_LIMIT: usize = 20;
const SAMPLE_TEXT_LIMIT: usize = 512;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };

    match inspect(&cli) {
        Ok(document) => emit(&cli, &document, 0),
        Err(InspectionError::Unavailable(message)) => {
            let document = unavailable_document(&cli, &message);
            emit(&cli, &document, 69)
        }
        Err(InspectionError::Rejected(message)) => {
            let document = rejected_document(&cli, &message);
            emit(&cli, &document, 65)
        }
    }
}

struct Cli {
    result_db: PathBuf,
    base_binary: PathBuf,
    candidate_binary: PathBuf,
    source_revision: String,
    provider_version: String,
    observed_at: u64,
    out: PathBuf,
}

enum InspectionError {
    Unavailable(String),
    Rejected(String),
}

fn parse_cli(raw: &[String]) -> Result<Cli, String> {
    let mut result_db = None;
    let mut base_binary = None;
    let mut candidate_binary = None;
    let mut source_revision = None;
    let mut provider_version = None;
    let mut observed_at = None;
    let mut out = None;
    let mut index = 0;

    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--result-db"
                | "--base-binary"
                | "--candidate-binary"
                | "--source-revision"
                | "--provider-version"
                | "--observed-at"
                | "--out"
        ) {
            return Err(format!("unknown Diaphora provider argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        match flag {
            "--result-db" => set_path(&mut result_db, value, flag)?,
            "--base-binary" => set_path(&mut base_binary, value, flag)?,
            "--candidate-binary" => set_path(&mut candidate_binary, value, flag)?,
            "--source-revision" => set_text(&mut source_revision, value, flag, "source revision")?,
            "--provider-version" => {
                set_text(&mut provider_version, value, flag, "provider version")?
            }
            "--observed-at" => set_timestamp(&mut observed_at, value, flag)?,
            "--out" => set_path(&mut out, value, flag)?,
            _ => unreachable!(),
        }
        index += 2;
    }

    Ok(Cli {
        result_db: result_db.ok_or("Diaphora provider requires --result-db")?,
        base_binary: base_binary.ok_or("Diaphora provider requires --base-binary")?,
        candidate_binary: candidate_binary
            .ok_or("Diaphora provider requires --candidate-binary")?,
        source_revision: source_revision.ok_or("Diaphora provider requires --source-revision")?,
        provider_version: provider_version
            .ok_or("Diaphora provider requires --provider-version")?,
        observed_at: observed_at.ok_or("Diaphora provider requires --observed-at")?,
        out: out.ok_or("Diaphora provider requires --out")?,
    })
}

fn set_path(slot: &mut Option<PathBuf>, value: &str, flag: &str) -> Result<(), String> {
    if slot.replace(PathBuf::from(value)).is_some() {
        return Err(format!("duplicate Diaphora provider argument: {flag}"));
    }
    Ok(())
}

fn set_text(slot: &mut Option<String>, value: &str, flag: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(format!(
            "Diaphora {label} must be printable and at most 256 bytes"
        ));
    }
    if slot.replace(value.to_string()).is_some() {
        return Err(format!("duplicate Diaphora provider argument: {flag}"));
    }
    Ok(())
}

fn set_timestamp(slot: &mut Option<u64>, value: &str, flag: &str) -> Result<(), String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "Diaphora --observed-at must be an unsigned Unix timestamp".to_string())?;
    if slot.replace(parsed).is_some() {
        return Err(format!("duplicate Diaphora provider argument: {flag}"));
    }
    Ok(())
}

fn inspect(cli: &Cli) -> Result<Value, InspectionError> {
    let base_binary_sha256 = file_sha256(&cli.base_binary, "base binary")?;
    let candidate_binary_sha256 = file_sha256(&cli.candidate_binary, "candidate binary")?;
    let result_database_sha256 = file_sha256(&cli.result_db, "Diaphora result database")?;
    let connection = Connection::open_with_flags(
        &cli.result_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| InspectionError::Rejected("Diaphora result database is unreadable".into()))?;

    ensure_table_columns(
        &connection,
        "config",
        &["main_db", "diff_db", "version", "date"],
    )?;
    ensure_table_columns(
        &connection,
        "results",
        &[
            "type",
            "line",
            "address",
            "name",
            "address2",
            "name2",
            "ratio",
            "nodes1",
            "nodes2",
            "description",
        ],
    )?;
    ensure_table_columns(
        &connection,
        "unmatched",
        &["type", "line", "address", "name"],
    )?;

    let result_schema_version = connection
        .query_row("SELECT version FROM config LIMIT 1", [], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|_| InspectionError::Rejected("Diaphora config table cannot be read".into()))?
        .ok_or_else(|| InspectionError::Rejected("Diaphora config version is missing".into()))?;
    let result_schema_version = checked_result_schema_version(result_schema_version)?;

    let matches = count_kinds(&connection, "results", &MATCH_KINDS)?;
    let unmatched = count_kinds(&connection, "unmatched", &UNMATCHED_KINDS)?;
    let result_rows = matches.values().sum::<u64>();
    let comparison_identity = comparison_identity(
        &base_binary_sha256,
        &candidate_binary_sha256,
        &result_database_sha256,
        &cli.source_revision,
    );

    Ok(json!({
        "schema":"code-intel-diaphora-observation.v1",
        "status":"observed",
        "provider":{
            "id":"diaphora",
            "upstreamVersion":cli.provider_version,
            "resultSchemaVersion":result_schema_version,
            "mode":"external_result_import"
        },
        "comparison":{
            "identity":comparison_identity,
            "baseBinarySha256":base_binary_sha256,
            "candidateBinarySha256":candidate_binary_sha256,
            "resultDatabaseSha256":result_database_sha256,
            "sourceRevision":cli.source_revision
        },
        "observedAt":cli.observed_at,
        "summary":{
            "resultRows":result_rows,
            "matches":matches,
            "unmatched":unmatched,
            "topMatches":top_matches(&connection)?
        },
        "authority":{
            "observationOnly":true,
            "engineeringFacts":[],
            "note":"Diaphora match evidence requires human review before use in a patch-diff conclusion."
        },
        "failure":{"kind":"none"}
    }))
}

fn file_sha256(path: &Path, label: &str) -> Result<String, InspectionError> {
    let mut file = File::open(path)
        .map_err(|_| InspectionError::Unavailable(format!("required {label} is unavailable")))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| {
            InspectionError::Unavailable(format!("required {label} cannot be read"))
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn checked_result_schema_version(value: String) -> Result<String, InspectionError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(InspectionError::Rejected(
            "Diaphora config version is invalid".into(),
        ));
    }
    Ok(value)
}

fn ensure_table_columns(
    connection: &Connection,
    table: &str,
    expected_columns: &[&str],
) -> Result<(), InspectionError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|_| {
            InspectionError::Rejected(format!("Diaphora {table} table cannot be inspected"))
        })?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|_| {
            InspectionError::Rejected(format!("Diaphora {table} table cannot be inspected"))
        })?;
    let actual = rows
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|_| InspectionError::Rejected(format!("Diaphora {table} table cannot be read")))?;
    if expected_columns
        .iter()
        .all(|column| actual.contains(*column))
    {
        Ok(())
    } else {
        Err(InspectionError::Rejected(format!(
            "Diaphora result database lacks required {table} columns"
        )))
    }
}

fn count_kinds(
    connection: &Connection,
    table: &str,
    expected_kinds: &[&str],
) -> Result<BTreeMap<String, u64>, InspectionError> {
    let mut counts = expected_kinds
        .iter()
        .map(|kind| ((*kind).to_string(), 0u64))
        .collect::<BTreeMap<_, _>>();
    let mut statement = connection
        .prepare(&format!("SELECT type, COUNT(*) FROM {table} GROUP BY type"))
        .map_err(|_| {
            InspectionError::Rejected(format!("Diaphora {table} summary cannot be read"))
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|_| {
            InspectionError::Rejected(format!("Diaphora {table} summary cannot be read"))
        })?;
    for row in rows {
        let (kind, count) = row.map_err(|_| {
            InspectionError::Rejected(format!("Diaphora {table} summary contains an invalid row"))
        })?;
        let count = u64::try_from(count).map_err(|_| {
            InspectionError::Rejected(format!(
                "Diaphora {table} summary contains an invalid count"
            ))
        })?;
        let Some(slot) = counts.get_mut(&kind) else {
            return Err(InspectionError::Rejected(format!(
                "Diaphora {table} summary contains unsupported type"
            )));
        };
        *slot = count;
    }
    Ok(counts)
}

fn top_matches(connection: &Connection) -> Result<Vec<Value>, InspectionError> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, name2, ratio, description
             FROM results
             WHERE type IN ('best', 'partial', 'unreliable', 'multimatch')
             ORDER BY CASE type
                 WHEN 'best' THEN 0
                 WHEN 'partial' THEN 1
                 WHEN 'unreliable' THEN 2
                 ELSE 3
             END, CAST(ratio AS REAL) DESC, line ASC
             LIMIT ?1",
        )
        .map_err(|_| InspectionError::Rejected("Diaphora matches cannot be queried".into()))?;
    let rows = statement
        .query_map([SAMPLE_LIMIT as i64], |row| {
            let ratio = row.get::<_, Option<f64>>(3)?;
            Ok(json!({
                "type":row.get::<_, String>(0)?,
                "baseName":bounded_text(row.get::<_, Option<String>>(1)?),
                "candidateName":bounded_text(row.get::<_, Option<String>>(2)?),
                "ratio":ratio,
                "heuristic":bounded_text(row.get::<_, Option<String>>(4)?)
            }))
        })
        .map_err(|_| InspectionError::Rejected("Diaphora matches cannot be queried".into()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| InspectionError::Rejected("Diaphora matches contain an invalid row".into()))
}

fn bounded_text(value: Option<String>) -> Option<String> {
    value.map(|text| text.chars().take(SAMPLE_TEXT_LIMIT).collect())
}

fn comparison_identity(base: &str, candidate: &str, result: &str, source_revision: &str) -> String {
    let canonical = format!("diaphora-v1\0{base}\0{candidate}\0{result}\0{source_revision}");
    crate::capability::sha256_hex(canonical.as_bytes())
}

fn unavailable_document(cli: &Cli, message: &str) -> Value {
    base_document(
        cli,
        "unavailable",
        Value::Null,
        Value::Null,
        json!({
            "kind":"provider_unavailable",
            "message":message
        }),
    )
}

fn rejected_document(cli: &Cli, message: &str) -> Value {
    base_document(
        cli,
        "rejected",
        Value::Null,
        Value::Null,
        json!({
            "kind":"process_failure",
            "message":message
        }),
    )
}

fn base_document(
    cli: &Cli,
    status: &str,
    comparison: Value,
    summary: Value,
    failure: Value,
) -> Value {
    json!({
        "schema":"code-intel-diaphora-observation.v1",
        "status":status,
        "provider":{
            "id":"diaphora",
            "upstreamVersion":cli.provider_version,
            "resultSchemaVersion":Value::Null,
            "mode":"external_result_import"
        },
        "comparison":comparison,
        "observedAt":cli.observed_at,
        "summary":summary,
        "authority":{
            "observationOnly":true,
            "engineeringFacts":[],
            "note":"Diaphora match evidence requires human review before use in a patch-diff conclusion."
        },
        "failure":failure
    })
}

fn emit(cli: &Cli, document: &Value, exit_code: i32) -> i32 {
    let bytes = match serde_json::to_vec(document) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("cannot serialize Diaphora observation");
            return 70;
        }
    };
    if fs::write(&cli.out, &bytes).is_err() {
        eprintln!("cannot write Diaphora observation output");
        return 65;
    }
    println!("{}", String::from_utf8_lossy(&bytes));
    if let Some(message) = document["failure"]["message"].as_str() {
        eprintln!("{message}");
    }
    exit_code
}
