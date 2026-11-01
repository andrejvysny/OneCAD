//! Client-side checks for the Create Component form. They mirror the library's own rules
//! (`onecad_library::package`) so a refusal is explained before anything is sent; the
//! service still decides.
use onecad_library::package::{MAX_CONFIGURATION_NAME_LEN, is_valid_configuration_name};

const MAX_ATTACHMENT_NAME: usize = 32;

/// Lower-case, dotted and path-safe: the id names the package directory.
pub(crate) fn id_error(id: &str) -> Option<String> {
    if id.is_empty() {
        return Some("Enter an id such as user.bracket".into());
    }
    if !id.contains('.') {
        return Some("The id must be namespaced, e.g. user.bracket".into());
    }
    let safe = id
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'));
    if !safe {
        return Some("Use lower-case letters, digits, '.', '_' and '-' only".into());
    }
    if id.split('.').any(str::is_empty) {
        return Some("The id cannot start or end with '.' or contain '..'".into());
    }
    None
}

/// A semver triple: three dot-separated numbers.
pub(crate) fn version_error(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    let numbers = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (!numbers).then(|| "Use a version like 1.0.0".into())
}

pub(crate) fn name_error(name: &str) -> Option<String> {
    name.trim().is_empty().then(|| "Enter a name".into())
}

/// A starting id from the display name: `user.` plus its lower-case slug.
pub(crate) fn suggest_id(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    if slug.is_empty() {
        "user.component".into()
    } else {
        format!("user.{slug}")
    }
}

pub(crate) fn configuration_name_error<'a>(
    name: &str,
    others: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    if !is_valid_configuration_name(name) {
        return Some(format!(
            "1 to {MAX_CONFIGURATION_NAME_LEN} characters of letters, digits, space, '_', '.', '-'"
        ));
    }
    others
        .into_iter()
        .any(|other| other == name)
        .then(|| "Another configuration has this name".into())
}

pub(crate) fn attachment_name_error<'a>(
    name: &str,
    others: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let ok = !name.is_empty()
        && name.len() <= MAX_ATTACHMENT_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    if !ok {
        return Some(format!(
            "1 to {MAX_ATTACHMENT_NAME} letters, digits, '_' or '-'"
        ));
    }
    others
        .into_iter()
        .any(|other| other == name)
        .then(|| "Another attachment has this name".into())
}

/// A configuration value: a finite number (document variables are numbers).
pub(crate) fn number(text: &str) -> Result<f64, String> {
    match onecad_widgets::components::inputs::parse_value(text) {
        onecad_widgets::components::inputs::ParsedValue::Number(n) => Ok(n),
        onecad_widgets::components::inputs::ParsedValue::Expression(_) => {
            Err("Enter a number; expressions are not evaluated here".into())
        }
        onecad_widgets::components::inputs::ParsedValue::Invalid => {
            Err("Enter a finite number".into())
        }
    }
}

pub(crate) fn csv(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}
