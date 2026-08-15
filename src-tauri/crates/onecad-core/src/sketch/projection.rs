//! Host-face boundary projection → locked sketch geometry (SCHEMA §7.6
//! `ProjectFaceBoundary`).
//!
//! **Pure.** This module owns the *translation* half of sketch-on-face: it turns
//! an already-parsed [`ProjectionPayload`] into the [`SketchEntity`] /
//! [`Constraint`] pair a fresh [`Sketch`](crate::sketch::Sketch) is built from.
//! It knows nothing about the worker, the wire, or the transport — the app layer
//! parses the §7.6 response into a [`ProjectionPayload`] and calls
//! [`projected_sketch_content`].
//!
//! ## What the payload means
//!
//! Coordinates are **mm in the supplied plane's UV** — the plane is the caller's
//! (SCHEMA §7.6 "the `plane` argument is authoritative"), so no basis work
//! happens here. Points are already merged within the worker's
//! `pointMergeTolerance`, so **a point index reused across entities IS the same
//! point**: that is how adjacent boundary curves share an endpoint, and it is why
//! this module mints exactly one [`SketchEntity::Point`] per payload point rather
//! than one per curve end.
//!
//! ## The two invariants this module upholds
//!
//! * **Everything is `reference_locked`** (never `construction`). Locked geometry
//!   is selectable and snappable, refuses every geometry-mutating edit
//!   ([`SketchError::ReferenceLocked`](crate::sketch::SketchError::ReferenceLocked)),
//!   and — the deliberate contrast with construction geometry — DOES bound
//!   regions (SCHEMA §7.3). A projected boundary that did not bound regions would
//!   be decorative.
//! * **Every projected point gets a [`Constraint::Fixed`]** at its own UV. The
//!   sketch these entities land in is brand new, so every point here is
//!   newly created and none can be a pre-existing user point — mirroring the
//!   oracle's `pointsToFix` semantics (`OneCAD-CPP` pinned exactly the points its
//!   projection created). Pinning is what makes the boundary immovable *in the
//!   solver* as well as in the edit layer, so a user dimension against the
//!   boundary drives the user's geometry rather than sliding the face outline.
//!
//! ## Arc angles are carried VERBATIM
//!
//! SCHEMA §7.6 is explicit: an arc's `startAngle`/`endAngle` are **always the
//! CCW-ordered pair** (sweeping CCW from `startAngle` reaches `endAngle`), and
//! `ccw` reports only the direction of the UNDERLYING kernel curve. The worker
//! already performs the swap (`worker/src/sketch/FaceBoundaryProjector.cpp` —
//! `if (!ccw) std::swap(arcStart, arcEnd);`). [`SketchEntity::Arc`] uses the same
//! CCW convention, so re-swapping on a `ccw:false` arc would invert a correct
//! sweep. `ccw` is therefore informational here and is deliberately NOT applied.

use crate::ids::{ConstraintId, EntityId};
use crate::math::{Vec2, Vec3};
use crate::sketch::{Constraint, CurvePosition, SketchEntity};

/// The kernel-exact frame of the seed face (SCHEMA §7.6 `exact`): the `gp_Pln`
/// origin plus the orientation-corrected unit normal (reversed for a
/// `TopAbs_REVERSED` face, so it points out of the solid).
///
/// The origin lies **ON** the face plane — unlike an element descriptor's
/// `center`, which is an axis-aligned bbox centre and sits off-plane for a tilted
/// face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceFrame {
    /// Plane origin in world coordinates (mm).
    pub origin: Vec3,
    /// Orientation-corrected unit normal.
    pub normal: Vec3,
}

impl FaceFrame {
    /// True when both frames agree componentwise within `eps`.
    ///
    /// Used as the §7.6 handshake **tripwire**: the second round-trip echoes
    /// `exact`, and it must still describe the frame the first round-trip
    /// reported, or the head moved underneath the two calls.
    #[must_use]
    pub fn approx_eq(&self, other: &Self, eps: f64) -> bool {
        self.origin.approx_eq(&other.origin, eps) && self.normal.approx_eq(&other.normal, eps)
    }
}

/// One projected boundary curve. Point references are **indices into
/// [`ProjectionPayload::points`]** — the wire's response-local `p<N>` refs,
/// already resolved and bounds-checked by the parser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProjectedEntity {
    /// A straight segment between two projected points.
    Line {
        /// Start point index.
        p0: usize,
        /// End point index.
        p1: usize,
    },
    /// A full circular edge.
    Circle {
        /// Centre point index.
        center: usize,
        /// Radius (mm).
        radius: f64,
    },
    /// A circular arc. `start_angle`/`end_angle` are radians CCW from the plane's
    /// +U axis and are ALREADY the CCW-ordered pair (see the module docs).
    Arc {
        /// Centre point index.
        center: usize,
        /// Radius (mm).
        radius: f64,
        /// Start angle (radians, CCW from +U).
        start_angle: f64,
        /// End angle (radians, CCW from +U).
        end_angle: f64,
        /// Direction of the underlying kernel curve — **informational**; the
        /// angle pair is CCW-ordered regardless.
        ccw: bool,
    },
}

/// A parsed SCHEMA §7.6 `ProjectFaceBoundary` result (the `present: true` case).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionPayload {
    /// The seed face's kernel-exact frame.
    pub exact: FaceFrame,
    /// Whether the projected geometry closes into at least one boundary.
    pub has_closed_boundary: bool,
    /// How many faces were projected (seed + coplanar companions).
    pub face_count: u32,
    /// Projected points in plane UV (mm), indexed by the wire's `p<N>` ordinal.
    pub points: Vec<Vec2>,
    /// Projected boundary curves, in the worker's normative emission order.
    pub entities: Vec<ProjectedEntity>,
}

/// Translates a projected face boundary into locked sketch content.
///
/// Returns `(entities, constraints)` in **insertion order**: every point first,
/// then the curves that reference them, so feeding the result straight into
/// [`Sketch::add_entity`](crate::sketch::Sketch::add_entity) never trips the
/// dangling-reference check. The constraints are one [`Constraint::Fixed`] per
/// projected point, pinned at that point's own UV.
///
/// Ids come from the caller's minters (Invariant 1 — Rust mints identity); the
/// function itself is deterministic given deterministic minters.
///
/// An entity whose point index is out of range is **skipped** rather than
/// panicking; the parser is responsible for rejecting a malformed response
/// loudly, so this is defence in depth, not the primary guard.
pub fn projected_sketch_content(
    payload: &ProjectionPayload,
    mint_entity: &mut impl FnMut() -> EntityId,
    mint_constraint: &mut impl FnMut() -> ConstraintId,
) -> (Vec<SketchEntity>, Vec<Constraint>) {
    let mut entities = Vec::with_capacity(payload.points.len() + payload.entities.len());
    let mut constraints = Vec::with_capacity(payload.points.len());

    // One Point entity per payload point — the merge already happened in the
    // worker, so a shared endpoint is ONE point here and both adjacent curves
    // reference it (that is what makes the boundary a closed loop to the solver).
    let point_ids: Vec<EntityId> = payload
        .points
        .iter()
        .map(|at| {
            let id = mint_entity();
            entities.push(SketchEntity::point(id, *at, false, true));
            constraints.push(Constraint::Fixed {
                id: mint_constraint(),
                point: id,
                // A projected point IS the point — no owner+role indirection.
                point_position: CurvePosition::Arbitrary,
                at: *at,
            });
            id
        })
        .collect();

    for projected in &payload.entities {
        let entity = match *projected {
            ProjectedEntity::Line { p0, p1 } => match (point_ids.get(p0), point_ids.get(p1)) {
                (Some(&start), Some(&end)) => {
                    Some(SketchEntity::line(mint_entity(), start, end, false))
                }
                _ => None,
            },
            ProjectedEntity::Circle { center, radius } => point_ids
                .get(center)
                .and_then(|&c| SketchEntity::circle(mint_entity(), c, radius, false)),
            ProjectedEntity::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                // `ccw` is NOT applied: the wire pair is already CCW-ordered
                // (SCHEMA §7.6), and `SketchEntity::Arc` uses that same
                // convention. See the module docs.
                ccw: _,
            } => point_ids.get(center).and_then(|&c| {
                SketchEntity::arc(mint_entity(), c, radius, start_angle, end_angle, false)
            }),
        };
        if let Some(entity) = entity {
            entities.push(entity.with_reference_locked(true));
        }
    }

    (entities, constraints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;
    use uuid::Uuid;

    /// Deterministic minters so a fixture's ids are assertable.
    fn minters() -> (impl FnMut() -> EntityId, impl FnMut() -> ConstraintId) {
        let mut e = 0u128;
        let mut c = 0u128;
        (
            move || {
                e += 1;
                EntityId(Uuid::from_u128(0xE000 + e))
            },
            move || {
                c += 1;
                ConstraintId(Uuid::from_u128(0xC000 + c))
            },
        )
    }

    fn payload(points: &[[f64; 2]], entities: Vec<ProjectedEntity>) -> ProjectionPayload {
        ProjectionPayload {
            exact: FaceFrame {
                origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            },
            has_closed_boundary: true,
            face_count: 1,
            points: points
                .iter()
                .map(|p| Vec2::new_unchecked(p[0], p[1]))
                .collect(),
            entities,
        }
    }

    /// The headline shape: a projected rectangle is 4 points + 4 lines, every one
    /// locked, plus one Fixed per point.
    #[test]
    fn a_rect_projects_to_four_locked_points_four_locked_lines_and_four_fixed() {
        let p = payload(
            &[[0.0, 0.0], [80.0, 0.0], [80.0, 60.0], [0.0, 60.0]],
            vec![
                ProjectedEntity::Line { p0: 0, p1: 1 },
                ProjectedEntity::Line { p0: 1, p1: 2 },
                ProjectedEntity::Line { p0: 2, p1: 3 },
                ProjectedEntity::Line { p0: 3, p1: 0 },
            ],
        );
        let (mut me, mut mc) = minters();
        let (entities, constraints) = projected_sketch_content(&p, &mut me, &mut mc);

        assert_eq!(entities.len(), 8, "4 points + 4 lines");
        assert_eq!(constraints.len(), 4, "one Fixed per projected point");
        assert!(
            entities.iter().all(SketchEntity::is_reference_locked),
            "EVERY projected entity is locked — points included"
        );
        assert!(
            !entities.iter().any(SketchEntity::is_construction),
            "projected geometry is real, not construction (it must bound regions)"
        );
        // Points come first so add_entity's dangling-ref check never trips.
        assert!(
            entities[..4]
                .iter()
                .all(|e| matches!(e, SketchEntity::Point { .. })),
            "points are emitted before the curves that reference them"
        );
        assert_eq!(
            entities[4..]
                .iter()
                .filter(|e| matches!(e, SketchEntity::Line { .. }))
                .count(),
            4
        );
        for (i, c) in constraints.iter().enumerate() {
            let Constraint::Fixed { point, at, .. } = c else {
                panic!("expected Fixed, got {c:?}");
            };
            assert_eq!(*point, entities[i].id());
            assert_eq!(*at, p.points[i], "pinned at its OWN projected UV");
        }
    }

    /// A circular face boundary stays a Circle (no polyline fallback) and its
    /// centre is a locked, Fixed point like any other.
    #[test]
    fn a_circle_keeps_its_center_point_and_radius() {
        let p = payload(
            &[[10.0, 20.0]],
            vec![ProjectedEntity::Circle {
                center: 0,
                radius: 7.5,
            }],
        );
        let (mut me, mut mc) = minters();
        let (entities, constraints) = projected_sketch_content(&p, &mut me, &mut mc);

        assert_eq!(entities.len(), 2, "centre point + circle");
        assert_eq!(constraints.len(), 1);
        let SketchEntity::Circle { center, radius, .. } = entities[1] else {
            panic!("expected a Circle, got {:?}", entities[1]);
        };
        assert_eq!(
            center,
            entities[0].id(),
            "the circle owns the minted centre"
        );
        assert!((radius - 7.5).abs() < f64::EPSILON);
        assert!(entities[1].is_reference_locked());
    }

    /// LOCK — a `ccw:false` arc's angles are carried **verbatim**.
    ///
    /// SCHEMA §7.6: `startAngle`/`endAngle` are ALWAYS the CCW-ordered pair;
    /// `ccw` reports the underlying kernel curve's direction only. The worker
    /// already swapped (`FaceBoundaryProjector.cpp`), so swapping again here
    /// would invert a correct sweep into its complement.
    #[test]
    fn a_ccw_false_arc_is_not_re_swapped() {
        let p = payload(
            &[[0.0, 0.0]],
            vec![ProjectedEntity::Arc {
                center: 0,
                radius: 5.0,
                start_angle: 0.0,
                end_angle: FRAC_PI_2,
                ccw: false,
            }],
        );
        let (mut me, mut mc) = minters();
        let (entities, _) = projected_sketch_content(&p, &mut me, &mut mc);

        let SketchEntity::Arc {
            start_angle,
            end_angle,
            ..
        } = entities[1]
        else {
            panic!("expected an Arc, got {:?}", entities[1]);
        };
        assert!(
            (start_angle - 0.0).abs() < f64::EPSILON
                && (end_angle - FRAC_PI_2).abs() < f64::EPSILON,
            "the wire pair is already CCW-ordered — re-swapping inverts the sweep \
             (got {start_angle}..{end_angle})"
        );
    }

    /// A point index reused across entities IS the same point (SCHEMA §7.6
    /// merge rule) — it must be minted ONCE, not once per curve end, or the
    /// "closed loop" the region detector needs never closes.
    #[test]
    fn a_shared_endpoint_is_minted_once_and_reused() {
        let p = payload(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            vec![
                ProjectedEntity::Line { p0: 0, p1: 1 },
                ProjectedEntity::Line { p0: 1, p1: 2 },
            ],
        );
        let (mut me, mut mc) = minters();
        let (entities, constraints) = projected_sketch_content(&p, &mut me, &mut mc);

        assert_eq!(entities.len(), 5, "3 points + 2 lines (NOT 4 points)");
        assert_eq!(constraints.len(), 3, "one Fixed per DISTINCT point");
        let SketchEntity::Line { end, .. } = entities[3] else {
            panic!("expected a Line");
        };
        let SketchEntity::Line { start, .. } = entities[4] else {
            panic!("expected a Line");
        };
        assert_eq!(end, start, "p1 is one entity shared by both lines");
        assert_eq!(end, entities[1].id());
    }

    /// Defence in depth: an out-of-range index drops that curve instead of
    /// panicking (the parser is the loud guard).
    #[test]
    fn an_out_of_range_point_index_drops_the_entity() {
        let p = payload(&[[0.0, 0.0]], vec![ProjectedEntity::Line { p0: 0, p1: 9 }]);
        let (mut me, mut mc) = minters();
        let (entities, _) = projected_sketch_content(&p, &mut me, &mut mc);
        assert_eq!(entities.len(), 1, "only the point survives");
    }
}
