//! Occurrence tree: placed instances of component definitions.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AssemblyError;
use crate::math::RigidTransform;

/// Provisional ADR26 cap on occurrence nesting depth (roots have depth 1),
/// pending measurement.
pub const MAX_DEPTH: usize = 8;
/// Provisional ADR26 cap on total occurrences per assembly, pending
/// measurement.
pub const MAX_OCCURRENCES: usize = 2000;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        // No `Default`: a random default id is a silent non-determinism footgun.
        #[allow(clippy::new_without_default)]
        impl $name {
            /// Mints a fresh id (UUID v4).
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

uuid_id!(
    /// Identity of one placed instance.
    OccurrenceId
);
uuid_id!(
    /// Identity of a component definition that occurrences instantiate.
    ComponentDefId
);

/// One placed instance of a component definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub id: OccurrenceId,
    pub def: ComponentDefId,
    pub parent: Option<OccurrenceId>,
    /// Pose relative to the parent (or the world for a root).
    pub local: RigidTransform,
    pub grounded: bool,
    pub name: String,
}

/// Ancestor chain, root first, ending at the addressed occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OccurrencePath(pub Vec<OccurrenceId>);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TreeRepr {
    occurrences: Vec<Occurrence>,
}

/// Occurrences keyed by id; serializes as `{ occurrences: [..] }` sorted by id.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(try_from = "TreeRepr", into = "TreeRepr")]
pub struct OccurrenceTree {
    occurrences: BTreeMap<OccurrenceId, Occurrence>,
}

impl From<OccurrenceTree> for TreeRepr {
    fn from(t: OccurrenceTree) -> Self {
        Self {
            occurrences: t.occurrences.into_values().collect(),
        }
    }
}

impl TryFrom<TreeRepr> for OccurrenceTree {
    type Error = AssemblyError;

    /// Rebuilds the tree and re-validates every invariant `insert` enforces.
    fn try_from(repr: TreeRepr) -> Result<Self, AssemblyError> {
        if repr.occurrences.len() > MAX_OCCURRENCES {
            return Err(AssemblyError::InstanceLimit {
                limit: MAX_OCCURRENCES,
            });
        }
        let mut tree = Self::new();
        for occ in repr.occurrences {
            if !occ.local.is_finite() {
                return Err(AssemblyError::NonFiniteTransform(occ.id));
            }
            let id = occ.id;
            if tree.occurrences.insert(id, occ).is_some() {
                return Err(AssemblyError::DuplicateOccurrence(id));
            }
        }
        for id in tree.occurrences.keys().copied().collect::<Vec<_>>() {
            if tree.path(id)?.0.len() > MAX_DEPTH {
                return Err(AssemblyError::DepthExceeded { limit: MAX_DEPTH });
            }
        }
        Ok(tree)
    }
}

impl OccurrenceTree {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.occurrences.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.occurrences.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: OccurrenceId) -> Option<&Occurrence> {
        self.occurrences.get(&id)
    }

    /// Occurrences in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Occurrence> {
        self.occurrences.values()
    }

    /// Inserts an occurrence. Refuses duplicates, unknown parents, non-finite
    /// transforms, nesting beyond `MAX_DEPTH` and counts beyond `MAX_OCCURRENCES`.
    pub fn insert(&mut self, occ: Occurrence) -> Result<(), AssemblyError> {
        if self.occurrences.contains_key(&occ.id) {
            return Err(AssemblyError::DuplicateOccurrence(occ.id));
        }
        if !occ.local.is_finite() {
            return Err(AssemblyError::NonFiniteTransform(occ.id));
        }
        if self.occurrences.len() >= MAX_OCCURRENCES {
            return Err(AssemblyError::InstanceLimit {
                limit: MAX_OCCURRENCES,
            });
        }
        if let Some(parent) = occ.parent {
            if self.path(parent)?.0.len() + 1 > MAX_DEPTH {
                return Err(AssemblyError::DepthExceeded { limit: MAX_DEPTH });
            }
        }
        self.occurrences.insert(occ.id, occ);
        Ok(())
    }

    /// Removes a leaf occurrence.
    pub fn remove(&mut self, id: OccurrenceId) -> Result<Occurrence, AssemblyError> {
        if !self.occurrences.contains_key(&id) {
            return Err(AssemblyError::UnknownOccurrence(id));
        }
        if self.occurrences.values().any(|o| o.parent == Some(id)) {
            return Err(AssemblyError::HasChildren(id));
        }
        self.occurrences
            .remove(&id)
            .ok_or(AssemblyError::UnknownOccurrence(id))
    }

    pub fn set_local(
        &mut self,
        id: OccurrenceId,
        local: RigidTransform,
    ) -> Result<(), AssemblyError> {
        if !local.is_finite() {
            return Err(AssemblyError::NonFiniteTransform(id));
        }
        let occ = self
            .occurrences
            .get_mut(&id)
            .ok_or(AssemblyError::UnknownOccurrence(id))?;
        occ.local = local;
        Ok(())
    }

    pub fn set_grounded(&mut self, id: OccurrenceId, grounded: bool) -> Result<(), AssemblyError> {
        let occ = self
            .occurrences
            .get_mut(&id)
            .ok_or(AssemblyError::UnknownOccurrence(id))?;
        occ.grounded = grounded;
        Ok(())
    }

    /// Direct children, sorted by id.
    #[must_use]
    pub fn children(&self, id: OccurrenceId) -> Vec<OccurrenceId> {
        self.occurrences
            .values()
            .filter(|o| o.parent == Some(id))
            .map(|o| o.id)
            .collect()
    }

    /// Occurrences without a parent, sorted by id.
    #[must_use]
    pub fn roots(&self) -> Vec<OccurrenceId> {
        self.occurrences
            .values()
            .filter(|o| o.parent.is_none())
            .map(|o| o.id)
            .collect()
    }

    /// Root-first ancestor chain ending at `id`.
    pub fn path(&self, id: OccurrenceId) -> Result<OccurrencePath, AssemblyError> {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            if chain.contains(&c) {
                return Err(AssemblyError::Cycle(c));
            }
            let occ = self
                .occurrences
                .get(&c)
                .ok_or(AssemblyError::UnknownOccurrence(c))?;
            chain.push(c);
            cur = occ.parent;
        }
        chain.reverse();
        Ok(OccurrencePath(chain))
    }

    /// World pose: local transforms composed root to leaf.
    pub fn world_transform(&self, id: OccurrenceId) -> Result<RigidTransform, AssemblyError> {
        let mut world = RigidTransform::IDENTITY;
        for step in self.path(id)?.0 {
            let occ = self
                .occurrences
                .get(&step)
                .ok_or(AssemblyError::UnknownOccurrence(step))?;
            world = world.compose(&occ.local);
        }
        Ok(world)
    }

    /// All occurrences instantiating `def`, sorted by id.
    #[must_use]
    pub fn instances_of(&self, def: ComponentDefId) -> Vec<OccurrenceId> {
        self.occurrences
            .values()
            .filter(|o| o.def == def)
            .map(|o| o.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Quat, Vec3d};
    use std::f64::consts::FRAC_PI_2;

    fn occ(def: ComponentDefId, parent: Option<OccurrenceId>) -> Occurrence {
        Occurrence {
            id: OccurrenceId::new(),
            def,
            parent,
            local: RigidTransform::IDENTITY,
            grounded: false,
            name: "part".to_owned(),
        }
    }

    fn translate(x: f64, y: f64, z: f64) -> RigidTransform {
        RigidTransform::new(Quat::IDENTITY, Vec3d::new(x, y, z))
    }

    #[test]
    fn insert_refuses_duplicate_and_unknown_parent() {
        let def = ComponentDefId::new();
        let mut tree = OccurrenceTree::new();
        let a = occ(def, None);
        tree.insert(a.clone()).unwrap();
        assert_eq!(
            tree.insert(a.clone()),
            Err(AssemblyError::DuplicateOccurrence(a.id))
        );
        let ghost = OccurrenceId::new();
        assert_eq!(
            tree.insert(occ(def, Some(ghost))),
            Err(AssemblyError::UnknownOccurrence(ghost))
        );
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn insert_refuses_non_finite_transform() {
        let mut tree = OccurrenceTree::new();
        let mut o = occ(ComponentDefId::new(), None);
        o.local = translate(f64::NAN, 0.0, 0.0);
        let id = o.id;
        assert_eq!(tree.insert(o), Err(AssemblyError::NonFiniteTransform(id)));
        let ok = occ(ComponentDefId::new(), None);
        let ok_id = ok.id;
        tree.insert(ok).unwrap();
        assert_eq!(
            tree.set_local(ok_id, translate(f64::INFINITY, 0.0, 0.0)),
            Err(AssemblyError::NonFiniteTransform(ok_id))
        );
    }

    #[test]
    fn depth_limit_enforced() {
        let def = ComponentDefId::new();
        let mut tree = OccurrenceTree::new();
        let mut parent = None;
        let mut last = None;
        for _ in 0..MAX_DEPTH {
            let o = occ(def, parent);
            parent = Some(o.id);
            last = Some(o.id);
            tree.insert(o).unwrap();
        }
        assert_eq!(tree.path(last.unwrap()).unwrap().0.len(), MAX_DEPTH);
        assert_eq!(
            tree.insert(occ(def, parent)),
            Err(AssemblyError::DepthExceeded { limit: MAX_DEPTH })
        );
    }

    #[test]
    fn instance_limit_enforced() {
        let def = ComponentDefId::new();
        let mut tree = OccurrenceTree::new();
        for _ in 0..MAX_OCCURRENCES {
            tree.insert(occ(def, None)).unwrap();
        }
        assert_eq!(
            tree.insert(occ(def, None)),
            Err(AssemblyError::InstanceLimit {
                limit: MAX_OCCURRENCES
            })
        );
        assert_eq!(tree.instances_of(def).len(), MAX_OCCURRENCES);
    }

    #[test]
    fn cycle_detected_in_path() {
        let def = ComponentDefId::new();
        let mut a = occ(def, None);
        let mut b = occ(def, None);
        a.parent = Some(b.id);
        b.parent = Some(a.id);
        let a_id = a.id;
        let mut tree = OccurrenceTree::new();
        tree.occurrences.insert(a.id, a);
        tree.occurrences.insert(b.id, b);
        assert!(matches!(tree.path(a_id), Err(AssemblyError::Cycle(_))));
        assert!(matches!(
            tree.world_transform(a_id),
            Err(AssemblyError::Cycle(_))
        ));
    }

    #[test]
    fn remove_refuses_with_children() {
        let def = ComponentDefId::new();
        let mut tree = OccurrenceTree::new();
        let root = occ(def, None);
        let child = occ(def, Some(root.id));
        let (rid, cid) = (root.id, child.id);
        tree.insert(root).unwrap();
        tree.insert(child).unwrap();
        assert_eq!(tree.remove(rid), Err(AssemblyError::HasChildren(rid)));
        assert_eq!(tree.children(rid), vec![cid]);
        assert_eq!(tree.roots(), vec![rid]);
        assert_eq!(tree.remove(cid).unwrap().id, cid);
        assert_eq!(tree.remove(rid).unwrap().id, rid);
        assert!(tree.is_empty());
        assert_eq!(tree.remove(rid), Err(AssemblyError::UnknownOccurrence(rid)));
    }

    #[test]
    fn nested_world_transform_composes_root_to_leaf() {
        let def = ComponentDefId::new();
        let mut tree = OccurrenceTree::new();
        let mut root = occ(def, None);
        root.local = RigidTransform::new(
            Quat::from_axis_angle(Vec3d::Z, FRAC_PI_2).unwrap(),
            Vec3d::new(10.0, 0.0, 0.0),
        );
        let mut mid = occ(def, Some(root.id));
        mid.local = translate(1.0, 0.0, 0.0);
        let mut leaf = occ(def, Some(mid.id));
        leaf.local = translate(0.0, 2.0, 0.0);
        let (rid, lid) = (root.id, leaf.id);
        tree.insert(root).unwrap();
        tree.insert(mid).unwrap();
        tree.insert(leaf).unwrap();

        let path = tree.path(lid).unwrap();
        assert_eq!(path.0.len(), 3);
        assert_eq!(path.0[0], rid);
        assert_eq!(path.0[2], lid);

        // Leaf origin: mid (1,0,0) + (0,2,0) = (1,2,0) in root frame,
        // rotated 90 deg about Z -> (-2,1,0), shifted by (10,0,0).
        let w = tree.world_transform(lid).unwrap();
        let p = w.apply_point(Vec3d::ZERO);
        assert!((p - Vec3d::new(8.0, 1.0, 0.0)).norm() < 1e-12);
        tree.set_grounded(rid, true).unwrap();
        assert!(tree.get(rid).unwrap().grounded);
    }

    #[test]
    fn serde_round_trip_is_sorted_and_validated() {
        let def = ComponentDefId::new();
        let mut tree2 = OccurrenceTree::new();
        let root2 = occ(def, None);
        let child2 = occ(def, Some(root2.id));
        tree2.insert(root2).unwrap();
        tree2.insert(child2).unwrap();
        let json = serde_json::to_string(&tree2).unwrap();
        assert!(json.starts_with("{\"occurrences\":["));
        let back: OccurrenceTree = serde_json::from_str(&json).unwrap();
        assert_eq!(tree2, back);
        let ids: Vec<_> = back.iter().map(|o| o.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn deserialize_rejects_unknown_parent() {
        let def = ComponentDefId::new();
        let orphan = occ(def, Some(OccurrenceId::new()));
        let json = serde_json::to_string(&TreeRepr {
            occurrences: vec![orphan],
        })
        .unwrap();
        assert!(serde_json::from_str::<OccurrenceTree>(&json).is_err());
    }
}
