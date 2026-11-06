//! Pure view-model for placed component instances: badges, status wording, parameter rows
//! and the draft an instance edits. Nothing here touches egui or the backend.
use super::InstanceView;
use crate::library_catalog::draft::{ParamDraft, format_number};
use onecad_app::dto::{ComponentInstanceDto, ComponentStatusDto};
use onecad_core::document::record::ComponentParamValue as Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Neutral,
    Accent,
}

/// Explorer text for a row: the designation when the package declares one.
pub(crate) fn title(instance: &ComponentInstanceDto) -> String {
    instance
        .designation
        .clone()
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| instance.name.clone())
}

pub(crate) fn link_badge(instance: &ComponentInstanceDto) -> (&'static str, Tone) {
    if instance.linked {
        ("Linked", Tone::Accent)
    } else {
        ("Copy", Tone::Neutral)
    }
}

pub(crate) fn status_badge(status: &ComponentStatusDto) -> Option<&'static str> {
    match status {
        ComponentStatusDto::UpgradeAvailable { .. } => Some("Upgrade available"),
        ComponentStatusDto::ArchivedSnapshot { .. } => Some("Saved definition"),
        ComponentStatusDto::DefinitionMissing => Some("Missing definition"),
        ComponentStatusDto::DefinitionUnreadable { .. } => Some("Unreadable"),
        ComponentStatusDto::RevisionMismatch { .. } => Some("Revision changed"),
        ComponentStatusDto::Linked | ComponentStatusDto::Detached => None,
    }
}

/// What the library state means for this instance. Repairs are always the user's: the
/// document never moves or drops geometry because a definition changed.
pub(crate) fn status_text(instance: &ComponentInstanceDto) -> String {
    match &instance.status {
        ComponentStatusDto::Linked => {
            "Linked to the library. Parameters can be edited here.".to_owned()
        }
        ComponentStatusDto::ArchivedSnapshot { catalog } => {
            let detail = match catalog.as_ref() {
                ComponentStatusDto::DefinitionMissing => "The external library is unavailable.",
                ComponentStatusDto::DefinitionUnreadable { .. } => {
                    "The external definition is unreadable."
                }
                ComponentStatusDto::RevisionMismatch { .. } => {
                    "The external package revision changed."
                }
                _ => "The external catalog is unavailable.",
            };
            format!("{detail} The document's saved definition supports parameter edits.")
        }
        ComponentStatusDto::UpgradeAvailable { version } => format!(
            "Version {version} is available. This instance stays on {} until you update it.",
            instance.component_version
        ),
        ComponentStatusDto::DefinitionMissing => {
            "The library no longer has this definition; geometry stays as placed. \
             Parameters cannot be edited. Detach keeps the geometry as a copy."
                .to_owned()
        }
        ComponentStatusDto::DefinitionUnreadable { reason } => format!(
            "The library definition cannot be read ({reason}); geometry stays as placed. \
             Parameters cannot be edited until it can."
        ),
        ComponentStatusDto::RevisionMismatch { .. } => {
            "The package contents changed without a new version; geometry stays as placed \
             until the document regenerates from the changed definition."
                .to_owned()
        }
        ComponentStatusDto::Detached => {
            "Copy: no longer linked to the library. Parameters are read-only and updates \
             do not apply."
                .to_owned()
        }
    }
}

/// Parameters may be edited while the library still serves the definition.
pub(crate) fn editable(instance: &ComponentInstanceDto) -> bool {
    instance.linked
        && matches!(
            instance.status,
            ComponentStatusDto::Linked
                | ComponentStatusDto::ArchivedSnapshot { .. }
                | ComponentStatusDto::UpgradeAvailable { .. }
                | ComponentStatusDto::RevisionMismatch { .. }
        )
}

pub(crate) fn upgradable(items: &[InstanceView]) -> Vec<String> {
    items
        .iter()
        .filter(|v| {
            v.dto.linked && matches!(v.dto.status, ComponentStatusDto::UpgradeAvailable { .. })
        })
        .map(|v| v.dto.record_id.clone())
        .collect()
}

/// One effective parameter as the read-only view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParamRow {
    pub key: String,
    pub effective: String,
    pub default: Option<String>,
    pub overridden: bool,
}

pub(crate) fn text(value: &Value) -> String {
    match value {
        Value::Number(n) => format_number(*n),
        Value::Text(t) => t.clone(),
        Value::Bool(b) => b.to_string(),
    }
}

pub(crate) fn param_rows(instance: &ComponentInstanceDto) -> Vec<ParamRow> {
    instance
        .effective
        .iter()
        .map(|(key, value)| ParamRow {
            key: key.clone(),
            effective: text(value),
            default: instance.defaults.get(key).map(text),
            overridden: instance.overrides.contains(key),
        })
        .collect()
}

/// The draft an instance's inspector edits, seeded with its effective values.
pub(crate) fn draft_for(view: &InstanceView) -> Option<ParamDraft> {
    if !editable(&view.dto) {
        return None;
    }
    let mut draft = ParamDraft::from_component(view.component.as_ref()?);
    for (key, value) in &view.dto.effective {
        draft.set_text(key, text(value));
    }
    Some(draft)
}

/// True when any free field's text differs from `baseline` (the values it was seeded with).
pub(crate) fn dirty(draft: &ParamDraft, baseline: &BTreeMap<String, Value>) -> bool {
    draft
        .fields()
        .iter()
        .filter(|f| f.is_free())
        .any(|f| baseline.get(&f.key).map(text).as_deref() != Some(draft.text(&f.key)))
}

/// One history step a typed draft asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Commit {
    /// Values that differ from the package default; defaults are never pinned.
    Set(BTreeMap<String, Value>),
    /// Keys the user returned to their default: the override is removed instead.
    Reset(Vec<String>),
}

/// The next commit the draft implies, only when it resolves and differs in value from what
/// the instance holds (re-typing "20.0" over 20 changes nothing). A draft that both changes
/// and restores keys commits the changes first; the restores follow as the next commit.
pub(crate) fn pending_commit(
    draft: &ParamDraft,
    instance: &ComponentInstanceDto,
) -> Option<Commit> {
    let params = draft.resolve().ok()?;
    let mut set = BTreeMap::new();
    let mut reset = Vec::new();
    for (key, value) in params {
        if instance.effective.get(&key) == Some(&value) {
            continue;
        }
        if instance.defaults.get(&key) == Some(&value) {
            reset.push(key);
        } else {
            set.insert(key, value);
        }
    }
    if !set.is_empty() {
        Some(Commit::Set(set))
    } else if !reset.is_empty() {
        Some(Commit::Reset(reset))
    } else {
        None
    }
}

/// The instance holds a parameter that differs from the package default.
pub(crate) fn has_overrides(instance: &ComponentInstanceDto) -> bool {
    !instance.overrides.is_empty()
        || instance
            .effective
            .iter()
            .any(|(key, value)| instance.defaults.get(key).is_some_and(|d| d != value))
}
