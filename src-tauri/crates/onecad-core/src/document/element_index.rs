//! Document-level element partition index (V1/V2 §3.2 partitioned ElementMap).
//!
//! A minimal, Rust-owned map `ElementId -> {body, kind}` recording, for each
//! **minted** persistent element, its **current** body partition and kind. It is
//! deliberately small: the ID-on-demand policy (V1/V2 §3.1) means only referenced
//! elements are ever minted, so this index holds only the elements a feature
//! input / constraint / named selection actually points at.
//!
//! ## Why partition membership lives here, not in the id
//!
//! An [`ElementId`] is globally unique and **does NOT embed `BodyId`** (SCHEMA §2;
//! `ids.rs`): which body an element belongs to is a **mapping**, not identity, so
//! an element survives body split/merge (its id is unchanged; only its
//! [`ElementEntry::body`] here moves). This index IS that mapping. Folding a
//! regen [`ElementMapDelta`](crate::regen::engine::ElementMapDelta) updates the
//! partition without ever changing an id (Invariant 1).
//!
//! It is a document-level addition (reported in `document/mod.rs`), serialized
//! only when non-empty so existing `document.json` fixtures are unaffected.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::document::refs::{AnchorIntent, ElementKind};
use crate::ids::{BodyId, ElementId};

/// The current partition membership of a minted element, plus the durable
/// evidence needed to find that element again in a FRESH session (DI-4).
///
/// ## Why the evidence is here at all
///
/// The worker's element-map partition is minted on demand and dies with the
/// process, so on reopen nothing in a new session knows which face any persisted
/// [`ElementId`] names — an authored face colour survives in the file and stops
/// being paintable. Storing `{body, kind}` alone cannot fix that: a binding needs
/// something the ladder can RESOLVE.
///
/// A [`TopoKey`](crate::ids::TopoKey) is deliberately **not** what gets stored.
/// It is snapshot-scoped evidence (SCHEMA §9), and persisting it as authority is
/// precisely the silent wrong-bind this migration exists to prevent. The anchor
/// and the worker's opaque descriptor are the durable halves, and they are what
/// [`ElementRef`](crate::document::refs::ElementRef) already carries for op
/// inputs — so a reopened document re-binds through the same ladder, under the
/// same confidence gate, as every other reference in the system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementEntry {
    /// The body this element currently belongs to (moves on split/merge).
    pub body: BodyId,
    /// The topological kind (face/edge/vertex).
    pub kind: ElementKind,
    /// Geometric selection intent captured when the id was minted. Optional:
    /// entries written before this field existed simply cannot be re-bound, and
    /// are left alone rather than guessed at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<AnchorIntent>,
    /// The worker's opaque descriptor for the element (SCHEMA §10), stored
    /// verbatim and never interpreted here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<serde_json::Value>,
}

impl ElementEntry {
    /// A partition entry with no re-bind evidence.
    #[must_use]
    pub fn new(body: BodyId, kind: ElementKind) -> Self {
        Self {
            body,
            kind,
            anchor: None,
            descriptor: None,
        }
    }

    /// The same entry carrying the evidence a fresh session needs to find this
    /// element again (DI-4).
    #[must_use]
    pub fn with_evidence(
        mut self,
        anchor: Option<AnchorIntent>,
        descriptor: Option<serde_json::Value>,
    ) -> Self {
        self.anchor = anchor;
        self.descriptor = descriptor;
        self
    }

    /// Whether this entry can be re-bound in a fresh session at all.
    #[must_use]
    pub fn has_rebind_evidence(&self) -> bool {
        self.anchor.is_some() || self.descriptor.is_some()
    }
}

/// The document's `ElementId -> {body, kind}` partition index. Serializes
/// transparently as a JSON object keyed by element id (deterministic `BTreeMap`
/// order, Invariant 5).
// `Eq` is deliberately absent: an entry's `descriptor` is an opaque
// `serde_json::Value`, which is only `PartialEq` (floats). Same precedent as
// `LibraryComponentDto`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementIndex {
    map: BTreeMap<ElementId, ElementEntry>,
}

impl ElementIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True iff no elements are indexed (the `skip_serializing_if` predicate).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Number of indexed elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The partition entry for `id`, if indexed.
    #[must_use]
    pub fn get(&self, id: &ElementId) -> Option<&ElementEntry> {
        self.map.get(id)
    }

    /// True iff `id` is indexed.
    #[must_use]
    pub fn contains(&self, id: &ElementId) -> bool {
        self.map.contains_key(id)
    }

    /// The body `id` currently partitions into, if indexed.
    #[must_use]
    pub fn body_of(&self, id: &ElementId) -> Option<BodyId> {
        self.map.get(id).map(|e| e.body)
    }

    /// Records / updates `id`'s partition (a mint or an element-map relabel).
    /// Updating an existing id's body is the split/merge re-partition path — the
    /// id itself never changes (Invariant 1).
    pub fn insert(&mut self, id: ElementId, entry: ElementEntry) {
        self.map.insert(id, entry);
    }

    /// Removes `id` from the index (it left every partition), returning its old
    /// entry if present.
    pub fn remove(&mut self, id: &ElementId) -> Option<ElementEntry> {
        self.map.remove(id)
    }

    /// Iterates the `(id, entry)` pairs in deterministic id order.
    pub fn iter(&self) -> impl Iterator<Item = (&ElementId, &ElementEntry)> {
        self.map.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn body(n: u128) -> BodyId {
        BodyId(Uuid::from_u128(n))
    }

    #[test]
    fn insert_get_remove_and_repartition() {
        let mut idx = ElementIndex::new();
        assert!(idx.is_empty());
        let e = ElementId::new("el_1");
        idx.insert(e.clone(), ElementEntry::new(body(1), ElementKind::Face));
        assert_eq!(idx.body_of(&e), Some(body(1)));
        // Split/merge re-partition: same id, new body — identity unchanged.
        idx.insert(e.clone(), ElementEntry::new(body(2), ElementKind::Face));
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.body_of(&e), Some(body(2)));
        assert!(idx.remove(&e).is_some());
        assert!(idx.is_empty());
    }

    #[test]
    fn serializes_transparently_as_object_when_non_empty() {
        let mut idx = ElementIndex::new();
        idx.insert(
            ElementId::new("el_1"),
            ElementEntry::new(body(1), ElementKind::Edge),
        );
        let v = serde_json::to_value(&idx).unwrap();
        assert!(v.is_object());
        assert_eq!(v["el_1"]["kind"], "edge");
        let back: ElementIndex = serde_json::from_value(v).unwrap();
        assert_eq!(back, idx);
    }
}
