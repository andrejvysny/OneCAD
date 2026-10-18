//! Namespaced module state carried inside the document (ADR-0004, ADR-0005).
//!
//! A module — built-in or from an addon — owns a slice of the document keyed by
//! its reverse-DNS id. The platform stores that slice and **never interprets
//! it**: the payload is opaque JSON, and its meaning, its schema and its
//! migrations belong to the owning module alone.
//!
//! That opacity is the whole point. A project opened on a machine where an addon
//! is missing must still open, still save, and still contain that addon's data
//! afterwards — so nothing here parses, normalizes or prunes a payload it does
//! not understand.
//!
//! What this is NOT: a place for another module to annotate modeling's bodies by
//! adding fields inside modeling's own state. An addon annotates a body from its
//! OWN namespace, keyed by the entity id.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A module or addon identity — reverse-DNS, e.g. `onecad.modeling`,
/// `com.example.foo`.
///
/// Authored ids go through [`ModuleId::parse`], which enforces the shape.
/// DESERIALIZATION IS DELIBERATELY PERMISSIVE: a document written by a newer
/// build (or by a tool with a laxer notion of the format) must still round-trip,
/// and refusing to load it would destroy exactly the data this module exists to
/// preserve.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModuleId(String);

/// Why a module id was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleIdError {
    /// Fewer than two dot-separated segments.
    NotReverseDns(String),
    /// A segment was empty or contained something other than `[a-z0-9-]`.
    BadSegment(String),
}

impl fmt::Display for ModuleIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReverseDns(v) => {
                write!(
                    f,
                    "\"{v}\" is not a reverse-DNS module id (expected e.g. \"com.example.foo\")"
                )
            }
            Self::BadSegment(v) => write!(f, "\"{v}\" has an empty or invalid segment"),
        }
    }
}

impl std::error::Error for ModuleIdError {}

impl ModuleId {
    /// Validates and wraps an authored id.
    ///
    /// # Errors
    /// [`ModuleIdError`] when the id is not two-or-more lowercase reverse-DNS
    /// segments.
    pub fn parse(value: &str) -> Result<Self, ModuleIdError> {
        let segments: Vec<&str> = value.split('.').collect();
        if segments.len() < 2 {
            return Err(ModuleIdError::NotReverseDns(value.to_string()));
        }
        let ok = |s: &&str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && s.starts_with(|c: char| c.is_ascii_lowercase())
        };
        if !segments.iter().all(ok) {
            return Err(ModuleIdError::BadSegment(value.to_string()));
        }
        Ok(Self(value.to_string()))
    }

    /// The id as it is written on the wire and in storage.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Wraps a value that came FROM storage, without validating it.
    ///
    /// Only for the load path: an id this build considers malformed must still
    /// survive a round trip (ADR-0005).
    #[must_use]
    pub fn from_stored(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One module's slice of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleState {
    /// The OWNING MODULE's schema version — independent of the application
    /// version, the container version and every other module's version.
    pub schema_version: u32,
    /// Opaque to everything except the owning module.
    pub payload: serde_json::Value,
}

impl ModuleState {
    /// A state slice at `schema_version` carrying `payload`.
    #[must_use]
    pub fn new(schema_version: u32, payload: serde_json::Value) -> Self {
        Self {
            schema_version,
            payload,
        }
    }
}

/// Module-owned state, keyed by owner. `BTreeMap` for deterministic
/// serialization — a document's bytes must not depend on hash iteration order.
pub type ModuleStateTable = BTreeMap<ModuleId, ModuleState>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_reverse_dns_ids() {
        assert!(ModuleId::parse("onecad.modeling").is_ok());
        assert!(ModuleId::parse("com.example.foo").is_ok());
        assert!(ModuleId::parse("dev.andrej.my-addon").is_ok());
    }

    #[test]
    fn refuses_authored_ids_that_are_not_reverse_dns() {
        assert!(matches!(
            ModuleId::parse("modeling"),
            Err(ModuleIdError::NotReverseDns(_))
        ));
        for bad in ["com..foo", "com.foo.", ".com.foo", "com.Foo", "com.1foo"] {
            assert!(
                matches!(ModuleId::parse(bad), Err(ModuleIdError::BadSegment(_))),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn stored_ids_are_accepted_verbatim() {
        // A stricter future build must not be able to make an existing document
        // unloadable, so the load path does not re-validate.
        let table: ModuleStateTable = serde_json::from_value(
            json!({ "Not.A.Valid.Id!": { "schemaVersion": 1, "payload": {} } }),
        )
        .expect("an unrecognized id must still deserialize");
        assert!(table.contains_key(&ModuleId::from_stored("Not.A.Valid.Id!".into())));
    }

    #[test]
    fn round_trips_an_opaque_payload_verbatim() {
        let original = json!({
            "com.example.foo": {
                "schemaVersion": 3,
                "payload": { "nested": [1, 2, { "deep": true }], "unicode": "påyload" }
            }
        });
        let table: ModuleStateTable = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(&table).unwrap(), original);
    }

    #[test]
    fn serializes_keys_in_deterministic_order() {
        let mut table = ModuleStateTable::new();
        for id in ["com.zeta.a", "com.alpha.b", "onecad.modeling"] {
            table.insert(ModuleId::parse(id).unwrap(), ModuleState::new(1, json!({})));
        }
        let text = serde_json::to_string(&table).unwrap();
        let alpha = text.find("com.alpha.b").unwrap();
        let zeta = text.find("com.zeta.a").unwrap();
        let onecad = text.find("onecad.modeling").unwrap();
        assert!(
            alpha < zeta && zeta < onecad,
            "keys must be ordered: {text}"
        );
    }
}
