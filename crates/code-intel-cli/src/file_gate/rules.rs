//! Gate names, the shared extension/directory rule data, the per-candidate
//! types, and the pure (no filesystem walking) predicate logic that decides
//! whether one candidate path or file matches one gate. `mod.rs::evaluate`
//! is the only caller of the non-`pub(crate)` items here; see that file for
//! the declared evaluation order.

use std::fs;
use std::io::Read;
use std::path::Path;

pub(crate) const GATE_UNSUPPORTED_EXT: &str = "unsupported_ext";
pub(crate) const GATE_USER_INCLUDE: &str = "user_include";
pub(crate) const GATE_USER_EXCLUDE: &str = "user_exclude";
pub(crate) const GATE_DEFAULT_PATH: &str = "default_path";
pub(crate) const GATE_OVERSIZED: &str = "oversized";
pub(crate) const GATE_REPOSITORY_IGNORED: &str = "repository_ignored";
pub(crate) const GATE_BINARY: &str = "binary";
pub(crate) const GATE_DEFAULT_INCLUDE: &str = "default_include";

/// Declared precedence, first match wins. See the module doc on
/// `file_gate::mod` for the full rationale behind this exact order.
pub(crate) const GATE_ORDER: [&str; 8] = [
    GATE_UNSUPPORTED_EXT,
    GATE_USER_INCLUDE,
    GATE_USER_EXCLUDE,
    GATE_DEFAULT_PATH,
    GATE_OVERSIZED,
    GATE_REPOSITORY_IGNORED,
    GATE_BINARY,
    GATE_DEFAULT_INCLUDE,
];

pub(crate) const SOURCE_BUILT_IN: &str = "built_in";
pub(crate) const SOURCE_PROJECT: &str = "project";

/// The canonical, shared set of source-code extensions (no leading dot,
/// lowercase). Union of what `sentrux scan` and `sentrux dsm` each
/// recognised separately before #152: `sentrux dsm`'s list was missing
/// `cpp`/`c`/`h`/`hpp`, silently dropping such files with no accounting at
/// all. `every_extension_is_only_classified_once` guards against silent
/// duplication.
pub(crate) const CODE_EXTENSIONS: &[&str] = &[
    "ps1", "psm1", "py", "rs", "go", "ts", "tsx", "js", "jsx", "mjs", "cjs", "java", "cs", "cpp",
    "c", "h", "hpp", "v",
];

/// Directory names default-excluded at *any* depth, not just the repository
/// root. Before #152, `sentrux dsm` checked `tools`/`vendor`/`third_party`/
/// `external` only against the first path segment, so `legacy/tools/*` was
/// invisible to that check -- the majority contributor to issue #148 C2's
/// 277/315 split on this repository's own self-scan. `sentrux scan`'s
/// `SKIP_DIRECTORIES` already matched at any depth; this list ports that
/// (superset, including the `vendors` / `third-party` spelling variants)
/// so both commands share one rule.
pub(crate) const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    ".git",
    ".repowise",
    ".understand-anything",
    ".sentrux",
    "tools",
    "vendor",
    "vendors",
    "third_party",
    "third-party",
    "external",
    "node_modules",
    ".pnpm",
    ".yarn",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".venv",
    "venv",
    "env",
    ".tox",
    "__pycache__",
    ".next",
    ".nuxt",
    ".turbo",
    ".cache",
];

pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Bytes read from the front of a candidate file to decide `binary`: enough
/// to catch a NUL byte in any realistic non-text blob without paying for a
/// full read of a large file. A text file never legitimately contains a NUL
/// in its first few KB; this is the same family of heuristic git itself
/// uses to decide whether to treat a blob as text.
const BINARY_SNIFF_BYTES: usize = 8000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Decision {
    Included,
    Excluded,
}

impl Decision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Decision::Included => "included",
            Decision::Excluded => "excluded",
        }
    }
}

/// One candidate file's judgment: `{path, decision, gate, source}`
/// (issue #152 requirement 1). `gate` and `source` are always one of the
/// `GATE_*` / `SOURCE_*` constants above -- never a free-form log string.
#[derive(Clone, Debug)]
pub(crate) struct GateDecision {
    pub(crate) path: String,
    pub(crate) decision: Decision,
    pub(crate) gate: &'static str,
    pub(crate) source: &'static str,
}

/// Configuration for the gate chain. See the module doc's "Where each
/// gate's rule comes from" section for why `user_exclude`/`user_include`
/// are always empty from [`GateConfig::built_in`].
#[derive(Default, Clone)]
pub(crate) struct GateConfig {
    pub(crate) user_exclude: Vec<String>,
    pub(crate) user_include: Vec<String>,
}

impl GateConfig {
    pub(crate) fn built_in() -> Self {
        Self::default()
    }
}

pub(crate) fn matches_pattern(relative: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|raw| {
        let pattern = raw.trim().trim_matches('/');
        !pattern.is_empty() && (relative == pattern || relative.starts_with(&format!("{pattern}/")))
    })
}

pub(crate) fn extension_of(relative: &str) -> String {
    relative
        .rsplit('/')
        .next()
        .and_then(|leaf| leaf.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The built-in default path exclusion: any directory component matching
/// [`DEFAULT_EXCLUDE_DIRS`] at any depth, any hidden directory (`.foo`), a
/// `.venv-*`/`venv-*`/`env-*` prefixed directory (all ported from
/// `sentrux scan`'s pre-#152 traversal-time skip), or the bundled/static
/// asset leaf-name convention below.
pub(crate) fn default_path_match(relative: &str) -> bool {
    let lowered = relative.to_ascii_lowercase();
    let mut parts: Vec<&str> = lowered.split('/').collect();
    parts.pop(); // the leaf file name itself never gates on this rule
    for part in parts {
        if DEFAULT_EXCLUDE_DIRS.contains(&part)
            || (part.starts_with('.') && part.len() > 1)
            || part.starts_with(".venv-")
            || part.starts_with("venv-")
            || part.starts_with("env-")
        {
            return true;
        }
    }
    is_bundled_or_static(&lowered)
}

/// Bundled/static-asset and minified-output convention, ported unchanged
/// from `sentrux scan`'s pre-#152 `is_skipped_relative`.
fn is_bundled_or_static(lowered: &str) -> bool {
    if lowered.starts_with("static/")
        || lowered.starts_with("public/")
        || lowered.starts_with("wwwroot/")
        || lowered.contains("/static/")
        || lowered.contains("/public/")
        || lowered.contains("/wwwroot/")
    {
        return true;
    }
    let leaf = lowered.rsplit('/').next().unwrap_or(lowered);
    leaf.ends_with(".min.js") || leaf.ends_with(".bundle.js")
}

pub(crate) fn is_oversized(repo: &Path, relative: &str) -> bool {
    fs::metadata(repo.join(relative))
        .map(|metadata| metadata.len() > MAX_FILE_BYTES)
        .unwrap_or(false)
}

pub(crate) fn looks_binary(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buffer = [0u8; BINARY_SNIFF_BYTES];
    let Ok(read) = file.read(&mut buffer) else {
        return false;
    };
    buffer[..read].contains(&0)
}
