//! Pure Trim / Extend planners (ONECAD-293 S293-a).
//!
//! Each planner reads an authoritative [`Sketch`] and returns either a
//! [`ModifyPlan`] — one atomic [`SketchEditOp`](crate::edit::SketchEditOp) batch
//! plus a total account of identity and constraint consequences — or a typed
//! [`ModifyRefusal`]. Nothing here mutates a sketch, talks to the worker or
//! decides admission; the app layer replans from the same intent under its
//! fence and submits the batch.
//!
//! **Trim** removes the picked span between the nearest boundary crossings
//! around the pick. Boundaries are every other sketch curve, construction and
//! reference geometry included; points never cut. A line or arc with no
//! crossing, or a circle with fewer than two, is deleted whole. Identity: the
//! surviving piece containing the original start keeps the entity id, other
//! pieces get fresh ids, and a kind change (circle → arc) always gets a fresh
//! id. Unchanged endpoints keep their point reference; cut endpoints are new
//! and are welded to the boundary (`Coincident` at an existing boundary
//! endpoint, otherwise `OnCurve`). An `OnCurve` holding that boundary endpoint
//! on the target — however the point is named — would restate the weld and is
//! dropped as superseded. Ellipses are refused: the core has no elliptical-arc
//! kind to express a trimmed ellipse.
//!
//! **Extend** grows the line/arc end nearer the pick along its own
//! continuation to the nearest boundary crossing; the end keeps its identity.
//! A `Coincident` weld at a boundary endpoint supersedes that point's `OnCurve`
//! on the target, as for trim. It refuses rather than guesses: no hit, a
//! tangent hit, a shared end point or any constraint pinning that end or the
//! line length.
//!
//! **Same-carrier boundaries** (a collinear line, an arc of the target's
//! circle) overlap rather than cross: their end points count as crossings. A
//! trim cuts there; an extend stops at the nearer one ahead and refuses
//! ([`ModifyRefusal::OverlapsBoundary`]) when its end already lies on one.
//!
//! **Ambiguous picks** are refused, never resolved arbitrarily: on a crossing
//! (trim), mid-way between the ends (extend), half-way around an arc's gap or
//! at an arc/circle centre (both).
//!
//! Tolerances follow the worker's sketch policy: [`COINCIDENT_MM`] is its
//! solver / loop-detection coincidence (`SOLVER_TOLERANCE`,
//! `LoopDetectorConfig::coincidenceTolerance`), [`MIN_PIECE_MM`] its minimum
//! geometry size (`MIN_GEOMETRY_SIZE`).

mod common;
mod extend;
mod geom;
mod intersect;
mod plan;
mod remap;
mod slots;
mod trim;
mod trim_emit;

#[cfg(test)]
mod tests;

pub use extend::plan_extend;
pub use plan::{
    AddReason, AddedConstraint, DropReason, DroppedConstraint, ExtendPlan, ModifyPlan,
    ModifyRefusal, PlannedPiece,
};
pub use trim::plan_trim;

use crate::ids::{ConstraintId, EntityId};
use crate::math::Vec2;

/// Two positions closer than this (mm) are the same point. Matches the worker's
/// solver tolerance and loop-detection coincidence: anything tighter would cut
/// where region detection already sees a weld.
pub const COINCIDENT_MM: f64 = 1e-4;

/// Smallest piece / extension (mm) a plan may create. Matches the worker's
/// `MIN_GEOMETRY_SIZE`; shorter geometry is refused, never silently absorbed.
pub const MIN_PIECE_MM: f64 = 0.01;

/// Source of fresh ids for new pieces, points and constraint copies. Injected
/// so a plan is reproducible (tests, preview vs commit replanning).
pub trait ModifyIds {
    /// A fresh entity id.
    fn entity_id(&mut self) -> EntityId;
    /// A fresh constraint id.
    fn constraint_id(&mut self) -> ConstraintId;
}

/// The production allocator: UUID v4, like every other Rust-minted sketch id.
#[derive(Debug, Default, Clone, Copy)]
pub struct FreshIds;

impl ModifyIds for FreshIds {
    fn entity_id(&mut self) -> EntityId {
        EntityId::new()
    }
    fn constraint_id(&mut self) -> ConstraintId {
        ConstraintId::new()
    }
}

/// Trim intent: remove the span of `entity` under `pick`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrimRequest {
    /// The curve to trim.
    pub entity: EntityId,
    /// Pick position in sketch coordinates (projected onto the curve).
    pub pick: Vec2,
}

/// Extend intent: grow the end of `entity` nearer `pick`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtendRequest {
    /// The line or arc to extend.
    pub entity: EntityId,
    /// Pick position in sketch coordinates; selects the end.
    pub pick: Vec2,
}
