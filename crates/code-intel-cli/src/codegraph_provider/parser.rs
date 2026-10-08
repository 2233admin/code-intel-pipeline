use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::Failure;

pub(super) struct Request {
    pub operation: String,
    pub repo: PathBuf,
    pub query: Option<String>,
    pub changed: Vec<String>,
    pub file: Option<String>,
    pub limit: Option<u32>,
    pub depth: Option<u32>,
    pub artifact_root: Option<PathBuf>,
}

impl Request {
    pub fn parse(raw: &[String]) -> Result<Self, Failure> {
        let operation = raw.first().filter(|s| matches!(s.as_str(), "index" | "sync" | "status" | "query" | "explore" | "node" | "callers" | "callees" | "impact" | "affected" | "files"))
            .ok_or_else(|| Failure::usage("expected codegraph operation: index, sync, status, query, explore, node, callers, callees, impact, affected, files"))?.clone();
        let mut request = Self {
            operation,
            repo: PathBuf::new(),
            query: None,
            changed: Vec::new(),
            file: None,
            limit: None,
            depth: None,
            artifact_root: None,
        };
        let mut seen = BTreeSet::new();
        let mut changed = BTreeSet::new();
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--repo"
                    | "--query"
                    | "--changed"
                    | "--file"
                    | "--limit"
                    | "--depth"
                    | "--artifact-root"
            ) {
                return Err(Failure::usage(format!(
                    "unknown codegraph argument: {flag}"
                )));
            }
            if flag != "--changed" && !seen.insert(flag) {
                return Err(Failure::usage(format!(
                    "duplicate codegraph argument: {flag}"
                )));
            }
            let value = raw
                .get(index + 1)
                .filter(|v| {
                    (flag == "--query" || !v.starts_with("--"))
                        && !v.trim().is_empty()
                        && !v.contains('\0')
                })
                .ok_or_else(|| Failure::usage(format!("{flag} requires one nonempty value")))?;
            match flag {
                "--repo" => request.repo = PathBuf::from(value),
                "--query" => request.query = Some(value.clone()),
                "--changed" => {
                    let path = relative(value)?;
                    if !changed.insert(path.to_lowercase()) {
                        return Err(Failure::usage("duplicate --changed path"));
                    }
                    request.changed.push(path);
                }
                "--file" => request.file = Some(relative(value)?),
                "--limit" => request.limit = Some(number(flag, value)?),
                "--depth" => request.depth = Some(number(flag, value)?),
                "--artifact-root" => request.artifact_root = Some(PathBuf::from(value)),
                _ => unreachable!(),
            }
            index += 2;
        }
        if request.repo.as_os_str().is_empty() {
            return Err(Failure::usage("codegraph requires --repo"));
        }
        let op = request.operation.as_str();
        let needs_query = matches!(op, "query" | "explore" | "callers" | "callees" | "impact");
        if needs_query && request.query.is_none() {
            return Err(Failure::usage(format!("{op} requires --query")));
        }
        if op == "node" && request.query.is_none() && request.file.is_none() {
            return Err(Failure::usage("node requires --query or --file"));
        }
        if !needs_query && op != "node" && request.query.is_some() {
            return Err(Failure::usage(format!("{op} does not accept --query")));
        }
        if op == "affected" && request.changed.is_empty() {
            return Err(Failure::usage("affected requires at least one --changed"));
        }
        if op != "affected" && !request.changed.is_empty() {
            return Err(Failure::usage(format!("{op} does not accept --changed")));
        }
        if request.file.is_some()
            && !matches!(op, "node" | "callers" | "callees" | "impact" | "files")
        {
            return Err(Failure::usage(format!("{op} does not accept --file")));
        }
        if request.limit.is_some()
            && !(matches!(op, "query" | "explore" | "callers" | "callees")
                || (op == "node" && request.file.is_some() && request.query.is_none()))
        {
            return Err(Failure::usage(format!(
                "{op} does not accept --limit in this mode"
            )));
        }
        if request.depth.is_some() && !matches!(op, "impact" | "affected") {
            return Err(Failure::usage(format!(
                "{op} does not accept --depth (files JSON ignores upstream tree depth)"
            )));
        }
        if op == "impact" && request.depth.is_some_and(|depth| depth > 10) {
            return Err(Failure::usage(
                "impact --depth must be between 1 and 10; upstream otherwise silently clamps",
            ));
        }
        // Upstream node treats path-like positional arguments as file reads.
        if op == "node" {
            if let Some(query) = &request.query {
                if query.contains('/') || query.contains('\\') {
                    return Err(Failure::usage(
                        "node file reads require explicit --file, not --query",
                    ));
                }
            }
        }
        Ok(request)
    }

    pub fn argv(&self, repo: &Path) -> Vec<String> {
        let op = self.operation.as_str();
        let mut args = vec![op.to_string()];
        if matches!(op, "index" | "sync" | "status") {
            args.push("--".into());
            args.push(super::process::argument_path(repo).into_owned());
            return args;
        }
        args.extend([
            "--path".into(),
            super::process::argument_path(repo).into_owned(),
        ]);
        if matches!(
            op,
            "query" | "callers" | "callees" | "impact" | "affected" | "files"
        ) {
            args.push("--json".into());
        }
        if let Some(file) = &self.file {
            args.extend([
                if op == "files" { "--filter" } else { "--file" }.into(),
                file.clone(),
            ]);
        }
        if let Some(limit) = self.limit {
            args.extend([
                if op == "explore" {
                    "--max-files"
                } else {
                    "--limit"
                }
                .into(),
                limit.to_string(),
            ]);
        }
        if let Some(depth) = self.depth {
            args.extend(["--depth".into(), depth.to_string()]);
        }
        args.push("--".into());
        if let Some(query) = &self.query {
            args.push(query.clone());
        }
        args.extend(self.changed.iter().cloned());
        args
    }

    pub fn json_output(&self) -> bool {
        matches!(
            self.operation.as_str(),
            "query" | "callers" | "callees" | "impact" | "affected" | "files"
        )
    }
    pub fn writes_index(&self) -> bool {
        matches!(self.operation.as_str(), "index" | "sync")
    }
}

fn number(flag: &str, value: &str) -> Result<u32, Failure> {
    if !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(Failure::usage(format!(
            "{flag} requires a positive integer"
        )));
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| Failure::usage(format!("{flag} requires a positive 32-bit integer")))
}

fn relative(value: &str) -> Result<String, Failure> {
    let portable = value.replace('\\', "/");
    if portable.contains(':')
        || portable.starts_with('/')
        || portable
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(&portable)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Failure::usage(
            "--file/--changed requires a normalized repository-relative path without traversal",
        ));
    }
    Ok(portable)
}
