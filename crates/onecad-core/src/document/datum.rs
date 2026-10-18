//! Datum (reference) planes usable as sketch planes.
//!
//! Ported faithfully from OneCAD-CPP `src/app/document/DatumPlane.h`. A datum
//! plane is defined **parametrically** (offset/angle relative to a base plane,
//! or offset from a model face) and carries a **cached resolved frame**. A
//! sketch placed on a datum copies the resolved frame at creation (frozen, like
//! sketch-on-face). The frame is re-derived by the worker in the regen epilogue
//! (V1/V2 §4.3); the core stores the parametric definition + the last resolved
//! frame.
//!
//! Serde: camelCase, no `deny_unknown_fields`; carries an `extra` flatten so
//! unknown fields round-trip. The typed-id fields (`base_body`, `base_face`,
//! `axis_edge`) are `Option`s — C++ stores them as (possibly empty) strings.

use serde::{Deserialize, Serialize};

use crate::document::refs::Extra;
use crate::ids::{BodyId, DatumPlaneId, ElementId};
use crate::math::Vec3;
use crate::sketch::SketchPlane;

/// How a datum plane's frame is derived (C++ `DatumPlane::Kind`). Serialized as
/// the PascalCase token (`datumPlaneKindName` in C++).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DatumKind {
    /// Offset along a base origin/datum plane normal.
    #[default]
    OffsetFromPlane,
    /// Offset from a model face (frozen snapshot).
    OffsetFromFace,
    /// Rotate a base plane about an edge by `angle_deg`.
    AngledFromEdge,
    /// Frame from three points (reserved).
    ThreePoint,
}

impl DatumKind {
    /// The PascalCase token this kind serializes as. Lock-tested against the
    /// serde output so a projection DTO can render the discriminator without
    /// re-serializing (and can never drift from the wire spelling).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OffsetFromPlane => "OffsetFromPlane",
            Self::OffsetFromFace => "OffsetFromFace",
            Self::AngledFromEdge => "AngledFromEdge",
            Self::ThreePoint => "ThreePoint",
        }
    }
}

/// A user-created reference plane (C++ `DatumPlane`).
///
/// The definition fields are parametric and re-derivable; `resolved_plane` /
/// `resolved_valid` cache the last resolved frame (the source of truth for
/// sketches placed on this datum).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatumPlane {
    /// Stable datum identity (C++ `id`, a UUID string).
    pub id: DatumPlaneId,
    /// Human-facing name.
    pub name: String,
    /// How the frame is derived.
    pub kind: DatumKind,

    // ── Parametric definition (re-derivable) ────────────────────────────────
    /// `"XY"`/`"XZ"`/`"YZ"` or another datum id (Offset/Angled). C++
    /// `basePlaneId` — a heterogeneous reference, kept as a raw string.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_plane_id: String,
    /// Owner body of `base_face` (`OffsetFromFace`). C++ `baseBodyId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_body: Option<BodyId>,
    /// The model face this datum is offset from (`OffsetFromFace`, frozen). C++
    /// `baseFaceId` (an ElementMap id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_face: Option<ElementId>,
    /// The edge this datum is rotated about (`AngledFromEdge`). C++ `axisEdgeId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_edge: Option<ElementId>,
    /// Offset distance (mm).
    pub offset: f64,
    /// Rotation angle (degrees).
    pub angle_deg: f64,

    // ── Cached resolved frame ───────────────────────────────────────────────
    /// The resolved coordinate frame (source of truth for placed sketches).
    pub resolved_plane: SketchPlane,
    /// Whether `resolved_plane` is currently valid.
    pub resolved_valid: bool,

    /// Unknown keys, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl DatumPlane {
    /// An `OffsetFromPlane` datum offset from a named world plane
    /// (`"XY"`/`"XZ"`/`"YZ"`), with an as-yet-unresolved frame.
    #[must_use]
    pub fn offset_from_plane(
        id: DatumPlaneId,
        name: impl Into<String>,
        base_plane_id: impl Into<String>,
        offset: f64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            kind: DatumKind::OffsetFromPlane,
            base_plane_id: base_plane_id.into(),
            base_body: None,
            base_face: None,
            axis_edge: None,
            offset,
            angle_deg: 0.0,
            resolved_plane: SketchPlane::xy(),
            resolved_valid: false,
            extra: Extra::new(),
        }
    }

    /// Sets the cached resolved frame (worker regen epilogue).
    pub fn set_resolved(&mut self, plane: SketchPlane) {
        self.resolved_plane = plane;
        self.resolved_valid = true;
    }
}

/// What a datum's definition refers to, supplied by the caller because the core
/// cannot look it up: a base world plane / another datum's frame, and (for
/// `OffsetFromFace` / `AngledFromEdge`) geometry that only the kernel knows.
#[derive(Debug, Clone, PartialEq)]
pub struct DatumContext {
    /// The frame `base_plane_id` names — a world plane's basis, or another
    /// datum's already-resolved frame.
    pub base: SketchPlane,
    /// The host face's frame (`OffsetFromFace`), from `QueryElement`.
    pub face: Option<SketchPlane>,
    /// The rotation axis for `AngledFromEdge`, as a world direction.
    pub axis_dir: Option<Vec3>,
    /// A point on that axis.
    pub axis_origin: Option<Vec3>,
}

/// Resolves a datum's parametric definition into a concrete frame.
///
/// PURE — every input the core cannot derive itself arrives in [`DatumContext`],
/// so this is fully unit-testable and carries no worker dependency. The caller
/// stores the result with [`DatumPlane::set_resolved`].
///
/// Returns `None` when the definition is unsatisfiable with the given context
/// (a missing face frame, a missing axis, an unimplemented kind) — the datum then
/// stays `resolved_valid == false` rather than silently resolving to somewhere
/// arbitrary, which is the same "never a silent wrong bind" rule the region and
/// element paths follow.
#[must_use]
pub fn resolve_datum(datum: &DatumPlane, ctx: &DatumContext) -> Option<SketchPlane> {
    match datum.kind {
        // Slide the base frame along its own normal. Axes are carried over
        // verbatim so an offset datum keeps the base plane's orientation — a
        // sketch on "XY offset 10" is oriented exactly like a sketch on XY.
        DatumKind::OffsetFromPlane => {
            let n = ctx.base.normal;
            Some(SketchPlane {
                origin: Vec3::new_unchecked(
                    ctx.base.origin.x + n.x * datum.offset,
                    ctx.base.origin.y + n.y * datum.offset,
                    ctx.base.origin.z + n.z * datum.offset,
                ),
                ..ctx.base
            })
        }
        // Same, but the base frame comes from a model face the caller resolved.
        DatumKind::OffsetFromFace => {
            let face = ctx.face?;
            let n = face.normal;
            Some(SketchPlane {
                origin: Vec3::new_unchecked(
                    face.origin.x + n.x * datum.offset,
                    face.origin.y + n.y * datum.offset,
                    face.origin.z + n.z * datum.offset,
                ),
                ..face
            })
        }
        // Rotate the base frame about an edge by `angle_deg` (Rodrigues).
        DatumKind::AngledFromEdge => {
            let axis = ctx.axis_dir?;
            let len = (axis.x * axis.x + axis.y * axis.y + axis.z * axis.z).sqrt();
            if !len.is_finite() || len < 1e-12 {
                return None;
            }
            let (ax, ay, az) = (axis.x / len, axis.y / len, axis.z / len);
            let theta = datum.angle_deg.to_radians();
            let (s, c) = theta.sin_cos();
            let rot = |v: Vec3| -> Vec3 {
                // v·cos + (k × v)·sin + k(k·v)(1 − cos)
                let kv = ax * v.x + ay * v.y + az * v.z;
                let cx = ay * v.z - az * v.y;
                let cy = az * v.x - ax * v.z;
                let cz = ax * v.y - ay * v.x;
                Vec3::new_unchecked(
                    v.x * c + cx * s + ax * kv * (1.0 - c),
                    v.y * c + cy * s + ay * kv * (1.0 - c),
                    v.z * c + cz * s + az * kv * (1.0 - c),
                )
            };
            // The origin rotates ABOUT the axis point, not about the world origin.
            let pivot = ctx.axis_origin.unwrap_or(ctx.base.origin);
            let rel = Vec3::new_unchecked(
                ctx.base.origin.x - pivot.x,
                ctx.base.origin.y - pivot.y,
                ctx.base.origin.z - pivot.z,
            );
            let rel_rot = rot(rel);
            Some(SketchPlane {
                origin: Vec3::new_unchecked(
                    pivot.x + rel_rot.x,
                    pivot.y + rel_rot.y,
                    pivot.z + rel_rot.z,
                ),
                x_axis: rot(ctx.base.x_axis),
                y_axis: rot(ctx.base.y_axis),
                normal: rot(ctx.base.normal),
            })
        }
        // Reserved in the C++ model; no definition fields exist for it yet, so it
        // resolves to nothing rather than guessing a frame.
        DatumKind::ThreePoint => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn did(n: u128) -> DatumPlaneId {
        DatumPlaneId(Uuid::from_u128(n))
    }

    #[test]
    fn datum_round_trips_and_kind_is_pascal_case() {
        let mut d = DatumPlane::offset_from_plane(did(1), "Datum 1", "XY", 10.0);
        d.set_resolved(SketchPlane::xz());
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "OffsetFromPlane");
        assert_eq!(v["basePlaneId"], "XY");
        assert_eq!(v["resolvedValid"], true);
        let back: DatumPlane = serde_json::from_value(v).unwrap();
        assert_eq!(d, back);
    }

    /// LOCK — `DatumKind::name()` must equal the serde token for every variant
    /// (the projection DTO renders `kind` from it without re-serializing).
    #[test]
    fn kind_name_matches_the_serde_token() {
        for k in [
            DatumKind::OffsetFromPlane,
            DatumKind::OffsetFromFace,
            DatumKind::AngledFromEdge,
            DatumKind::ThreePoint,
        ] {
            assert_eq!(serde_json::to_value(k).unwrap(), k.name(), "{k:?}");
        }
    }

    #[test]
    fn offset_from_face_carries_typed_refs() {
        let mut d = DatumPlane::offset_from_plane(did(2), "Datum 2", "", 5.0);
        d.kind = DatumKind::OffsetFromFace;
        d.base_body = Some(BodyId(Uuid::from_u128(0xB0)));
        d.base_face = Some(ElementId::new("el_face_9"));
        let back: DatumPlane = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(d, back);
    }

    // ── resolve_datum ───────────────────────────────────────────────────────

    fn ctx(base: SketchPlane) -> DatumContext {
        DatumContext {
            base,
            face: None,
            axis_dir: None,
            axis_origin: None,
        }
    }

    #[test]
    fn offset_from_plane_slides_along_the_base_normal_and_keeps_its_axes() {
        let d = DatumPlane::offset_from_plane(did(1), "D1", "XY", 10.0);
        let base = SketchPlane::xy();
        let r = resolve_datum(&d, &ctx(base)).expect("resolves");
        // XY's normal is +Z, so a 10mm offset lands at z = 10.
        assert_eq!(r.origin, Vec3::new_unchecked(0.0, 0.0, 10.0));
        // Orientation is carried over verbatim: a sketch on "XY offset 10" reads
        // exactly like a sketch on XY.
        assert_eq!(r.x_axis, base.x_axis);
        assert_eq!(r.y_axis, base.y_axis);
        assert_eq!(r.normal, base.normal);
    }

    #[test]
    fn a_negative_offset_goes_the_other_way() {
        let d = DatumPlane::offset_from_plane(did(2), "D2", "XY", -4.0);
        let r = resolve_datum(&d, &ctx(SketchPlane::xy())).unwrap();
        assert_eq!(r.origin, Vec3::new_unchecked(0.0, 0.0, -4.0));
    }

    #[test]
    fn offset_from_face_needs_a_face_frame_and_never_guesses_one() {
        let mut d = DatumPlane::offset_from_plane(did(3), "D3", "", 5.0);
        d.kind = DatumKind::OffsetFromFace;
        // No face supplied ⇒ unresolvable, NOT silently resolved to the base.
        assert!(resolve_datum(&d, &ctx(SketchPlane::xy())).is_none());

        let face = SketchPlane {
            origin: Vec3::new_unchecked(1.0, 2.0, 3.0),
            ..SketchPlane::xy()
        };
        let mut c = ctx(SketchPlane::xy());
        c.face = Some(face);
        let r = resolve_datum(&d, &c).unwrap();
        assert_eq!(r.origin, Vec3::new_unchecked(1.0, 2.0, 8.0));
    }

    #[test]
    fn angled_from_edge_rotates_about_the_axis_through_its_pivot() {
        let mut d = DatumPlane::offset_from_plane(did(4), "D4", "XY", 0.0);
        d.kind = DatumKind::AngledFromEdge;
        d.angle_deg = 90.0;
        let mut c = ctx(SketchPlane::xy());
        c.axis_dir = Some(Vec3::new_unchecked(1.0, 0.0, 0.0)); // world +X
        c.axis_origin = Some(Vec3::new_unchecked(0.0, 0.0, 0.0));
        let r = resolve_datum(&d, &c).unwrap();
        // XY's normal +Z rotated +90° about +X → −Y (right-hand rule: k × v with
        // k=+X, v=+Z is (0,−1,0)).
        let close = |v: Vec3, x: f64, y: f64, z: f64| {
            (v.x - x).abs() < 1e-12 && (v.y - y).abs() < 1e-12 && (v.z - z).abs() < 1e-12
        };
        assert!(close(r.normal, 0.0, -1.0, 0.0), "normal {:?}", r.normal);
        // The frame stays orthonormal through the rotation.
        let dot = r.x_axis.x * r.y_axis.x + r.x_axis.y * r.y_axis.y + r.x_axis.z * r.y_axis.z;
        assert!(dot.abs() < 1e-12);
    }

    #[test]
    fn angled_from_edge_without_an_axis_or_with_a_degenerate_one_is_unresolvable() {
        let mut d = DatumPlane::offset_from_plane(did(5), "D5", "XY", 0.0);
        d.kind = DatumKind::AngledFromEdge;
        d.angle_deg = 30.0;
        assert!(resolve_datum(&d, &ctx(SketchPlane::xy())).is_none());
        let mut c = ctx(SketchPlane::xy());
        c.axis_dir = Some(Vec3::new_unchecked(0.0, 0.0, 0.0));
        assert!(resolve_datum(&d, &c).is_none());
    }

    #[test]
    fn three_point_is_reserved_and_resolves_to_nothing_rather_than_a_guess() {
        let mut d = DatumPlane::offset_from_plane(did(6), "D6", "XY", 1.0);
        d.kind = DatumKind::ThreePoint;
        assert!(resolve_datum(&d, &ctx(SketchPlane::xy())).is_none());
    }

    #[test]
    fn a_resolved_datum_is_stored_and_marked_valid() {
        let mut d = DatumPlane::offset_from_plane(did(7), "D7", "XY", 3.0);
        assert!(!d.resolved_valid);
        let r = resolve_datum(&d, &ctx(SketchPlane::xy())).unwrap();
        d.set_resolved(r);
        assert!(d.resolved_valid);
        assert_eq!(d.resolved_plane.origin, Vec3::new_unchecked(0.0, 0.0, 3.0));
    }
}
