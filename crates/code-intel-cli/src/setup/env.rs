//! Persisting `PATH` and `CODE_INTEL_HOME` for future shells.
//!
//! Follows rustup's pattern. On Unix a generated `env.sh` guards the PATH
//! prefix against duplication, and one `. "<env.sh>"` line is appended once to
//! the shell startup files. On Windows the user-scope registry values are
//! written directly (never `setx`, which truncates at 1024 characters) and a
//! `WM_SETTINGCHANGE` broadcast tells running Explorer sessions to reload.

use std::fs;
use std::path::{Path, PathBuf};

use super::Action;

pub(super) struct Persisted {
    pub(super) actions: Vec<Action>,
    /// What the current shell must run to see the change without a restart.
    pub(super) refresh: Option<String>,
}

#[cfg(not(windows))]
pub(super) fn persist(
    home: &Path,
    bin: &Path,
    code_intel_home: &Path,
    modify_profiles: bool,
) -> Persisted {
    let env_file = config_dir(home).join("env.sh");
    let mut actions = vec![write_env_sh(&env_file, bin, code_intel_home)];
    let source_line = format!(". \"{}\"", env_file.display());
    if modify_profiles {
        for profile in profile_targets(home) {
            actions.push(append_source_line(&profile, &source_line));
        }
    } else {
        actions.push(Action::skipped(
            "env:profile",
            "--no-modify-path: shell startup files were left untouched",
            Some(format!("add `{source_line}` to your shell profile")),
        ));
    }
    Persisted {
        actions,
        refresh: Some(source_line),
    }
}

#[cfg(not(windows))]
fn config_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("code-intel")
}

/// The `env.sh` body: idempotent when sourced repeatedly, so a profile that
/// sources it from several startup files never stacks PATH entries.
#[cfg(not(windows))]
pub(super) fn env_sh(bin: &Path, code_intel_home: &Path) -> String {
    let bin = shell_quote(&bin.display().to_string());
    let home = shell_quote(&code_intel_home.display().to_string());
    format!(
        "#!/bin/sh\n# Written by `code-intel setup`; rewritten on every setup run.\ncase \":${{PATH}}:\" in\n    *:{bin}:*) ;;\n    *) export PATH={bin}\":$PATH\" ;;\nesac\nexport CODE_INTEL_HOME={home}\n"
    )
}

/// Double-quoted shell literal; `$`, `` ` ``, `"` and `\` are escaped.
#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        if matches!(character, '$' | '`' | '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

#[cfg(not(windows))]
fn write_env_sh(path: &Path, bin: &Path, code_intel_home: &Path) -> Action {
    let body = env_sh(bin, code_intel_home);
    if fs::read_to_string(path).is_ok_and(|current| current == body) {
        return Action::already_present("env:file", &path.display().to_string());
    }
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(path, &body));
    match written {
        Ok(()) => Action::installed("env:file", &path.display().to_string()),
        Err(error) => Action::failed(
            "env:file",
            &format!("write {}: {error}", path.display()),
            None,
        ),
    }
}

/// rustup's set: `~/.profile` always; `~/.bashrc` and `~/.bash_profile` when
/// they exist; zsh's `.zshenv` (in `$ZDOTDIR` if set) when it exists or zsh
/// is installed, because zsh never reads `~/.profile`.
#[cfg(not(windows))]
fn profile_targets(home: &Path) -> Vec<PathBuf> {
    let mut targets = vec![home.join(".profile")];
    for name in [".bashrc", ".bash_profile"] {
        let path = home.join(name);
        if path.is_file() {
            targets.push(path);
        }
    }
    let zdotdir = std::env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.to_path_buf());
    let zshenv = zdotdir.join(".zshenv");
    let zsh_installed = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.is_absolute() && dir.join("zsh").is_file())
    });
    if zshenv.is_file() || zsh_installed {
        targets.push(zshenv);
    }
    targets
}

#[cfg(not(windows))]
fn append_source_line(profile: &Path, line: &str) -> Action {
    let name = format!(
        "env:profile:{}",
        profile
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    let current = match fs::read_to_string(profile) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Action::failed(&name, &format!("read {}: {error}", profile.display()), None)
        }
    };
    if current.lines().any(|existing| existing.trim() == line) {
        return Action::already_present(&name, &profile.display().to_string());
    }
    let mut updated = current;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(line);
    updated.push('\n');
    match fs::write(profile, updated) {
        Ok(()) => Action::installed(&name, &profile.display().to_string()),
        Err(error) => Action::failed(
            &name,
            &format!("write {}: {error}", profile.display()),
            Some(format!("add `{line}` to {} by hand", profile.display())),
        ),
    }
}

#[cfg(windows)]
pub(super) fn persist(
    _home: &Path,
    bin: &Path,
    code_intel_home: &Path,
    modify_profiles: bool,
) -> Persisted {
    if !modify_profiles {
        return Persisted {
            actions: vec![Action::skipped(
                "env:registry",
                "--no-modify-path: the user environment was left untouched",
                Some(format!(
                    "add {} to your user PATH and set CODE_INTEL_HOME={}",
                    bin.display(),
                    code_intel_home.display()
                )),
            )],
            refresh: None,
        };
    }
    let actions = match windows::persist(bin, code_intel_home) {
        Ok(actions) => actions,
        Err(error) => vec![Action::failed(
            "env:registry",
            &format!("write HKCU\\Environment: {error}"),
            None,
        )],
    };
    Persisted {
        actions,
        refresh: Some("open a new terminal to pick up PATH and CODE_INTEL_HOME".into()),
    }
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::path::Path;

    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::{RegKey, RegValue};

    use super::super::Action;

    pub(super) fn persist(bin: &Path, code_intel_home: &Path) -> io::Result<Vec<Action>> {
        let environment = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
        let bin = bin.display().to_string();
        let mut actions = Vec::new();

        let current: String = environment.get_value("Path").unwrap_or_default();
        let entries = current
            .split(';')
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>();
        if entries
            .first()
            .is_some_and(|first| first.eq_ignore_ascii_case(&bin))
        {
            actions.push(Action::already_present("env:PATH", &bin));
        } else {
            let mut updated = vec![bin.as_str()];
            updated.extend(
                entries
                    .iter()
                    .filter(|entry| !entry.eq_ignore_ascii_case(&bin)),
            );
            set_expand_string(&environment, "Path", &updated.join(";"))?;
            actions.push(Action::installed("env:PATH", &bin));
        }

        let home = code_intel_home.display().to_string();
        let existing: Option<String> = environment.get_value("CODE_INTEL_HOME").ok();
        if existing.as_deref() == Some(home.as_str()) {
            actions.push(Action::already_present("env:CODE_INTEL_HOME", &home));
        } else {
            environment.set_value("CODE_INTEL_HOME", &home)?;
            actions.push(Action::installed("env:CODE_INTEL_HOME", &home));
        }
        broadcast_environment_change();
        Ok(actions)
    }

    /// `Path` stays `REG_EXPAND_SZ` so entries like `%USERPROFILE%\...` keep
    /// expanding; `set_value::<&str>` would silently downgrade it to `REG_SZ`.
    fn set_expand_string(key: &RegKey, name: &str, value: &str) -> io::Result<()> {
        let bytes: Vec<u8> = value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        key.set_raw_value(
            name,
            &RegValue {
                bytes: bytes.into(),
                vtype: RegType::REG_EXPAND_SZ,
            },
        )
    }

    /// Tell Explorer and other top-level windows the environment changed, so
    /// terminals opened afterwards see the new values without a sign-out.
    fn broadcast_environment_change() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };
        let parameter: Vec<u16> = "Environment\0".encode_utf16().collect();
        let mut result = 0usize;
        // SAFETY: `parameter` is a NUL-terminated UTF-16 buffer that outlives
        // the call, and `result` is a valid out-pointer.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                parameter.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            );
        }
    }
}
