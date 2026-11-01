//! Persisted single-key tool remaps. Overrides are sparse: a tool without an
//! entry keeps its default key; an entry with `key: None` is deliberately unbound.
use crate::tool_registry::{ToolId, default_bindings, effective_key, recordable};
use serde::{Deserialize, Serialize};

const MAX_OVERRIDES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyContext {
    Modeling,
    Sketch,
}

impl KeyContext {
    pub fn sketch(self) -> bool {
        self == Self::Sketch
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShortcutOverride {
    pub context: KeyContext,
    pub tool: ToolId,
    /// `egui::Key::name()`; `None` leaves the tool without a key.
    pub key: Option<String>,
}

fn default_key(context: KeyContext, tool: ToolId) -> Option<egui::Key> {
    default_bindings(context)
        .iter()
        .find(|(_, id)| *id == tool)
        .map(|(key, _)| *key)
}

/// Another tool in the same context that currently holds `key`.
pub fn conflict(
    overrides: &[ShortcutOverride],
    context: KeyContext,
    tool: ToolId,
    key: egui::Key,
) -> Option<ToolId> {
    default_bindings(context)
        .iter()
        .map(|(_, id)| *id)
        .find(|id| *id != tool && effective_key(overrides, *id, context) == Some(key))
}

fn set(
    overrides: &mut Vec<ShortcutOverride>,
    context: KeyContext,
    tool: ToolId,
    key: Option<&str>,
) {
    let key = key.map(str::to_owned);
    match overrides
        .iter_mut()
        .find(|o| o.context == context && o.tool == tool)
    {
        Some(existing) => existing.key = key,
        None => overrides.push(ShortcutOverride { context, tool, key }),
    }
}

fn normalize(overrides: &mut Vec<ShortcutOverride>) {
    overrides.retain(|o| {
        let default = default_key(o.context, o.tool).map(egui::Key::name);
        o.key.is_none() || o.key.as_deref() != default
    });
}

/// Binds `key`; a tool already on it in this context becomes unbound.
pub fn assign(
    overrides: &mut Vec<ShortcutOverride>,
    context: KeyContext,
    tool: ToolId,
    key: egui::Key,
) {
    if !recordable(key) || default_key(context, tool).is_none() {
        return;
    }
    if let Some(other) = conflict(overrides, context, tool, key) {
        set(overrides, context, other, None);
    }
    set(overrides, context, tool, Some(key.name()));
    normalize(overrides);
}

/// Restores the default key; a tool that took it meanwhile becomes unbound.
pub fn reset(overrides: &mut Vec<ShortcutOverride>, context: KeyContext, tool: ToolId) {
    overrides.retain(|o| !(o.context == context && o.tool == tool));
    if let Some(key) = default_key(context, tool)
        && let Some(other) = conflict(overrides, context, tool, key)
    {
        set(overrides, context, other, None);
    }
    normalize(overrides);
}

pub fn reset_all(overrides: &mut Vec<ShortcutOverride>) {
    overrides.clear();
}

pub fn changed_count(overrides: &[ShortcutOverride]) -> usize {
    overrides
        .iter()
        .filter(|o| default_key(o.context, o.tool).map(egui::Key::name) != o.key.as_deref())
        .count()
}

pub fn validate(overrides: &[ShortcutOverride]) -> Result<(), &'static str> {
    if overrides.len() > MAX_OVERRIDES {
        return Err("too many shortcut overrides");
    }
    for (i, o) in overrides.iter().enumerate() {
        if default_key(o.context, o.tool).is_none() {
            return Err("shortcut override names a tool without a default key");
        }
        if overrides[..i]
            .iter()
            .any(|p| p.context == o.context && p.tool == o.tool)
        {
            return Err("duplicate shortcut override");
        }
        if let Some(name) = &o.key
            // Canonical names only: normalize compares strings, so an alias of the default would linger.
            && !egui::Key::from_name(name).is_some_and(|key| recordable(key) && key.name() == name)
        {
            return Err("shortcut key must be a letter or digit");
        }
    }
    for context in [KeyContext::Modeling, KeyContext::Sketch] {
        let mut seen: Vec<egui::Key> = Vec::new();
        for (_, id) in default_bindings(context) {
            if let Some(key) = effective_key(overrides, *id, context) {
                if seen.contains(&key) {
                    return Err("two tools share one shortcut key");
                }
                seen.push(key);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "shortcut_prefs_tests.rs"]
mod tests;
