//! The sketch coordinate plane.
//!
//! Ports OneCAD-CPP's **NON-STANDARD** basis VERBATIM from
//! `OneCAD-CPP/src/core/sketch/Sketch.h` (`SketchPlane::XY()/XZ()/YZ()` +
//! `toWorld`/`toSketch`). Do NOT "fix" these to a conventional basis —
//! downstream world geometry and every stored file depend on the exact numbers.
//! A lock-test in this module pins them (failure message cites `Sketch.h`).
//!
//! The named-plane bases (from `Sketch.h`):
//!
//! | plane | x_axis     | y_axis     | normal    | note (Sketch.h)                       |
//! |-------|------------|------------|-----------|---------------------------------------|
//! | XY    | (0, 1, 0)  | (-1, 0, 0) | (0, 0, 1) | Sketch X → World Y+, Sketch Y → World X- |
//! | XZ    | (0, 1, 0)  | (0, 0, 1)  | (1, 0, 0) | User X = Geom Y+, User Z = Geom Z+     |
//! | YZ    | (-1, 0, 0) | (0, 0, 1)  | (0, 1, 0) | User Y = Geom X-, User Z = Geom Z+     |
//!
//! Serde: camelCase (`origin`, `xAxis`, `yAxis`, `normal`) — aligns with the
//! SCHEMA §7.3 `plane` basis field names (the SCHEMA `kind` discriminator is
//! carried by [`SketchAttachment`](crate::sketch::SketchAttachment) in the
//! domain, not by this frame). Unknown fields (e.g. an incoming `kind`) are
//! ignored, so a SCHEMA §7.3 `plane` object parses straight into `SketchPlane`.

use serde::{Deserialize, Serialize};

use crate::math::{Vec2, Vec3};

/// World-space direction that a sketch's local +X axis maps to on the XY plane
/// (Sketch.h `SketchPlane::XY()`): `(0, 1, 0)`.
pub const XY_LOCAL_X_IN_WORLD: [f64; 3] = [0.0, 1.0, 0.0];

/// World-space direction that a sketch's local +Y axis maps to on the XY plane
/// (Sketch.h `SketchPlane::XY()`): `(-1, 0, 0)`.
pub const XY_LOCAL_Y_IN_WORLD: [f64; 3] = [-1.0, 0.0, 0.0];

/// A sketch's coordinate frame in world space: origin + orthonormal basis.
///
/// The basis is carried **verbatim** (never re-derived). See the module docs
/// for the non-standard named-plane bases ported from `Sketch.h`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchPlane {
    /// Plane origin in world coordinates.
    pub origin: Vec3,
    /// World direction of the sketch's local +X axis.
    pub x_axis: Vec3,
    /// World direction of the sketch's local +Y axis.
    pub y_axis: Vec3,
    /// Plane normal (`x_axis × y_axis`), carried verbatim.
    pub normal: Vec3,
}

impl SketchPlane {
    /// Default **XY** plane — NON-STANDARD basis ported verbatim from `Sketch.h`
    /// `SketchPlane::XY()`: `x=(0,1,0)`, `y=(-1,0,0)`, `n=(0,0,1)` (Sketch X →
    /// World Y+, Sketch Y → World X-).
    #[must_use]
    pub const fn xy() -> Self {
        Self {
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        }
    }

    /// **XZ** plane — `Sketch.h` `SketchPlane::XZ()`: `x=(0,1,0)`, `y=(0,0,1)`,
    /// `n=(1,0,0)`.
    #[must_use]
    pub const fn xz() -> Self {
        Self {
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            y_axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
            normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
        }
    }

    /// **YZ** plane — `Sketch.h` `SketchPlane::YZ()`: `x=(-1,0,0)`, `y=(0,0,1)`,
    /// `n=(0,1,0)`.
    #[must_use]
    pub const fn yz() -> Self {
        Self {
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
            y_axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
            normal: Vec3::new_unchecked(0.0, 1.0, 0.0),
        }
    }

    /// Constructs a plane from an arbitrary frame, rejecting any non-finite
    /// component (SCHEMA §4). Does NOT check orthonormality — the basis is
    /// carried verbatim, matching [`crate::math::Plane::new`].
    #[must_use]
    pub fn new(origin: Vec3, x_axis: Vec3, y_axis: Vec3, normal: Vec3) -> Option<Self> {
        let p = Self {
            origin,
            x_axis,
            y_axis,
            normal,
        };
        (origin.is_finite() && x_axis.is_finite() && y_axis.is_finite() && normal.is_finite())
            .then_some(p)
    }

    /// Converts a 2D sketch point to 3D world coordinates.
    ///
    /// Ports `Sketch.h` `toWorld`: `origin + p.x·x_axis + p.y·y_axis`.
    #[must_use]
    pub fn to_world(&self, p: Vec2) -> Vec3 {
        Vec3::new_unchecked(
            self.origin.x + p.x * self.x_axis.x + p.y * self.y_axis.x,
            self.origin.y + p.x * self.x_axis.y + p.y * self.y_axis.y,
            self.origin.z + p.x * self.x_axis.z + p.y * self.y_axis.z,
        )
    }

    /// Projects a 3D world point to 2D sketch coordinates.
    ///
    /// Ports `Sketch.h` `toSketch`: with `rel = p3d − origin`,
    /// `(rel·x_axis, rel·y_axis)`.
    #[must_use]
    pub fn to_sketch(&self, p: Vec3) -> Vec2 {
        let rx = p.x - self.origin.x;
        let ry = p.y - self.origin.y;
        let rz = p.z - self.origin.z;
        Vec2::new_unchecked(
            rx * self.x_axis.x + ry * self.x_axis.y + rz * self.x_axis.z,
            rx * self.y_axis.x + ry * self.y_axis.y + rz * self.y_axis.z,
        )
    }
}

/// Builds a sketch plane from a point on the plane and its normal, choosing the
/// in-plane axes by a **fixed, deterministic rule**.
///
/// This is the frame a sketch placed on a model FACE or a datum plane freezes at
/// creation (MODEL-OPS W2). Determinism is load-bearing, not cosmetic: the frame
/// is persisted with the sketch and every entity coordinate is expressed in it, so
/// if the rule ever produced a different basis for the same normal, reopening a
/// document would silently rotate the sketch inside its own plane. The rule is
/// therefore lock-tested exactly like the named-plane bases in SCHEMA §7.3.
///
/// **The rule.** `x = normalize(seed × n)` where the seed is world **+Z**, falling
/// back to world **+X** when `n` is (anti)parallel to +Z and the cross product
/// would vanish; then `y = n × x`. So:
/// * a face whose normal is +Z (a top face) uses the +X fallback and lands on
///   `x=(0,1,0)`, `y=(-1,0,0)` — the SAME basis as the named XY plane, so a sketch
///   on a flat top face is oriented like a sketch on XY;
/// * any other face gets a horizontal `x`, i.e. the sketch's local +X is level in
///   the world, which reads the right way up for a user.
///
/// `normal` is normalized here; a zero/non-finite normal falls back to XY rather
/// than producing NaNs.
#[must_use]
pub fn plane_from_point_normal(origin: Vec3, normal: Vec3) -> SketchPlane {
    let norm = |v: [f64; 3]| -> Option<[f64; 3]> {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if !len.is_finite() || len < 1e-12 {
            return None;
        }
        Some([v[0] / len, v[1] / len, v[2] / len])
    };
    let cross = |a: [f64; 3], b: [f64; 3]| -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };

    let Some(n) = norm([normal.x, normal.y, normal.z]) else {
        return SketchPlane::xy();
    };

    // Seed +Z (`x = z × n`); when the normal is (anti)parallel to +Z the cross
    // product vanishes, so fall back to the +X seed with the operands ORDERED
    // `n × x` — that ordering is what makes a +Z normal reproduce the named XY
    // basis exactly (`z × n` would give x = (0,-1,0), the XY basis mirrored).
    let z_seed = [0.0, 0.0, 1.0];
    let x = match norm(cross(z_seed, n)) {
        Some(v) => v,
        None => norm(cross(n, [1.0, 0.0, 0.0])).unwrap_or([1.0, 0.0, 0.0]),
    };
    let y = cross(n, x);

    SketchPlane {
        origin,
        x_axis: Vec3::new_unchecked(x[0], x[1], x[2]),
        y_axis: Vec3::new_unchecked(y[0], y[1], y[2]),
        normal: Vec3::new_unchecked(n[0], n[1], n[2]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// LOCK-TEST — the exact XY basis numbers. If this fails, someone
    /// "normalized" the coordinate system and broke every stored file.
    #[test]
    fn xy_basis_is_the_non_standard_cpp_basis() {
        const MSG: &str = "non-standard basis is load-bearing — see Sketch.h";
        let p = SketchPlane::xy();
        assert_eq!(
            [p.origin.x, p.origin.y, p.origin.z],
            [0.0, 0.0, 0.0],
            "{MSG}"
        );
        assert_eq!(
            [p.x_axis.x, p.x_axis.y, p.x_axis.z],
            [0.0, 1.0, 0.0],
            "{MSG}"
        );
        assert_eq!(
            [p.y_axis.x, p.y_axis.y, p.y_axis.z],
            [-1.0, 0.0, 0.0],
            "{MSG}"
        );
        assert_eq!(
            [p.normal.x, p.normal.y, p.normal.z],
            [0.0, 0.0, 1.0],
            "{MSG}"
        );
        // The exported constants must agree with the constructor.
        assert_eq!(
            [p.x_axis.x, p.x_axis.y, p.x_axis.z],
            XY_LOCAL_X_IN_WORLD,
            "{MSG}"
        );
        assert_eq!(
            [p.y_axis.x, p.y_axis.y, p.y_axis.z],
            XY_LOCAL_Y_IN_WORLD,
            "{MSG}"
        );
    }

    /// LOCK-TEST — XZ / YZ bases (verified against `Sketch.h`, not guessed).
    #[test]
    fn xz_and_yz_bases_match_sketch_h() {
        const MSG: &str = "non-standard basis is load-bearing — see Sketch.h";
        let xz = SketchPlane::xz();
        assert_eq!(
            [xz.x_axis.x, xz.x_axis.y, xz.x_axis.z],
            [0.0, 1.0, 0.0],
            "{MSG}"
        );
        assert_eq!(
            [xz.y_axis.x, xz.y_axis.y, xz.y_axis.z],
            [0.0, 0.0, 1.0],
            "{MSG}"
        );
        assert_eq!(
            [xz.normal.x, xz.normal.y, xz.normal.z],
            [1.0, 0.0, 0.0],
            "{MSG}"
        );

        let yz = SketchPlane::yz();
        assert_eq!(
            [yz.x_axis.x, yz.x_axis.y, yz.x_axis.z],
            [-1.0, 0.0, 0.0],
            "{MSG}"
        );
        assert_eq!(
            [yz.y_axis.x, yz.y_axis.y, yz.y_axis.z],
            [0.0, 0.0, 1.0],
            "{MSG}"
        );
        assert_eq!(
            [yz.normal.x, yz.normal.y, yz.normal.z],
            [0.0, 1.0, 0.0],
            "{MSG}"
        );
    }

    /// The XY mapping is exactly the C++ `toWorld` contract for the basis axes.
    #[test]
    fn xy_to_world_maps_axes_per_sketch_h() {
        let p = SketchPlane::xy();
        // Sketch X (+1,0) → World Y+ (0,1,0); Sketch Y (0,+1) → World X- (-1,0,0).
        assert_eq!(
            p.to_world(Vec2::new_unchecked(1.0, 0.0)),
            Vec3::new_unchecked(0.0, 1.0, 0.0)
        );
        assert_eq!(
            p.to_world(Vec2::new_unchecked(0.0, 1.0)),
            Vec3::new_unchecked(-1.0, 0.0, 0.0)
        );
    }

    proptest! {
        /// Round-trip: `to_sketch ∘ to_world == id` for every named plane and a
        /// custom translated plane (exercises the origin term).
        #[test]
        fn to_sketch_is_left_inverse_of_to_world(x in -1.0e6f64..1.0e6, y in -1.0e6f64..1.0e6) {
            let custom = SketchPlane {
                origin: Vec3::new_unchecked(10.0, -20.0, 30.0),
                ..SketchPlane::xy()
            };
            for plane in [SketchPlane::xy(), SketchPlane::xz(), SketchPlane::yz(), custom] {
                let p = Vec2::new_unchecked(x, y);
                let back = plane.to_sketch(plane.to_world(p));
                prop_assert!(back.approx_eq(&p, 1e-6), "round-trip drifted: {p:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn plane_serde_camel_case_and_ignores_unknown_kind() {
        let p = SketchPlane::xy();
        let v = serde_json::to_value(p).unwrap();
        assert_eq!(v["xAxis"], serde_json::json!([0.0, 1.0, 0.0]));
        assert_eq!(v["yAxis"], serde_json::json!([-1.0, 0.0, 0.0]));
        // A SCHEMA §7.3 `plane` object (with a `kind` discriminator) parses in,
        // ignoring `kind` (no deny_unknown_fields).
        let schema_plane = serde_json::json!({
            "kind": "XY", "origin": [0, 0, 0], "xAxis": [0, 1, 0],
            "yAxis": [-1, 0, 0], "normal": [0, 0, 1]
        });
        let back: SketchPlane = serde_json::from_value(schema_plane).unwrap();
        assert_eq!(back, SketchPlane::xy());
    }

    // ── plane_from_point_normal — LOCK TESTS ────────────────────────────────
    //
    // The basis is frozen with every face-hosted / datum sketch and every entity
    // coordinate is expressed in it, so a change to this rule silently rotates
    // existing sketches inside their own plane on reopen. Treat these exactly like
    // the named-plane basis locks above.

    #[test]
    fn face_normal_plus_z_matches_the_named_xy_basis() {
        // +Z is the seed direction, so the +X fallback fires. Landing on the SAME
        // basis as the named XY plane means a sketch on a flat top face is
        // oriented like a sketch on XY.
        let p = plane_from_point_normal(
            Vec3::new_unchecked(1.0, 2.0, 3.0),
            Vec3::new_unchecked(0.0, 0.0, 1.0),
        );
        assert_eq!(p.origin, Vec3::new_unchecked(1.0, 2.0, 3.0));
        assert_eq!(p.x_axis, SketchPlane::xy().x_axis);
        assert_eq!(p.y_axis, SketchPlane::xy().y_axis);
        assert_eq!(p.normal, Vec3::new_unchecked(0.0, 0.0, 1.0));
    }

    #[test]
    fn face_normal_minus_z_uses_the_same_fallback_seed() {
        let p = plane_from_point_normal(
            Vec3::new_unchecked(0.0, 0.0, 0.0),
            Vec3::new_unchecked(0.0, 0.0, -1.0),
        );
        // Same +X fallback, mirrored by the flipped normal:
        // x = normalize(-Z × +X) = (0,-1,0); y = -Z × (0,-1,0) = (-1,0,0).
        assert_eq!(p.x_axis, Vec3::new_unchecked(0.0, -1.0, 0.0));
        assert_eq!(p.y_axis, Vec3::new_unchecked(-1.0, 0.0, 0.0));
    }

    #[test]
    fn a_vertical_face_gets_a_level_x_axis() {
        // Normal +X (a side face): x = normalize(+Z × +X) = (0,1,0) — horizontal,
        // so the sketch's local +X is level in the world.
        let p = plane_from_point_normal(
            Vec3::new_unchecked(0.0, 0.0, 0.0),
            Vec3::new_unchecked(1.0, 0.0, 0.0),
        );
        assert_eq!(p.x_axis, Vec3::new_unchecked(0.0, 1.0, 0.0));
        assert_eq!(p.y_axis, Vec3::new_unchecked(0.0, 0.0, 1.0));
        assert!((p.x_axis.z).abs() < 1e-12, "x is level");
    }

    #[test]
    fn the_basis_is_orthonormal_and_right_handed_for_arbitrary_normals() {
        let normals = [
            [1.0, 2.0, 3.0],
            [-4.0, 0.5, 2.0],
            [0.0, 1.0, 0.0],
            [0.3, -0.7, 0.1],
            [0.0, 0.0, 7.0],
        ];
        for n in normals {
            let p = plane_from_point_normal(
                Vec3::new_unchecked(0.0, 0.0, 0.0),
                Vec3::new_unchecked(n[0], n[1], n[2]),
            );
            let dot = |a: Vec3, b: Vec3| a.x * b.x + a.y * b.y + a.z * b.z;
            let len = |v: Vec3| dot(v, v).sqrt();
            assert!((len(p.x_axis) - 1.0).abs() < 1e-12, "x unit for {n:?}");
            assert!((len(p.y_axis) - 1.0).abs() < 1e-12, "y unit for {n:?}");
            assert!((len(p.normal) - 1.0).abs() < 1e-12, "n unit for {n:?}");
            assert!(dot(p.x_axis, p.y_axis).abs() < 1e-12, "x ⟂ y for {n:?}");
            assert!(dot(p.x_axis, p.normal).abs() < 1e-12, "x ⟂ n for {n:?}");
            assert!(dot(p.y_axis, p.normal).abs() < 1e-12, "y ⟂ n for {n:?}");
            // Right-handed: x × y == n.
            let cx = Vec3::new_unchecked(
                p.x_axis.y * p.y_axis.z - p.x_axis.z * p.y_axis.y,
                p.x_axis.z * p.y_axis.x - p.x_axis.x * p.y_axis.z,
                p.x_axis.x * p.y_axis.y - p.x_axis.y * p.y_axis.x,
            );
            assert!((cx.x - p.normal.x).abs() < 1e-12, "right-handed for {n:?}");
            assert!((cx.y - p.normal.y).abs() < 1e-12, "right-handed for {n:?}");
            assert!((cx.z - p.normal.z).abs() < 1e-12, "right-handed for {n:?}");
        }
    }

    #[test]
    fn the_rule_is_deterministic_and_scale_invariant() {
        let o = Vec3::new_unchecked(0.0, 0.0, 0.0);
        let a = plane_from_point_normal(o, Vec3::new_unchecked(1.0, 2.0, 3.0));
        let b = plane_from_point_normal(o, Vec3::new_unchecked(1.0, 2.0, 3.0));
        // Identical input ⇒ BITWISE identical frame. This is the property replay
        // depends on, and it is exact.
        assert_eq!(a, b);

        // A scaled normal gives the same frame to floating-point tolerance — the
        // division by a different magnitude perturbs the last ulp, so this one is
        // deliberately NOT an equality assertion.
        let scaled = plane_from_point_normal(o, Vec3::new_unchecked(10.0, 20.0, 30.0));
        let close = |x: Vec3, y: Vec3| {
            (x.x - y.x).abs() < 1e-12 && (x.y - y.y).abs() < 1e-12 && (x.z - y.z).abs() < 1e-12
        };
        assert!(
            close(a.x_axis, scaled.x_axis),
            "x: {:?} vs {:?}",
            a.x_axis,
            scaled.x_axis
        );
        assert!(
            close(a.y_axis, scaled.y_axis),
            "y: {:?} vs {:?}",
            a.y_axis,
            scaled.y_axis
        );
        assert!(
            close(a.normal, scaled.normal),
            "n: {:?} vs {:?}",
            a.normal,
            scaled.normal
        );
    }

    #[test]
    fn a_degenerate_normal_falls_back_to_xy_rather_than_nan() {
        let p = plane_from_point_normal(
            Vec3::new_unchecked(0.0, 0.0, 0.0),
            Vec3::new_unchecked(0.0, 0.0, 0.0),
        );
        assert_eq!(p, SketchPlane::xy());
    }
}
