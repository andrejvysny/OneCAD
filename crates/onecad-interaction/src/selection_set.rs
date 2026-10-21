//! Typed, ordered model selection and staged operation operands.
//!
//! Pure state only: hosts decide when to apply, invalidate or promote. Topology
//! items carry the scene they were picked in, so a scene change refuses them as
//! stale instead of rebinding them silently.

use crate::ProfileSelection;
use onecad_core::ids::{BodyId, DatumPlaneId, ElementId, RecordId, SketchId, TopoKey};
use onecad_scene::SceneIdentity;
use std::collections::BTreeSet;

pub use crate::sketch::{SelectionMode, MAX_SELECTION_TARGETS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionKind {
    Body,
    Face,
    Edge,
    Vertex,
    Feature,
    Sketch,
    Profile,
    Datum,
}

/// MESH1 carries either a snapshot TopoKey or an already promoted ElementId.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TopologyId {
    Snapshot(TopoKey),
    Element(ElementId),
}

/// A picked face, edge or vertex, scoped to the scene that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyRef {
    pub scene: SceneIdentity,
    pub body: BodyId,
    pub id: TopologyId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectionItem {
    Body(BodyId),
    Face(TopologyRef),
    Edge(TopologyRef),
    Vertex(TopologyRef),
    Feature(RecordId),
    Sketch(SketchId),
    Profile(Box<ProfileSelection>),
    Datum(DatumPlaneId),
}

impl SelectionItem {
    pub fn kind(&self) -> SelectionKind {
        match self {
            Self::Body(_) => SelectionKind::Body,
            Self::Face(_) => SelectionKind::Face,
            Self::Edge(_) => SelectionKind::Edge,
            Self::Vertex(_) => SelectionKind::Vertex,
            Self::Feature(_) => SelectionKind::Feature,
            Self::Sketch(_) => SelectionKind::Sketch,
            Self::Profile(_) => SelectionKind::Profile,
            Self::Datum(_) => SelectionKind::Datum,
        }
    }

    /// Owning body of whole-body and topology items.
    pub fn body(&self) -> Option<BodyId> {
        match self {
            Self::Body(body) => Some(*body),
            Self::Face(t) | Self::Edge(t) | Self::Vertex(t) => Some(t.body),
            _ => None,
        }
    }

    /// Scene that scopes this item; persistent ids are document-scoped.
    pub fn scene(&self) -> Option<SceneIdentity> {
        match self {
            Self::Face(t) | Self::Edge(t) | Self::Vertex(t) => Some(t.scene),
            Self::Profile(p) => Some(p.context.scene),
            _ => None,
        }
    }

    pub fn is_stale_for(&self, scene: SceneIdentity) -> bool {
        self.scene().is_some_and(|own| own != scene)
    }

    /// Identity ignores pick-time evidence such as capture and region anchor.
    pub fn same_target(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Profile(a), Self::Profile(b)) => {
                a.profile.sketch == b.profile.sketch
                    && a.profile.region == b.profile.region
                    && a.profile.region_identity_version == b.profile.region_identity_version
                    && a.context.scene == b.context.scene
            }
            _ => self == other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionRefusal {
    TooMany { limit: usize },
}

/// Ordered, identity-deduplicated selection. `revision` moves only on change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectionSet {
    items: Vec<SelectionItem>,
    revision: u64,
}

impl SelectionSet {
    pub fn items(&self) -> &[SelectionItem] {
        &self.items
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, item: &SelectionItem) -> bool {
        self.items.iter().any(|current| current.same_target(item))
    }

    /// Applies one gesture atomically; an over-limit result leaves the set unchanged.
    pub fn apply(
        &mut self,
        mode: SelectionMode,
        items: impl IntoIterator<Item = SelectionItem>,
    ) -> Result<bool, SelectionRefusal> {
        let mut next = if mode == SelectionMode::Replace {
            Vec::new()
        } else {
            self.items.clone()
        };
        let mut seen: Vec<SelectionItem> = Vec::new();
        for item in items {
            if seen.iter().any(|s| s.same_target(&item)) {
                continue;
            }
            seen.push(item.clone());
            match next.iter().position(|current| current.same_target(&item)) {
                Some(index) if matches!(mode, SelectionMode::Toggle | SelectionMode::Subtract) => {
                    next.remove(index);
                }
                Some(_) => {}
                None if mode != SelectionMode::Subtract => next.push(item),
                None => {}
            }
        }
        if next.len() > MAX_SELECTION_TARGETS {
            return Err(SelectionRefusal::TooMany {
                limit: MAX_SELECTION_TARGETS,
            });
        }
        Ok(self.replace_items(next))
    }

    pub fn clear(&mut self) -> bool {
        self.replace_items(Vec::new())
    }

    /// Keeps items matching `keep`; returns the removed items in selection order.
    pub fn retain(&mut self, mut keep: impl FnMut(&SelectionItem) -> bool) -> Vec<SelectionItem> {
        let (kept, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|item| keep(item));
        self.items = kept;
        if !removed.is_empty() {
            self.revision += 1;
        }
        removed
    }

    /// Removes items matching `stale`; returns them in selection order.
    pub fn invalidate(
        &mut self,
        mut stale: impl FnMut(&SelectionItem) -> bool,
    ) -> Vec<SelectionItem> {
        self.retain(|item| !stale(item))
    }

    pub fn kinds(&self) -> BTreeSet<SelectionKind> {
        self.items.iter().map(SelectionItem::kind).collect()
    }

    /// The only kind present, when the set is non-empty and homogeneous.
    pub fn single_kind(&self) -> Option<SelectionKind> {
        let kinds = self.kinds();
        (kinds.len() == 1).then(|| *kinds.first().expect("one kind"))
    }

    /// The one body shared by every item, when every item has a body.
    pub fn single_body(&self) -> Option<BodyId> {
        let first = self.items.first()?.body()?;
        self.items
            .iter()
            .all(|item| item.body() == Some(first))
            .then_some(first)
    }

    pub fn of_kind(&self, kind: SelectionKind) -> impl Iterator<Item = &SelectionItem> {
        self.items.iter().filter(move |item| item.kind() == kind)
    }

    pub fn bodies(&self) -> impl Iterator<Item = BodyId> + '_ {
        self.items.iter().filter_map(|item| match item {
            SelectionItem::Body(body) => Some(*body),
            _ => None,
        })
    }

    pub fn faces(&self) -> impl Iterator<Item = &TopologyRef> {
        self.items.iter().filter_map(|item| match item {
            SelectionItem::Face(t) => Some(t),
            _ => None,
        })
    }

    pub fn edges(&self) -> impl Iterator<Item = &TopologyRef> {
        self.items.iter().filter_map(|item| match item {
            SelectionItem::Edge(t) => Some(t),
            _ => None,
        })
    }

    pub fn vertices(&self) -> impl Iterator<Item = &TopologyRef> {
        self.items.iter().filter_map(|item| match item {
            SelectionItem::Vertex(t) => Some(t),
            _ => None,
        })
    }

    pub fn profiles(&self) -> impl Iterator<Item = &ProfileSelection> {
        self.items.iter().filter_map(|item| match item {
            SelectionItem::Profile(p) => Some(p.as_ref()),
            _ => None,
        })
    }

    fn replace_items(&mut self, next: Vec<SelectionItem>) -> bool {
        if next == self.items {
            return false;
        }
        self.items = next;
        self.revision += 1;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperandRole {
    Target,
    Tool,
    Edges,
    Faces,
    Profile,
    Axis,
    Plane,
    Placement,
}

impl OperandRole {
    /// Consumed operands are exclusive; reference roles may reuse picked geometry.
    pub fn is_exclusive(self) -> bool {
        !matches!(self, Self::Axis | Self::Plane | Self::Placement)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OperandStage {
    pub role: OperandRole,
    pub accepts: BTreeSet<SelectionKind>,
    pub min: usize,
    pub max: Option<usize>,
    pub items: Vec<SelectionItem>,
}

impl OperandStage {
    pub fn new(
        role: OperandRole,
        accepts: impl IntoIterator<Item = SelectionKind>,
        min: usize,
        max: Option<usize>,
    ) -> Self {
        Self {
            role,
            accepts: accepts.into_iter().collect(),
            min,
            max,
            items: Vec::new(),
        }
    }

    pub fn is_satisfied(&self) -> bool {
        self.items.len() >= self.min && self.max.is_none_or(|max| self.items.len() <= max)
    }

    pub fn is_full(&self) -> bool {
        self.max.is_some_and(|max| self.items.len() >= max)
    }

    fn holds(&self, item: &SelectionItem) -> bool {
        self.items.iter().any(|current| current.same_target(item))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandPlanError {
    Empty,
    NoAcceptedKinds(OperandRole),
    ZeroMax(OperandRole),
    MinAboveMax(OperandRole),
    NotEmpty(OperandRole),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperandRefusal {
    WrongKind {
        role: OperandRole,
        kind: SelectionKind,
    },
    /// `role` names the stage that already holds the item.
    Duplicate {
        role: OperandRole,
    },
    TooMany {
        role: OperandRole,
        max: usize,
    },
    Stale {
        scene: SceneIdentity,
    },
    Unsatisfied {
        role: OperandRole,
        have: usize,
        min: usize,
    },
    NoNextStage,
    NoSuchStage(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub stage: usize,
    pub role: OperandRole,
    /// True when the stage filled and the next stage became active.
    pub advanced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Toggled {
    Added(Accepted),
    Removed(SelectionItem),
}

/// Typed record of an operand dropped by invalidation.
#[derive(Clone, Debug, PartialEq)]
pub struct InvalidatedOperand {
    pub stage: usize,
    pub role: OperandRole,
    pub item: SelectionItem,
}

/// Ordered operand stages for one operation, scoped to one accepted scene.
#[derive(Clone, Debug, PartialEq)]
pub struct StagedOperands {
    stages: Vec<OperandStage>,
    active: usize,
    scene: SceneIdentity,
}

impl StagedOperands {
    pub fn new(scene: SceneIdentity, stages: Vec<OperandStage>) -> Result<Self, OperandPlanError> {
        if stages.is_empty() {
            return Err(OperandPlanError::Empty);
        }
        for stage in &stages {
            if stage.accepts.is_empty() {
                return Err(OperandPlanError::NoAcceptedKinds(stage.role));
            }
            if stage.max == Some(0) {
                return Err(OperandPlanError::ZeroMax(stage.role));
            }
            if stage.max.is_some_and(|max| stage.min > max) {
                return Err(OperandPlanError::MinAboveMax(stage.role));
            }
            if !stage.items.is_empty() {
                return Err(OperandPlanError::NotEmpty(stage.role));
            }
        }
        Ok(Self {
            stages,
            active: 0,
            scene,
        })
    }

    pub fn stages(&self) -> &[OperandStage] {
        &self.stages
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &OperandStage {
        &self.stages[self.active]
    }

    pub fn scene(&self) -> SceneIdentity {
        self.scene
    }

    pub fn is_complete(&self) -> bool {
        self.stages.iter().all(OperandStage::is_satisfied)
    }

    /// Adds `item` to the active stage, advancing when that stage fills.
    pub fn offer(&mut self, item: SelectionItem) -> Result<Accepted, OperandRefusal> {
        if item.is_stale_for(self.scene) {
            return Err(OperandRefusal::Stale { scene: self.scene });
        }
        let stage = &self.stages[self.active];
        if !stage.accepts.contains(&item.kind()) {
            return Err(OperandRefusal::WrongKind {
                role: stage.role,
                kind: item.kind(),
            });
        }
        if let Some(role) = self.conflicting_role(&item) {
            return Err(OperandRefusal::Duplicate { role });
        }
        let stage = &mut self.stages[self.active];
        if let Some(max) = stage.max.filter(|_| stage.is_full()) {
            return Err(OperandRefusal::TooMany {
                role: stage.role,
                max,
            });
        }
        stage.items.push(item);
        let accepted = Accepted {
            stage: self.active,
            role: stage.role,
            advanced: stage.is_full() && self.active + 1 < self.stages.len(),
        };
        if accepted.advanced {
            self.active += 1;
        }
        Ok(accepted)
    }

    /// Removes `item` from the active stage only.
    pub fn remove(&mut self, item: &SelectionItem) -> Option<SelectionItem> {
        let stage = &mut self.stages[self.active];
        let index = stage.items.iter().position(|c| c.same_target(item))?;
        Some(stage.items.remove(index))
    }

    pub fn toggle(&mut self, item: SelectionItem) -> Result<Toggled, OperandRefusal> {
        match self.remove(&item) {
            Some(removed) => Ok(Toggled::Removed(removed)),
            None => self.offer(item).map(Toggled::Added),
        }
    }

    /// Moves past a satisfied active stage.
    pub fn advance(&mut self) -> Result<usize, OperandRefusal> {
        let stage = &self.stages[self.active];
        if stage.items.len() < stage.min {
            return Err(OperandRefusal::Unsatisfied {
                role: stage.role,
                have: stage.items.len(),
                min: stage.min,
            });
        }
        if self.active + 1 >= self.stages.len() {
            return Err(OperandRefusal::NoNextStage);
        }
        self.active += 1;
        Ok(self.active)
    }

    /// Activates a stage whose predecessors are all satisfied.
    pub fn activate(&mut self, index: usize) -> Result<(), OperandRefusal> {
        if index >= self.stages.len() {
            return Err(OperandRefusal::NoSuchStage(index));
        }
        if let Some(stage) = self.stages[..index].iter().find(|s| !s.is_satisfied()) {
            return Err(OperandRefusal::Unsatisfied {
                role: stage.role,
                have: stage.items.len(),
                min: stage.min,
            });
        }
        self.active = index;
        Ok(())
    }

    /// Drops matching items; the first stage left unsatisfied becomes active
    /// when it precedes the current one.
    pub fn invalidate(
        &mut self,
        mut stale: impl FnMut(&SelectionItem) -> bool,
    ) -> Vec<InvalidatedOperand> {
        let mut removed = Vec::new();
        for (index, stage) in self.stages.iter_mut().enumerate() {
            let (kept, dropped): (Vec<_>, Vec<_>) = std::mem::take(&mut stage.items)
                .into_iter()
                .partition(|item| !stale(item));
            stage.items = kept;
            removed.extend(dropped.into_iter().map(|item| InvalidatedOperand {
                stage: index,
                role: stage.role,
                item,
            }));
        }
        if let Some(first) = self.stages.iter().position(|s| !s.is_satisfied()) {
            self.active = self.active.min(first);
        }
        removed
    }

    /// Rescopes to `scene`, dropping items picked in any other scene.
    pub fn set_scene(&mut self, scene: SceneIdentity) -> Vec<InvalidatedOperand> {
        self.scene = scene;
        self.invalidate(|item| item.is_stale_for(scene))
    }

    fn conflicting_role(&self, item: &SelectionItem) -> Option<OperandRole> {
        let own = self.stages[self.active].role;
        self.stages.iter().enumerate().find_map(|(index, stage)| {
            let conflicts = if index == self.active {
                stage.holds(item)
            } else {
                (own.is_exclusive() || stage.role.is_exclusive()) && stage.holds(item)
            };
            conflicts.then_some(stage.role)
        })
    }
}
