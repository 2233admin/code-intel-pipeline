use serde_json::{Map, Value};

// ---------------------------------------------------------------------
// Generic JSON-object helpers. `closed_object` is this module's
// `additionalProperties: false`: every key must be declared required or
// optional, and every required key must be present.
// ---------------------------------------------------------------------

fn as_object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))
}

pub(super) fn closed_object<'a>(
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
    context: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = as_object(value, context)?;
    for key in required {
        if !object.contains_key(*key) {
            return Err(format!("{context} is missing required field \"{key}\""));
        }
    }
    for key in object.keys() {
        let key = key.as_str();
        if !required.contains(&key) && !optional.contains(&key) {
            return Err(format!("{context} has an unrecognized field \"{key}\""));
        }
    }
    Ok(object)
}

pub(super) fn required_str(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{context}.{key} must be a non-empty string"))
}

pub(super) fn optional_str(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string when present")),
    }
}

/// An optional nested object: `None` when the key is absent or JSON `null`,
/// otherwise the raw `Value` for the caller to parse further (e.g. via
/// another `*_from_value` constructor, which enforces its own shape with
/// `closed_object`).
pub(super) fn optional_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a Value>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(value)),
    }
}

pub(super) fn required_nullable_string(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Err(format!("{context} is missing required field \"{key}\"")),
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{context}.{key} must be a string or null")),
    }
}

pub(super) fn required_bool(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{context}.{key} must be a boolean"))
}

pub(super) fn required_nullable_number(
    object: &Map<String, Value>,
    key: &str,
    min: f64,
    max: f64,
    context: &str,
) -> Result<Option<f64>, String> {
    match object.get(key) {
        None => Err(format!("{context} is missing required field \"{key}\"")),
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| *number >= min && *number <= max)
            .map(Some)
            .ok_or_else(|| format!("{context}.{key} must be a number between {min} and {max}")),
    }
}

pub(super) fn optional_nullable_uint_min(
    object: &Map<String, Value>,
    key: &str,
    min: u64,
    context: &str,
) -> Result<Option<u64>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|number| *number >= min)
            .map(Some)
            .ok_or_else(|| format!("{context}.{key} must be an integer >= {min}")),
    }
}

pub(super) fn required_string_array(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Vec<String>, String> {
    let items = object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}.{key} must be an array"))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{context}.{key} entries must be non-empty strings"))
        })
        .collect()
}

pub(super) fn optional_string_array(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Vec<String>, String> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(value) => {
            let items = value
                .as_array()
                .ok_or_else(|| format!("{context}.{key} must be an array"))?;
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| format!("{context}.{key} entries must be non-empty strings"))
                })
                .collect()
        }
    }
}

pub(super) fn required_enum<T>(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, String> {
    let raw = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context}.{key} must be a string"))?;
    parse(raw).ok_or_else(|| format!("{context}.{key} has an unrecognized value \"{raw}\""))
}

pub(super) fn optional_nullable_enum<T>(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => parse(raw)
            .map(Some)
            .ok_or_else(|| format!("{context}.{key} has an unrecognized value \"{raw}\"")),
        Some(_) => Err(format!("{context}.{key} must be a string or null")),
    }
}
