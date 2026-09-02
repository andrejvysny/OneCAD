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

use serde::{Deserialize, Serialize};

use crate::document::refs::ElementKind;
use crate::ids::{BodyId, ConstraintId, ElementId, EntityId};
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
    /// A **full** ellipse — the parallel projection of a tilted, closed circular
    /// edge (SCHEMA §7.6 `ProjectToSketchPlane`). `ProjectFaceBoundary` never
    /// emits one.
    ///
    /// There is no elliptical ARC anywhere in the stack (no angular extent on the
    /// wire, in this enum, or in [`SketchEntity::Ellipse`]), which is why a
    /// TRIMMED tilted circular edge is refused by name on the wire instead of
    /// being approximated here.
    Ellipse {
        /// Centre point index.
        center: usize,
        /// Semi-major radius (mm), already `>= minor_r` (wire normalization).
        major_r: f64,
        /// Semi-minor radius (mm).
        minor_r: f64,
        /// Major-axis rotation (radians, CCW from +U).
        rotation: f64,
    },
}

/// Where one projected entity came from, plus the hash of the geometry it was
/// projected to (SCHEMA §7.6 `ProjectToSketchPlane` `sourceRef` + `projectedHash`).
///
/// **Persisted** on [`Sketch::projections`](crate::sketch::Sketch::projections),
/// which is what makes a projection re-runnable (update) and detachable, and what
/// gives the staleness check a baseline to compare against.
///
/// `projected_hash` covers the projected **UV geometry only** — never the source
/// in 3D, never the point refs, never emission order. Two edges projecting onto
/// the same 2D curve hash identically, deliberately: the question it answers is
/// "did the picture in this sketch change", not "did the model change", so a
/// source that slides along the sketch normal is NOT stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedSource {
    /// The body the source sub-shape belongs to. Persisted because re-running the
    /// verb addresses each source as `{bodyId, elementId}` — an `ElementId` alone
    /// cannot be turned back into a request.
    pub source_body: BodyId,
    /// The Rust-minted `ElementId` of the source edge / face.
    pub source_element_id: ElementId,
    /// Whether the source is an EDGE or a FACE. Persisted because it decides the
    /// re-request: an edge source is re-projected with `mode:"edges"`, a face
    /// source with `mode:"faceOutline"`, and sending the wrong one comes back as a
    /// `kindMismatch` refusal.
    pub source_kind: ElementKind,
    /// This entity's 0-based index within its SOURCE's emission run.
    ///
    /// Always `0` for an edge source (one edge projects to one curve); `0..n` for
    /// a face source under `faceOutline`, which emits its whole boundary. SCHEMA
    /// §7.6 makes emission order normative, so this ordinal is what re-associates
    /// a re-projected curve with the entity that already carries it — without it,
    /// a four-line face outline could only be replaced wholesale, discarding every
    /// user constraint hung off it and moving the region ids.
    pub source_ordinal: u32,
    /// FNV-1a 64-bit over the projected UV geometry, 16 lowercase hex chars
    /// (`quantizationVersion = 1`, `llround(v / 1e-6)`). Minted by the worker and
    /// carried VERBATIM — Rust never recomputes it.
    pub projected_hash: String,
}

/// The projected 2D content of ONE §7.6 response: merged points plus the curves
/// that reference them by index.
///
/// Shared by both projection verbs. `ProjectFaceBoundary` wraps it in a
/// [`ProjectionPayload`] (which adds the seed face's kernel-exact frame);
/// `ProjectToSketchPlane` returns it on its own, because a batch of picked edges
/// has no single face and therefore no frame to report. Keeping the frame OUT of
/// this struct is what lets the batch verb reuse the translator without
/// fabricating one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectedGeometry {
    /// Projected points in plane UV (mm), indexed by the wire's `p<N>` ordinal.
    pub points: Vec<Vec2>,
    /// Projected curves, in the worker's normative emission order.
    pub entities: Vec<ProjectedEntity>,
    /// Per-entity source + projected hash, **parallel to `entities`** (SCHEMA §7.6
    /// `ProjectToSketchPlane`). EMPTY for a `ProjectFaceBoundary` response, which
    /// has no per-entity source — an entity with no entry simply gets no
    /// `projections` map row.
    pub entity_sources: Vec<ProjectedSource>,
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
    /// The projected points + curves.
    pub geometry: ProjectedGeometry,
}

/// Translates a projected boundary into locked sketch content.
///
/// Returns `(entities, constraints, projections)` in **insertion order**: every
/// point first, then the curves that reference them, so feeding the result
/// straight into [`Sketch::add_entity`](crate::sketch::Sketch::add_entity) never
/// trips the dangling-reference check. The constraints are one
/// [`Constraint::Fixed`] per projected point, pinned at that point's own UV.
///
/// `projections` pairs each minted CURVE with its
/// [`ProjectedSource`], for the caller to write into
/// [`Sketch::projections`](crate::sketch::Sketch::projections). It is EMPTY when
/// the payload carries no `entity_sources` (every `ProjectFaceBoundary` payload),
/// and it never names a projected POINT: a point is shared by the curves that
/// meet at it, so it has no single source.
///
/// Ids come from the caller's minters (Invariant 1 — Rust mints identity); the
/// function itself is deterministic given deterministic minters.
///
/// An entity whose point index is out of range is **skipped** rather than
/// panicking; the parser is responsible for rejecting a malformed response
/// loudly, so this is defence in depth, not the primary guard. A skipped entity
/// contributes no `projections` row either — the map can never name an entity the
/// sketch does not have.
pub fn projected_sketch_content(
    geometry: &ProjectedGeometry,
    mint_entity: &mut impl FnMut() -> EntityId,
    mint_constraint: &mut impl FnMut() -> ConstraintId,
) -> (
    Vec<SketchEntity>,
    Vec<Constraint>,
    Vec<(EntityId, ProjectedSource)>,
) {
    let mut entities = Vec::with_capacity(geometry.points.len() + geometry.entities.len());
    let mut constraints = Vec::with_capacity(geometry.points.len());

    // One Point entity per payload point — the merge already happened in the
    // worker, so a shared endpoint is ONE point here and both adjacent curves
    // reference it (that is what makes the boundary a closed loop to the solver).
    let point_ids: Vec<EntityId> = geometry
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

    let mut projections: Vec<(EntityId, ProjectedSource)> = Vec::new();
    for (index, projected) in geometry.entities.iter().enumerate() {
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
            ProjectedEntity::Ellipse {
                center,
                major_r,
                minor_r,
                rotation,
            } => point_ids.get(center).and_then(|&c| {
                SketchEntity::ellipse(mint_entity(), c, major_r, minor_r, rotation, false)
            }),
        };
        if let Some(entity) = entity {
            let entity = entity.with_reference_locked(true);
            if let Some(source) = geometry.entity_sources.get(index) {
                projections.push((entity.id(), source.clone()));
            }
            entities.push(entity);
        }
    }

    (entities, constraints, projections)
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

    fn source(element: &str, hash: &str) -> ProjectedSource {
        ProjectedSource {
            source_body: crate::ids::BodyId(Uuid::from_u128(0xB0D1)),
            source_element_id: ElementId::new(element),
            source_kind: ElementKind::Edge,
            source_ordinal: 0,
            projected_hash: hash.to_string(),
        }
    }

    fn payload(points: &[[f64; 2]], entities: Vec<ProjectedEntity>) -> ProjectedGeometry {
        ProjectedGeometry {
            points: points
                .iter()
                .map(|p| Vec2::new_unchecked(p[0], p[1]))
                .collect(),
            entities,
            entity_sources: Vec::new(),
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
        let (entities, constraints, _) = projected_sketch_content(&p, &mut me, &mut mc);

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
        let (entities, constraints, _) = projected_sketch_content(&p, &mut me, &mut mc);

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
        let (entities, _, _) = projected_sketch_content(&p, &mut me, &mut mc);

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
        let (entities, constraints, _) = projected_sketch_content(&p, &mut me, &mut mc);

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

    /// A tilted, CLOSED circular edge projects to a full Ellipse (WP-P). It is
    /// locked like every other projected curve; §7.4 forbids naming an ellipse
    /// ENTITY in any curve-taking constraint, so `reference_locked` is the ONLY
    /// thing holding its `major_r`/`minor_r`/`rotation` — which is sound, because
    /// nothing can move them either.
    #[test]
    fn a_tilted_circle_projects_to_a_locked_full_ellipse() {
        let p = payload(
            &[[5.0, 5.0]],
            vec![ProjectedEntity::Ellipse {
                center: 0,
                major_r: 10.0,
                minor_r: 7.0710678118654755,
                rotation: 0.25,
            }],
        );
        let (mut me, mut mc) = minters();
        let (entities, constraints, _) = projected_sketch_content(&p, &mut me, &mut mc);

        assert_eq!(entities.len(), 2, "centre point + ellipse");
        assert_eq!(
            constraints.len(),
            1,
            "the CENTRE is Fixed; the radii are not"
        );
        let SketchEntity::Ellipse {
            center,
            major_r,
            minor_r,
            rotation,
            ..
        } = entities[1]
        else {
            panic!("expected an Ellipse, got {:?}", entities[1]);
        };
        assert_eq!(center, entities[0].id());
        assert!((major_r - 10.0).abs() < f64::EPSILON);
        assert!((minor_r - 7.0710678118654755).abs() < f64::EPSILON);
        assert!((rotation - 0.25).abs() < f64::EPSILON);
        assert!(entities[1].is_reference_locked());
    }

    /// `entity_sources` is parallel to `entities` and lands on the CURVES only —
    /// a projected point is shared by the curves meeting at it and has no single
    /// source.
    #[test]
    fn entity_sources_map_onto_the_minted_curves_not_the_points() {
        let mut p = payload(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            vec![
                ProjectedEntity::Line { p0: 0, p1: 1 },
                ProjectedEntity::Line { p0: 1, p1: 2 },
            ],
        );
        p.entity_sources = vec![source("el_a", "aaaa"), source("el_b", "bbbb")];

        let (mut me, mut mc) = minters();
        let (entities, _, projections) = projected_sketch_content(&p, &mut me, &mut mc);

        assert_eq!(
            projections.len(),
            2,
            "one row per CURVE (3 points, 2 lines)"
        );
        assert_eq!(projections[0].0, entities[3].id());
        assert_eq!(projections[1].0, entities[4].id());
        assert_eq!(projections[0].1.source_element_id.as_str(), "el_a");
        assert_eq!(projections[1].1.projected_hash, "bbbb");
    }

    /// A `ProjectFaceBoundary` payload carries no sources, so it writes no map
    /// rows — the field is additive, not a behaviour change for sketch-on-face.
    #[test]
    fn a_payload_without_sources_writes_no_projection_rows() {
        let p = payload(
            &[[0.0, 0.0], [1.0, 0.0]],
            vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
        );
        let (mut me, mut mc) = minters();
        let (_, _, projections) = projected_sketch_content(&p, &mut me, &mut mc);
        assert!(projections.is_empty());
    }

    /// A DROPPED curve (out-of-range ref) must not leave a `projections` row
    /// naming an entity the sketch never got.
    #[test]
    fn a_dropped_curve_contributes_no_projection_row() {
        let mut p = payload(
            &[[0.0, 0.0], [1.0, 0.0]],
            vec![
                ProjectedEntity::Line { p0: 0, p1: 9 },
                ProjectedEntity::Line { p0: 0, p1: 1 },
            ],
        );
        p.entity_sources = vec![source("el_dead", "dead"), source("el_live", "live")];
        let (mut me, mut mc) = minters();
        let (entities, _, projections) = projected_sketch_content(&p, &mut me, &mut mc);
        assert_eq!(entities.len(), 3, "2 points + the surviving line");
        assert_eq!(projections.len(), 1);
        assert_eq!(projections[0].0, entities[2].id());
        assert_eq!(projections[0].1.source_element_id.as_str(), "el_live");
    }

    /// Defence in depth: an out-of-range index drops that curve instead of
    /// panicking (the parser is the loud guard).
    #[test]
    fn an_out_of_range_point_index_drops_the_entity() {
        let p = payload(&[[0.0, 0.0]], vec![ProjectedEntity::Line { p0: 0, p1: 9 }]);
        let (mut me, mut mc) = minters();
        let (entities, _, _) = projected_sketch_content(&p, &mut me, &mut mc);
        assert_eq!(entities.len(), 1, "only the point survives");
    }
}
