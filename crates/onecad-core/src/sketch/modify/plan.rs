//! Result and refusal types of the Trim/Extend planners.

use crate::edit::SketchEditOp;
use crate::ids::{ConstraintId, EntityId};
use crate::math::Vec2;
use crate::sketch::CurvePosition;

/// One surviving (trim) or reshaped (extend) curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedPiece {
    /// The piece's entity id.
    pub id: EntityId,
    /// The piece reuses the modified entity's id (it contains the original
    /// start point and kept its kind).
    pub keeps_target_id: bool,
    /// The piece still ends at the original START point / role.
    pub owns_start: bool,
    /// The piece still ends at the original END point / role.
    pub owns_end: bool,
}

/// Why the plan adds a constraint that did not exist before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddReason {
    /// A further copy of `of` onto another surviving piece (the first copy
    /// keeps `of`'s id and is listed in `kept_constraints`).
    CopiedFrom {
        /// The original constraint.
        of: ConstraintId,
    },
    /// Holds a trim cut endpoint on the boundary curve that produced the cut.
    CutWeld {
        /// The boundary curve.
        boundary: EntityId,
    },
    /// Holds an extended endpoint on the boundary curve it was grown to.
    ExtendWeld {
        /// The boundary curve.
        boundary: EntityId,
    },
}

/// A constraint the plan adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddedConstraint {
    /// The new constraint id.
    pub id: ConstraintId,
    /// Why it exists.
    pub reason: AddReason,
}

/// Why the plan removes a constraint without re-adding it under its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// The whole entity was trimmed away.
    EntityDeleted,
    /// It references an endpoint (or defining point) that no longer exists.
    PointRemoved,
    /// A length relation (`Equal` on a line, a line/arc end-to-end `Distance`)
    /// whose measured length the trim changed.
    LengthChanged,
    /// A `Midpoint` of a line whose midpoint the trim moved.
    MidpointMoved,
    /// The entity split into several pieces and the relation cannot be
    /// assigned to one of them without guessing.
    SplitAmbiguous,
    /// Its on-curve point or tangent contact lies in the removed span.
    OnRemovedSpan,
    /// An `OnCurve` on the target of a point the cut now welds with a
    /// `Coincident` (however the point is named); keeping both would state the
    /// same fact twice and over-constrain the point.
    SupersededByCutWeld,
    /// An `OnCurve` on the target of the boundary end point the extended end
    /// now welds to with a `Coincident`; same redundancy as above.
    SupersededByExtendWeld,
    /// No remap rule covers this constraint/entity combination.
    Unsupported,
}

/// A constraint the plan removes for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DroppedConstraint {
    /// The removed constraint.
    pub id: ConstraintId,
    /// Why it could not be preserved.
    pub reason: DropReason,
}

/// A Trim/Extend plan: one atomic [`SketchEditOp`] batch plus an explicit
/// account of every identity and constraint consequence.
///
/// Constraint accounting is total: every constraint the batch removes appears
/// exactly once in `kept_constraints` (re-added under the same id, references
/// remapped) or `dropped_constraints`; every new id appears in
/// `added_constraints`. Nothing is dropped silently.
#[derive(Debug, Clone)]
pub struct ModifyPlan {
    /// The batch, in apply order.
    pub ops: Vec<SketchEditOp>,
    /// Surviving / reshaped curves, ordered along the original curve.
    pub pieces: Vec<PlannedPiece>,
    /// Entities that no longer exist after the batch (the target when no
    /// piece keeps its id, plus orphaned defining points).
    pub removed_entities: Vec<EntityId>,
    /// Fresh point entities (trim cut endpoints of a line).
    pub added_points: Vec<EntityId>,
    /// Constraints re-added under their original id with remapped references.
    pub kept_constraints: Vec<ConstraintId>,
    /// Constraints minted by the plan.
    pub added_constraints: Vec<AddedConstraint>,
    /// Constraints removed for good, with the reason.
    pub dropped_constraints: Vec<DroppedConstraint>,
}

/// An Extend plan plus where the grown end landed.
#[derive(Debug, Clone)]
pub struct ExtendPlan {
    /// The batch and its accounting.
    pub plan: ModifyPlan,
    /// Which end grew (`Start` or `End`).
    pub end: CurvePosition,
    /// The boundary intersection the end grew to.
    pub tip: Vec2,
}

/// Why a Trim/Extend was refused. A refusal has no side effects: the sketch is
/// untouched and no id was drawn from the allocator.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModifyRefusal {
    /// The entity is not in the sketch.
    #[error("entity {0} not found")]
    UnknownEntity(EntityId),
    /// Points are neither trimmed nor extended.
    #[error("entity {0} is a point, not a curve")]
    NotACurve(EntityId),
    /// Projected / host-face reference geometry is never modified.
    #[error("entity {0} is reference-locked and cannot be modified")]
    ReferenceLocked(EntityId),
    /// The core has no kind the result could be expressed in.
    #[error("{kind} {entity} cannot be {action}")]
    UnsupportedKind {
        /// The refused entity.
        entity: EntityId,
        /// Its kind.
        kind: &'static str,
        /// `trimmed` / `extended`.
        action: &'static str,
    },
    /// Zero-length line, zero radius/sweep or missing defining point.
    #[error("entity {0} is degenerate")]
    DegenerateTarget(EntityId),
    /// The pick sits on a boundary intersection (trim), exactly between both
    /// ends (extend), half-way around an arc's gap, or on an arc/circle
    /// centre, so the intended span/end is unknowable.
    #[error("pick on {0} is ambiguous")]
    AmbiguousPick(EntityId),
    /// A surviving piece or the extension would be shorter than
    /// [`MIN_PIECE_MM`](super::MIN_PIECE_MM).
    #[error("modifying {0} would leave degenerate geometry")]
    DegeneratePiece(EntityId),
    /// The end's continuation meets no boundary.
    #[error("no boundary ahead of {0}")]
    NoBoundaryHit(EntityId),
    /// The end already lies on a boundary sharing the target's carrier
    /// (collinear line, arc of the same circle) that continues ahead: the
    /// extension would run along it, never meet it. Also any trim cut or
    /// extend stop at an end point of an arc of the target's own circle.
    #[error("{entity} shares its carrier with boundary {boundary}")]
    OverlapsBoundary {
        /// The extended entity.
        entity: EntityId,
        /// The same-carrier boundary.
        boundary: EntityId,
    },
    /// The nearest boundary only touches the continuation tangentially.
    #[error("nearest boundary ahead of {0} is tangent")]
    TangentHit(EntityId),
    /// The end to grow is a point shared with another entity.
    #[error("end point {point} of {entity} is shared with other geometry")]
    EndpointShared {
        /// The extended entity.
        entity: EntityId,
        /// The shared point.
        point: EntityId,
    },
    /// Constraints pin the end (or the length) the extend would change.
    #[error("{entity} end is held by constraints {constraints:?}")]
    EndpointConstrained {
        /// The extended entity.
        entity: EntityId,
        /// The conflicting constraints, in sketch order.
        constraints: Vec<ConstraintId>,
    },
}
