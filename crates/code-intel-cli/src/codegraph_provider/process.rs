use std::borrow::Cow;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

use super::Failure;

pub(super) struct Engine {
    executable: PathBuf,
    command: PathBuf,
    prefix: Vec<String>,
    sdk: Option<(PathBuf, PathBuf)>,
    pub identity: Value,
}

impl Engine {
    pub fn discover(repo: &Path) -> Result<Self, Failure> {
        let executable = match env::var_os("CODE_INTEL_CODEGRAPH_BIN") {
            Some(value) => {
                let path = PathBuf::from(value);
                if !path.is_absolute() { return Err(Failure::usage("CODE_INTEL_CODEGRAPH_BIN must be absolute")); }
                if !path.is_file() { return Err(Failure::unavailable(format!("CodeGraph executable unavailable: {}", path.display()))); }
                path
            }
            None => super::tool_path::locate("codegraph", None).ok_or_else(|| Failure::unavailable("CodeGraph is not installed on absolute PATH; install official CodeGraph >=1.6.2"))?,
        };
        let executable = fs::canonicalize(executable)
            .map_err(|e| Failure::io(format!("resolve CodeGraph executable: {e}")))?;
        if executable.starts_with(repo) {
            return Err(Failure::contract(
                "CodeGraph executable must not resolve inside the source repository",
            ));
        }
        let parent = executable
            .parent()
            .ok_or_else(|| Failure::contract("CodeGraph executable has no parent directory"))?;
        let npm_root = if executable
            .file_name()
            .is_some_and(|name| name == "npm-shim.js")
        {
            Some(parent.to_path_buf())
        } else {
            let root = parent.join("node_modules/@colbymchenry/codegraph");
            root.join("npm-shim.js").is_file().then_some(root)
        };
        let platform = if cfg!(windows) {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let package = format!("codegraph-{platform}-{arch}");
        let mut bundles = Vec::new();
        if let Some(root) = &npm_root {
            bundles.push(root.join("node_modules/@colbymchenry").join(&package));
            if let Some(namespace) = root.parent() {
                bundles.push(namespace.join(&package));
            }
            if let Ok(text) = fs::read_to_string(root.join("package.json")) {
                if let Ok(metadata) = serde_json::from_str::<Value>(&text) {
                    if let Some(version) = metadata["version"].as_str().filter(|version| {
                        !version.is_empty()
                            && version.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+')
                            })
                    }) {
                        let install_root = env::var_os("CODEGRAPH_INSTALL_DIR")
                            .map(PathBuf::from)
                            .or_else(|| {
                                env::var_os("HOME")
                                    .or_else(|| env::var_os("USERPROFILE"))
                                    .map(|home| PathBuf::from(home).join(".codegraph"))
                            });
                        if let Some(install_root) = install_root.filter(|path| path.is_absolute()) {
                            bundles.push(
                                install_root
                                    .join("bundles")
                                    .join(format!("{platform}-{arch}-{version}")),
                            );
                        }
                    }
                }
            }
        }
        bundles.push(parent.to_path_buf());
        if let Some(root) = parent.parent() {
            bundles.push(root.to_path_buf());
        }
        let sdk = bundles.into_iter().find_map(|root| {
            let node = root.join(if cfg!(windows) { "node.exe" } else { "node" });
            let entry = root.join("lib/dist/index.js");
            (node.is_file() && entry.is_file()).then_some((node, entry))
        });
        if let Some((node, entry)) = &sdk {
            for path in [
                node.clone(),
                entry.clone(),
                entry.parent().unwrap().join("bin/codegraph.js"),
            ] {
                let resolved = fs::canonicalize(&path).map_err(|e| {
                    Failure::io(format!(
                        "resolve installed CodeGraph bundle {}: {e}",
                        path.display()
                    ))
                })?;
                if resolved.starts_with(repo) {
                    return Err(Failure::contract(
                        "CodeGraph runtime/entry must not resolve inside the source repository",
                    ));
                }
            }
        }
        let cmd = executable.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
        let (command, prefix) = if cmd {
            // Do not invoke cmd.exe: queries must never pass through its expansion,
            // metacharacter or percent-variable interpretation. Only recognized
            // official bundle layouts can replace an npm/bundle command shim.
            let (node, entry) = sdk.as_ref().ok_or_else(|| Failure::unavailable("official CodeGraph command shim has no installed bundled Node/entry; network fallback is disabled"))?;
            let cli = entry.parent().unwrap().join("bin/codegraph.js");
            let shim = fs::read_to_string(&executable)
                .map_err(|e| Failure::io(format!("read CodeGraph command shim: {e}")))?;
            if !cli.is_file() || !(shim.contains("npm-shim.js") || shim.contains("codegraph.js")) {
                return Err(Failure::contract(
                    "unrecognized CodeGraph Windows command shim",
                ));
            }
            (
                node.clone(),
                vec![
                    "--liftoff-only".into(),
                    "--disable-warning=ExperimentalWarning".into(),
                    argument_path(&cli).into_owned(),
                ],
            )
        } else if executable
            .file_name()
            .is_some_and(|name| name == "npm-shim.js")
            && sdk.is_some()
        {
            let (node, entry) = sdk.as_ref().unwrap();
            (
                node.clone(),
                vec![
                    "--liftoff-only".into(),
                    "--disable-warning=ExperimentalWarning".into(),
                    argument_path(&entry.parent().unwrap().join("bin/codegraph.js")).into_owned(),
                ],
            )
        } else {
            (executable.clone(), Vec::new())
        };
        if !command.is_absolute() {
            return Err(Failure::contract("CodeGraph launcher must be absolute"));
        }
        let mut digests = vec![json!({"path":executable,"sha256":hash_file(&executable)?})];
        if command != executable {
            digests.push(json!({"path":command,"sha256":hash_file(&command)?}));
        }
        if let Some((_, entry)) = &sdk {
            for path in [
                entry.clone(),
                entry.parent().unwrap().join("bin/codegraph.js"),
                entry.parent().unwrap().join("../package.json"),
            ] {
                if path.is_file() {
                    digests.push(json!({"path":path,"sha256":hash_file(&path)?}));
                }
            }
        }
        let digest = crate::capability::sha256_hex(
            &serde_json::to_vec(&digests).map_err(|e| Failure::contract(e.to_string()))?,
        );
        let mut engine = Self {
            executable,
            command,
            prefix,
            sdk,
            identity: Value::Null,
        };
        let version_output = engine.invoke(repo, &["--version".into()])?;
        if !version_output.status.success() {
            return Err(Failure::process(
                "CodeGraph version probe failed",
                &version_output,
            ));
        }
        let version_text = std::str::from_utf8(&version_output.stdout)
            .map_err(|e| Failure::contract(format!("CodeGraph version is not UTF-8: {e}")))?
            .trim();
        let version = version_text.strip_prefix('v').unwrap_or(version_text);
        let release = version.split_once('-').map_or(version, |(base, _)| base);
        let parts = release
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Failure::contract("malformed CodeGraph semantic version"))?;
        if parts.len() != 3
            || parts.as_slice() < [1, 6, 2].as_slice()
            || (parts.as_slice() == [1, 6, 2].as_slice() && version.contains('-'))
        {
            return Err(Failure::contract(format!(
                "CodeGraph version {version} is below supported stable floor 1.6.2 or malformed"
            )));
        }
        engine.identity = json!({"id":"codegraph.cli","implementation":{"id":"colbymchenry/codegraph","version":version,"digest":digest},"executable":engine.executable,"launcher":engine.command,"identityFiles":digests,"digestScope":"launcher/runtime/entry/package identity; not a claim of all transitive dependency bytes"});
        Ok(engine)
    }

    fn command(&self, repo: &Path, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(repo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("DO_NOT_TRACK", "1")
            .env("CODEGRAPH_TELEMETRY", "0")
            .env("CODEGRAPH_NO_UPDATE_CHECK", "1")
            .env("CODEGRAPH_NO_DOWNLOAD", "1")
            .env("CODEGRAPH_NO_DAEMON", "1")
            .env("CODEGRAPH_NO_WATCH", "1")
            .env("CODEGRAPH_DIR", ".codegraph")
            .env("NO_COLOR", "1");
        command
    }

    pub fn invoke(&self, repo: &Path, args: &[String]) -> Result<Output, Failure> {
        self.command(repo, &self.command)
            .args(&self.prefix)
            .args(args)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Failure::unavailable(format!("CodeGraph launcher unavailable: {e}"))
                } else {
                    Failure::new(70, format!("launch CodeGraph: {e}"))
                }
            })
    }

    pub fn invocation(&self, args: &[String]) -> Value {
        json!({"executable":self.command,"argv":self.prefix.iter().chain(args.iter()).collect::<Vec<_>>(),"discoveredExecutable":self.executable})
    }

    pub fn initial_index(&self, repo: &Path) -> Result<Output, Failure> {
        let (node, entry) = self.sdk.as_ref().ok_or_else(|| Failure::contract("initial index needs the official installed SDK bundle; initialize externally without hooks or install the official bundled distribution"))?;
        // Fixed transport glue only: upstream owns extraction, indexing and
        // writer locking. No query/path interpolation in JavaScript source.
        const SCRIPT: &str = "(async()=>{const entry=process.argv[1],repo=process.argv[2];const path=require('node:path');const locks=require(path.join(path.dirname(entry),'mcp/writer-lock.js'));const lock=locks.tryAcquireWriterLock(repo,'index');if(lock.kind==='taken')throw new Error(locks.writerLockHeldMessage(lock.existing,lock.pidPath));let graph;try{const CG=require(entry).default;graph=await CG.init(repo,{index:false});const result=await graph.indexAll();console.log(JSON.stringify(result));}finally{if(graph)graph.destroy();locks.releaseWriterLock(repo);}})().catch(e=>{console.error(e&&e.stack||String(e));process.exitCode=1;});";
        self.command(repo, node)
            .args([
                "--liftoff-only",
                "--disable-warning=ExperimentalWarning",
                "-e",
                SCRIPT,
                "--",
            ])
            .arg(argument_path(entry).as_ref())
            .arg(argument_path(repo).as_ref())
            .output()
            .map_err(|e| Failure::new(70, format!("launch CodeGraph SDK initialization: {e}")))
    }

    pub fn indexed_files(&self, repo: &Path) -> Result<Value, Failure> {
        let (node, entry) = self.sdk.as_ref().ok_or_else(|| Failure::contract("snapshot binding requires the official installed SDK bundle for its indexed file inventory"))?;
        const SCRIPT: &str = "(async()=>{const CG=require(process.argv[1]).default;const graph=await CG.open(process.argv[2],{readOnly:true});try{console.log(JSON.stringify(graph.getFiles()));}finally{graph.destroy();}})().catch(e=>{console.error(e&&e.stack||String(e));process.exitCode=1;});";
        let output = self
            .command(repo, node)
            .args([
                "--liftoff-only",
                "--disable-warning=ExperimentalWarning",
                "-e",
                SCRIPT,
                "--",
            ])
            .arg(argument_path(entry).as_ref())
            .arg(argument_path(repo).as_ref())
            .output()
            .map_err(|error| {
                Failure::new(70, format!("read CodeGraph SDK indexed inventory: {error}"))
            })?;
        if !output.status.success() {
            return Err(Failure::process(
                "CodeGraph SDK indexed inventory failed",
                &output,
            ));
        }
        let text = std::str::from_utf8(&output.stdout).map_err(|error| {
            Failure::contract(format!("CodeGraph indexed inventory is not UTF-8: {error}"))
        })?;
        crate::capability::reject_duplicate_json_keys_within(text, 64 * 1024 * 1024)
            .map_err(Failure::contract)?;
        serde_json::from_str(text).map_err(|error| {
            Failure::contract(format!("CodeGraph indexed inventory is malformed: {error}"))
        })
    }
}

// Keep canonical/verbatim paths for Rust identity and containment checks.
// Node's Windows main-module resolver requires conventional drive/UNC spelling.
pub(super) fn argument_path(path: &Path) -> Cow<'_, str> {
    let text = path.to_string_lossy();
    if !cfg!(windows) {
        return text;
    }
    match text {
        Cow::Borrowed(text) => {
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                Cow::Owned(format!(r"\\{rest}"))
            } else {
                Cow::Borrowed(text.strip_prefix(r"\\?\").unwrap_or(text))
            }
        }
        Cow::Owned(mut text) => {
            if text.starts_with(r"\\?\UNC\") {
                text.replace_range(..8, r"\\");
            } else if text.starts_with(r"\\?\") {
                text.drain(..4);
            }
            Cow::Owned(text)
        }
    }
}

pub(super) fn hash_file(path: &Path) -> Result<String, Failure> {
    let mut file = fs::File::open(path).map_err(|e| {
        Failure::io(format!(
            "read implementation/cache identity {}: {e}",
            path.display()
        ))
    })?;
    let before = file.metadata().map_err(|e| Failure::io(e.to_string()))?;
    if !before.is_file() {
        return Err(Failure::contract(
            "implementation/cache identity must be a regular file",
        ));
    }
    let mut digest = crate::capability::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| {
            Failure::io(format!(
                "read implementation/cache identity {}: {e}",
                path.display()
            ))
        })?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|e| Failure::io(e.to_string()))?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(Failure::contract(
            "implementation/cache bytes changed while hashing",
        ));
    }
    Ok(digest.finish())
}
