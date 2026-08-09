use std::collections::BTreeSet;
use std::path::{Component, Path};

use serde_json::Value;

use crate::capability::is_digest;

const MAX_TEXT_LENGTH: usize = 4 * 1024;

pub(super) fn require_object_keys(
    value: &Value,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{context} fields are invalid"));
    }
    Ok(())
}

pub(super) fn require_const(
    value: &Value,
    field: &str,
    expected: &str,
    context: &str,
) -> Result<(), String> {
    if value[field].as_str() != Some(expected) {
        return Err(format!("{context}.{field} must be {expected}"));
    }
    Ok(())
}

pub(super) fn object_field<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a Value, String> {
    value[field]
        .as_object()
        .map(|_| &value[field])
        .ok_or_else(|| format!("{context}.{field} must be an object"))
}

pub(super) fn array_field<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a [Value], String> {
    value[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{context}.{field} must be an array"))
}

pub(super) fn text_field<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| {
            !text.is_empty()
                && text.len() <= MAX_TEXT_LENGTH
                && !text.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            format!(
                "{context}.{field} must be a non-empty printable string up to {MAX_TEXT_LENGTH} bytes"
            )
        })
}

pub(super) fn identifier_field<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    let identifier = text_field(value, field, context)?;
    if !identifier.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
    }) {
        return Err(format!(
            "{context}.{field} must use identifier characters only"
        ));
    }
    Ok(identifier)
}

pub(super) fn digest_field<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    let digest = text_field(value, field, context)?;
    if !is_digest(digest) {
        return Err(format!(
            "{context}.{field} must be a lowercase SHA-256 digest"
        ));
    }
    Ok(digest)
}

pub(super) fn revision_field<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    let revision = text_field(value, field, context)?;
    if !matches!(revision.len(), 40 | 64)
        || !revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{context}.{field} must be a lowercase 40- or 64-character git revision"
        ));
    }
    Ok(revision)
}

pub(super) fn positive_integer_field(
    value: &Value,
    field: &str,
    context: &str,
) -> Result<u64, String> {
    value[field]
        .as_u64()
        .filter(|line| *line > 0)
        .ok_or_else(|| format!("{context}.{field} must be a positive integer"))
}

pub(super) fn enum_field<'a>(
    value: &'a Value,
    field: &str,
    allowed: &[&str],
    context: &str,
) -> Result<&'a str, String> {
    let actual = text_field(value, field, context)?;
    if !allowed.contains(&actual) {
        return Err(format!("{context}.{field} has an unsupported value"));
    }
    Ok(actual)
}

pub(super) fn repo_relative_file<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, String> {
    let file = text_field(value, field, context)?;
    let path = Path::new(file);
    if path.components().any(|component| {
        matches!(
            component,
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "{context}.{field} must be repository-relative without '..'"
        ));
    }
    Ok(file)
}
