//! Installing the Skill where agents look for it.
//!
//! One canonical copy lives at `~/.agents/skills/code-intel-pipeline`; the
//! Claude Code and Codex skill directories link to it. A link that cannot be
//! created (Windows without symlink privilege) falls back to a copy. A path
//! that is already occupied by something else is moved aside, never deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Action;

pub(super) const SKILL_NAME: &str = "code-intel-pipeline";

pub(super) fn install(home: &Path, release_root: &Path) -> Vec<Action> {
    let source = release_root.join("skills").join(SKILL_NAME);
    if !source.join("SKILL.md").is_file() {
        return vec![Action::failed(
            "skill:source",
            &format!("release has no {}", source.join("SKILL.md").display()),
            Some("pass --release-root pointing at an extracted release".into()),
        )];
    }
    let canonical = home.join(".agents").join("skills").join(SKILL_NAME);
    let mut actions = vec![install_canonical(&source, &canonical)];
    if actions[0].is_failed() {
        return actions;
    }
    for (name, directory) in [("skill:claude", ".claude"), ("skill:codex", ".codex")] {
        let target = home.join(directory).join("skills").join(SKILL_NAME);
        actions.push(link(name, &canonical, &target));
    }
    actions
}

fn install_canonical(source: &Path, canonical: &Path) -> Action {
    match (tree_digest(source), tree_digest(canonical)) {
        (Ok(wanted), Ok(present)) if wanted == present => {
            return Action::already_present("skill:source", &canonical.display().to_string())
        }
        (Err(error), _) => {
            return Action::failed(
                "skill:source",
                &format!("read {}: {error}", source.display()),
                None,
            )
        }
        _ => {}
    }
    match replace_tree(source, canonical) {
        Ok(()) => Action::installed("skill:source", &canonical.display().to_string()),
        Err(error) => Action::failed(
            "skill:source",
            &format!("install {}: {error}", canonical.display()),
            None,
        ),
    }
}

/// Copy into a sibling staging directory, then swap it in, so an interrupted
/// setup never leaves a half-written skill where agents read it.
fn replace_tree(source: &Path, destination: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent"))?;
    fs::create_dir_all(parent)?;
    let nonce = std::process::id();
    let staging = parent.join(format!(".{SKILL_NAME}.staging-{nonce}"));
    let backup = parent.join(format!(".{SKILL_NAME}.backup-{nonce}"));
    remove_any(&staging)?;
    copy_tree(source, &staging)?;
    let had_previous = fs::symlink_metadata(destination).is_ok();
    if had_previous {
        remove_any(&backup)?;
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if had_previous {
            let _ = fs::rename(&backup, destination);
        }
        let _ = remove_any(&staging);
        return Err(error);
    }
    if had_previous {
        remove_any(&backup)?;
    }
    Ok(())
}

fn link(name: &str, canonical: &Path, target: &Path) -> Action {
    if serves(target, canonical) {
        return Action::already_present(name, &target.display().to_string());
    }
    let moved = match move_aside(target) {
        Ok(moved) => moved,
        Err(error) => {
            return Action::failed(name, &format!("free {}: {error}", target.display()), None)
        }
    };
    let created = target
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| symlink_dir(canonical, target));
    let detail = match &moved {
        Some(aside) => format!(
            "{} (previous content moved to {})",
            target.display(),
            aside.display()
        ),
        None => target.display().to_string(),
    };
    match created {
        Ok(()) => Action::installed(name, &detail),
        // No symlink privilege (Windows without Developer Mode): serve a copy.
        Err(_) => match copy_tree(canonical, target) {
            Ok(()) => Action::installed(name, &format!("{detail} (copied; links unavailable)")),
            Err(error) => Action::failed(
                name,
                &format!("link or copy {}: {error}", target.display()),
                None,
            ),
        },
    }
}

/// A link resolving to `canonical`, or a plain copy with identical contents.
fn serves(target: &Path, canonical: &Path) -> bool {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(target).ok() == fs::canonicalize(canonical).ok()
        }
        Ok(metadata) if metadata.is_dir() => {
            matches!((tree_digest(target), tree_digest(canonical)), (Ok(a), Ok(b)) if a == b)
        }
        _ => false,
    }
}

/// A stale link is just unlinked; real content is renamed next to itself.
fn move_aside(target: &Path) -> io::Result<Option<PathBuf>> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        remove_any(target)?;
        return Ok(None);
    }
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut suffix = 0u32;
    let aside = loop {
        let candidate = target.with_file_name(format!(
            "{file_name}.replaced-{:08x}",
            std::process::id().wrapping_add(suffix)
        ));
        if fs::symlink_metadata(&candidate).is_err() {
            break candidate;
        }
        suffix += 1;
    };
    fs::rename(target, &aside)?;
    Ok(Some(aside))
}

#[cfg(unix)]
fn symlink_dir(source: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, link)
}

#[cfg(windows)]
fn symlink_dir(source: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, link)
}

fn remove_any(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
        }
        Ok(_) => fs::remove_file(path).or_else(|_| fs::remove_dir(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn skip(name: &str) -> bool {
    name == "__pycache__" || name.ends_with(".pyc")
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip(&name.to_string_lossy()) {
            continue;
        }
        let from = entry.path();
        let to = destination.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Relative path and SHA-256 of every file, sorted: equal digests mean the
/// two trees serve identical skill content.
fn tree_digest(root: &Path) -> io::Result<String> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries)?;
    entries.sort();
    Ok(crate::capability::sha256_hex(entries.join("\n").as_bytes()))
}

fn collect(root: &Path, directory: &Path, entries: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if skip(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if fs::metadata(&path)?.is_dir() {
            collect(root, &path, entries)?;
        } else {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            entries.push(format!(
                "{} {}",
                relative.to_string_lossy().replace('\\', "/"),
                crate::capability::sha256_hex(&fs::read(&path)?)
            ));
        }
    }
    Ok(())
}
