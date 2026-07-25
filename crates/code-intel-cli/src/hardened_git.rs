//! Git launched against a repository the operator pointed at, not one this
//! process owns.
//!
//! Every `git` invocation in this pipeline runs *inside* the scanned tree, so
//! that tree's own `.git/config` is live for the command — and several Git
//! config keys name a program Git will execute. `core.fsmonitor` is the sharp
//! one: Git runs it on ordinary read commands like `git status`, before any
//! gate in this pipeline has looked at the repository. `git clone` does not
//! transfer config, so the precondition is a repository that arrived with its
//! `.git` intact (an archive, a backup, a copied directory) — exactly the
//! shape a scanned target can have.
//!
//! The pipeline already applies this standard to ripgrep (`RIPGREP_CONFIG_PATH`
//! is stripped at every call site); this is the same rule for the tool that can
//! start a process rather than merely change a file list.

use std::{path::Path, process::Command};

/// Config keys that make Git execute a program, each pinned to an empty value
/// so a repository-supplied setting cannot take effect. `-c` beats every
/// config file, so this holds regardless of what the target's `.git/config`,
/// the user's global config, or the system config says.
const DISARMED_KEYS: [&str; 5] = [
    // Run by ordinary read commands (status, diff) when set.
    "core.fsmonitor",
    // Hook directory: neutralized so a copied .git cannot supply hooks.
    "core.hooksPath",
    // Program substitutions Git will spawn for content or transport.
    "core.sshCommand",
    "diff.external",
    "core.pager",
];

/// A `git` command with the target repository's program-executing config keys
/// disarmed. Callers add their own subcommand and arguments; `repo` is where
/// the command runs.
pub(crate) fn command(repo: &Path) -> Command {
    let mut command = Command::new("git");
    for key in DISARMED_KEYS {
        command.arg("-c").arg(format!("{key}="));
    }
    // Ignore the system-wide config as well: it is not attacker-controlled in
    // the threat model above, but it can silently reintroduce a substitution
    // this list disarms.
    command.env_remove("GIT_CONFIG_SYSTEM");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.current_dir(repo);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn disarms_every_program_executing_config_key() {
        let command = command(&repo_root());
        let rendered = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        for key in DISARMED_KEYS {
            assert!(rendered.contains(&format!("{key}=")), "{key} not disarmed");
        }
    }

    /// The hardening must not cost the caller the command's actual output:
    /// a disarmed `git` still answers ordinary read commands.
    #[test]
    fn still_runs_a_read_command_against_a_real_repository() {
        let output = command(&repo_root())
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .expect("git launches");
        assert!(output.status.success(), "git failed under hardening");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    }
}
