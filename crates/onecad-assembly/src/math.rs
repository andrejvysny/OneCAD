//! Pure f64 rigid-transform math for assemblies: vectors, unit quaternions,
//! rigid transforms and orthonormal frames.

use std::ops::{Add, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

/// Below this a vector or quaternion norm counts as degenerate.
const NORM_EPS: f64 = 1e-12;
/// Below this rotation angle the exp/log maps use Taylor expansions.
const SMALL_ANGLE: f64 = 1e-8;

/// 3D vector / point in f64.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Vec3d {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3d {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    #[must_use]
    pub fn norm_squared(self) -> f64 {
        self.dot(self)
    }

    #[must_use]
    pub fn norm(self) -> f64 {
        self.norm_squared().sqrt()
    }

    /// Unit vector, or `None` if the norm is below 1e-12 or non-finite.
    #[must_use]
    pub fn normalized(self) -> Option<Self> {
        let n = self.norm();
        if !n.is_finite() || n < NORM_EPS {
            return None;
        }
        Some(self * (1.0 / n))
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3d {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3d {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for Vec3d {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f64> for Vec3d {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

/// Unit quaternion (Hamilton convention, `w` scalar part).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "QuatRepr", into = "QuatRepr")]
pub struct Quat {
    pub(crate) w: f64,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuatRepr {
    w: f64,
    x: f64,
    y: f64,
    z: f64,
}

impl From<Quat> for QuatRepr {
    fn from(q: Quat) -> Self {
        Self {
            w: q.w,
            x: q.x,
            y: q.y,
            z: q.z,
        }
    }
}

impl TryFrom<QuatRepr> for Quat {
    type Error = &'static str;
    fn try_from(q: QuatRepr) -> Result<Self, Self::Error> {
        Self::try_new(q.w, q.x, q.y, q.z).ok_or("quaternion must be a finite unit rotation")
    }
}

impl Quat {
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Accepts only unit quaternions within 1e-12 norm-squared roundoff.
    #[must_use]
    pub fn try_new(w: f64, x: f64, y: f64, z: f64) -> Option<Self> {
        let q = Self { w, x, y, z };
        q.is_unit().then_some(q)
    }

    #[must_use]
    pub fn components(self) -> [f64; 4] {
        [self.w, self.x, self.y, self.z]
    }

    fn vector_part(self) -> Vec3d {
        Vec3d::new(self.x, self.y, self.z)
    }

    /// Rotation of `angle_rad` about `axis`; `None` if the axis is degenerate.
    #[must_use]
    pub fn from_axis_angle(axis: Vec3d, angle_rad: f64) -> Option<Self> {
        let a = axis.normalized()?;
        if !angle_rad.is_finite() {
            return None;
        }
        let half = 0.5 * angle_rad;
        let s = half.sin();
        Some(Self {
            w: half.cos(),
            x: a.x * s,
            y: a.y * s,
            z: a.z * s,
        })
    }

    /// Exponential map: rotation vector (axis * angle) to quaternion.
    #[must_use]
    pub fn from_rotation_vector(v: Vec3d) -> Option<Self> {
        let angle = v.norm();
        if !angle.is_finite() {
            return None;
        }
        let (w, k) = if angle < SMALL_ANGLE {
            let a2 = angle * angle;
            (1.0 - a2 / 8.0, 0.5 - a2 / 48.0)
        } else {
            let half = 0.5 * angle;
            (half.cos(), half.sin() / angle)
        };
        Self::try_new(w, v.x * k, v.y * k, v.z * k)
    }

    /// Log map, shortest arc (angle in [0, pi]).
    #[must_use]
    pub fn to_rotation_vector(self) -> Vec3d {
        let q = if self.w < 0.0 { self.negated() } else { self };
        let u = q.vector_part();
        let s = u.norm();
        if s < SMALL_ANGLE {
            // angle ~ 2 * s / w for small s; w is ~1 here.
            return u * (2.0 / q.w.max(NORM_EPS));
        }
        let angle = 2.0 * s.atan2(q.w);
        u * (angle / s)
    }

    fn negated(self) -> Self {
        Self {
            w: -self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// Hamilton product `self * rhs` (`rhs` applied first when rotating).
    #[must_use]
    #[allow(clippy::should_implement_trait)] // `Mul` is also implemented below.
    pub fn mul(self, rhs: Self) -> Self {
        Self {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        }
        .normalized()
        .expect("product of unit quaternions is finite and nonzero")
    }

    #[must_use]
    pub fn conjugate(self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// Unit-length copy, or `None` if the norm is below 1e-12 or non-finite.
    #[must_use]
    pub fn normalized(self) -> Option<Self> {
        let n = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if !n.is_finite() || n < NORM_EPS {
            return None;
        }
        let k = 1.0 / n;
        Some(Self {
            w: self.w * k,
            x: self.x * k,
            y: self.y * k,
            z: self.z * k,
        })
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.w.is_finite() && self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// True for finite unit rotations, allowing 1e-12 norm-squared roundoff.
    #[must_use]
    pub fn is_unit(self) -> bool {
        let n2 = self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z;
        n2.is_finite() && (n2 - 1.0).abs() <= 1e-12
    }

    /// Rotates `v`: `v + 2w (u x v) + 2 u x (u x v)` with `u` the vector part.
    #[must_use]
    pub fn rotate(self, v: Vec3d) -> Vec3d {
        let u = self.vector_part();
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Row-major 3x3 rotation matrix.
    fn to_matrix(self) -> [[f64; 3]; 3] {
        let Self { w, x, y, z } = self;
        [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - w * z),
                2.0 * (x * z + w * y),
            ],
            [
                2.0 * (x * y + w * z),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - w * x),
            ],
            [
                2.0 * (x * z - w * y),
                2.0 * (y * z + w * x),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ]
    }

    /// Shepperd's method; `m` is a row-major proper rotation matrix.
    fn from_matrix(m: &[[f64; 3]; 3]) -> Option<Self> {
        let t = m[0][0] + m[1][1] + m[2][2];
        let q = if t > 0.0 {
            let s = (t + 1.0).sqrt() * 2.0;
            Self {
                w: 0.25 * s,
                x: (m[2][1] - m[1][2]) / s,
                y: (m[0][2] - m[2][0]) / s,
                z: (m[1][0] - m[0][1]) / s,
            }
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            Self {
                w: (m[2][1] - m[1][2]) / s,
                x: 0.25 * s,
                y: (m[0][1] + m[1][0]) / s,
                z: (m[0][2] + m[2][0]) / s,
            }
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            Self {
                w: (m[0][2] - m[2][0]) / s,
                x: (m[0][1] + m[1][0]) / s,
                y: 0.25 * s,
                z: (m[1][2] + m[2][1]) / s,
            }
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            Self {
                w: (m[1][0] - m[0][1]) / s,
                x: (m[0][2] + m[2][0]) / s,
                y: (m[1][2] + m[2][1]) / s,
                z: 0.25 * s,
            }
        };
        q.normalized()
    }
}

impl Mul for Quat {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Quat::mul(self, rhs)
    }
}

/// Rigid transform: rotate, then translate (`p' = R p + t`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RigidTransform {
    pub rotation: Quat,
    pub translation: Vec3d,
}

impl RigidTransform {
    pub const IDENTITY: Self = Self {
        rotation: Quat::IDENTITY,
        translation: Vec3d::ZERO,
    };

    #[must_use]
    pub const fn new(rotation: Quat, translation: Vec3d) -> Self {
        Self {
            rotation,
            translation,
        }
    }

    /// `self ∘ child`: `child` is applied first, then `self`.
    #[must_use]
    pub fn compose(&self, child: &Self) -> Self {
        Self {
            rotation: self.rotation.mul(child.rotation),
            translation: self.rotation.rotate(child.translation) + self.translation,
        }
    }

    #[must_use]
    pub fn inverse(&self) -> Self {
        let r = self.rotation.conjugate();
        Self {
            rotation: r,
            translation: -r.rotate(self.translation),
        }
    }

    #[must_use]
    pub fn apply_point(&self, p: Vec3d) -> Vec3d {
        self.rotation.rotate(p) + self.translation
    }

    /// Rotation only (directions ignore translation).
    #[must_use]
    pub fn apply_vector(&self, v: Vec3d) -> Vec3d {
        self.rotation.rotate(v)
    }

    /// Column-major 4x4 matrix (last column holds the translation).
    #[must_use]
    pub fn to_cols_array(&self) -> [f64; 16] {
        let r = self.rotation.to_matrix();
        let t = self.translation;
        [
            r[0][0], r[1][0], r[2][0], 0.0, //
            r[0][1], r[1][1], r[2][1], 0.0, //
            r[0][2], r[1][2], r[2][2], 0.0, //
            t.x, t.y, t.z, 1.0,
        ]
    }

    /// Translation distance <= `linear_eps` and relative rotation angle <=
    /// `angular_eps_rad`.
    #[must_use]
    pub fn approx_eq(&self, other: &Self, linear_eps: f64, angular_eps_rad: f64) -> bool {
        if (self.translation - other.translation).norm() > linear_eps {
            return false;
        }
        let rel = self.rotation.conjugate().mul(other.rotation);
        // |w| folds q and -q (same rotation) together.
        let angle = 2.0 * rel.vector_part().norm().atan2(rel.w.abs());
        angle <= angular_eps_rad
    }

    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.rotation.is_finite() && self.translation.is_finite()
    }

    #[must_use]
    pub fn is_rigid(&self) -> bool {
        self.rotation.is_unit() && self.translation.is_finite()
    }
}

/// Orthonormal frame given by origin, z axis and x axis; `y = z × x` is
/// derived, never stored (same convention as `MateFrame` in onecad-core).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frame {
    pub origin: Vec3d,
    pub z: Vec3d,
    pub x: Vec3d,
}

impl Frame {
    /// Transform mapping the local frame axes onto this frame's axes
    /// (local origin to `origin`). `z` is normalized and `x` is
    /// Gram-Schmidt orthogonalized against it. `None` if degenerate/parallel.
    #[must_use]
    pub fn to_transform(&self) -> Option<RigidTransform> {
        if !self.origin.is_finite() {
            return None;
        }
        let z = self.z.normalized()?;
        let xn = self.x.normalized()?;
        let x = (xn - z * xn.dot(z)).normalized()?;
        let y = z.cross(x);
        let m = [[x.x, y.x, z.x], [x.y, y.y, z.y], [x.z, y.z, z.z]];
        let rotation = Quat::from_matrix(&m)?;
        Some(RigidTransform::new(rotation, self.origin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    const EPS: f64 = 1e-12;

    fn close(a: Vec3d, b: Vec3d, eps: f64) -> bool {
        (a - b).norm() <= eps
    }

    #[test]
    fn rotate_90_about_z_is_exact() {
        let q = Quat::from_axis_angle(Vec3d::Z, FRAC_PI_2).unwrap();
        assert!(close(q.rotate(Vec3d::X), Vec3d::Y, EPS));
        assert!(close(q.rotate(Vec3d::Y), -Vec3d::X, EPS));
        assert!(close(q.rotate(Vec3d::Z), Vec3d::Z, EPS));
    }

    #[test]
    fn rotation_vector_round_trip() {
        for v in [
            Vec3d::new(0.3, -0.2, 0.5),
            Vec3d::new(1e-10, 0.0, 0.0),
            Vec3d::new(0.0, 0.0, 0.0),
            Vec3d::new(2.0, 1.0, -1.0),
        ] {
            let back = Quat::from_rotation_vector(v).unwrap().to_rotation_vector();
            assert!(close(back, v, EPS), "{v:?} -> {back:?}");
        }
    }

    #[test]
    fn log_map_takes_shortest_arc() {
        let q = Quat::from_rotation_vector(Vec3d::new(0.0, 0.0, 4.0)).unwrap();
        let v = q.to_rotation_vector();
        assert!(close(v, Vec3d::new(0.0, 0.0, 4.0 - 2.0 * PI), 1e-9));
        let neg = Quat {
            w: -q.w,
            x: -q.x,
            y: -q.y,
            z: -q.z,
        };
        assert!(close(neg.to_rotation_vector(), v, 1e-9));
    }

    #[test]
    fn quat_mul_matches_sequential_rotation() {
        let a = Quat::from_axis_angle(Vec3d::X, 0.7).unwrap();
        let b = Quat::from_axis_angle(Vec3d::new(1.0, 2.0, 3.0), -1.1).unwrap();
        let v = Vec3d::new(0.4, -1.5, 2.0);
        assert!(close((a * b).rotate(v), a.rotate(b.rotate(v)), EPS));
        let id = a.mul(a.conjugate());
        assert!((id.w - 1.0).abs() < EPS && id.vector_part().norm() < EPS);
    }

    #[test]
    fn normalized_rejects_degenerate() {
        assert!(Vec3d::ZERO.normalized().is_none());
        assert!(Vec3d::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        let zero = Quat {
            w: 0.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(zero.normalized().is_none());
        assert!(Quat::from_axis_angle(Vec3d::ZERO, 1.0).is_none());
    }

    #[test]
    fn compose_inverse_is_identity() {
        let t = RigidTransform::new(
            Quat::from_axis_angle(Vec3d::new(1.0, 1.0, 0.0), 0.9).unwrap(),
            Vec3d::new(1.0, -2.0, 3.0),
        );
        let id = t.compose(&t.inverse());
        assert!(id.approx_eq(&RigidTransform::IDENTITY, 1e-12, 1e-12));
        let id2 = t.inverse().compose(&t);
        assert!(id2.approx_eq(&RigidTransform::IDENTITY, 1e-12, 1e-12));
    }

    #[test]
    fn compose_applies_child_first() {
        let parent = RigidTransform::new(
            Quat::from_axis_angle(Vec3d::Z, FRAC_PI_2).unwrap(),
            Vec3d::new(10.0, 0.0, 0.0),
        );
        let child = RigidTransform::new(Quat::IDENTITY, Vec3d::new(1.0, 0.0, 0.0));
        let both = parent.compose(&child);
        let p = Vec3d::new(0.0, 0.0, 0.0);
        assert!(close(
            both.apply_point(p),
            parent.apply_point(child.apply_point(p)),
            EPS
        ));
        assert!(close(both.apply_point(p), Vec3d::new(10.0, 1.0, 0.0), EPS));
        assert!(close(parent.apply_vector(Vec3d::X), Vec3d::Y, EPS));
    }

    #[test]
    fn approx_eq_handles_sign_and_tolerance() {
        let a = RigidTransform::IDENTITY;
        let mut b = a;
        b.rotation = Quat {
            w: -1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(a.approx_eq(&b, 1e-9, 1e-9));
        let c = RigidTransform::new(Quat::from_axis_angle(Vec3d::X, 0.01).unwrap(), Vec3d::ZERO);
        assert!(!a.approx_eq(&c, 1e-9, 1e-3));
        assert!(a.approx_eq(&c, 1e-9, 0.02));
        let d = RigidTransform::new(Quat::IDENTITY, Vec3d::new(0.0, 0.5, 0.0));
        assert!(!a.approx_eq(&d, 0.1, 1.0));
    }

    #[test]
    fn cols_array_is_column_major() {
        let t = RigidTransform::new(
            Quat::from_axis_angle(Vec3d::Z, FRAC_PI_2).unwrap(),
            Vec3d::new(1.0, 2.0, 3.0),
        );
        let m = t.to_cols_array();
        // Column 0 is the image of X: (0, 1, 0, 0).
        assert!((m[0]).abs() < EPS && (m[1] - 1.0).abs() < EPS && m[2].abs() < EPS);
        assert_eq!(&m[12..], &[1.0, 2.0, 3.0, 1.0]);
        assert_eq!(m[3], 0.0);
    }

    #[test]
    fn frame_maps_local_axes_and_origin() {
        let f = Frame {
            origin: Vec3d::new(1.0, 2.0, 3.0),
            z: Vec3d::Y,
            x: Vec3d::X,
        };
        let t = f.to_transform().unwrap();
        assert!(close(t.apply_point(Vec3d::ZERO), f.origin, EPS));
        assert!(close(t.apply_vector(Vec3d::X), Vec3d::X, EPS));
        assert!(close(t.apply_vector(Vec3d::Z), Vec3d::Y, EPS));
        // y = z x x = Y x X = -Z.
        assert!(close(t.apply_vector(Vec3d::Y), -Vec3d::Z, EPS));
    }

    #[test]
    fn frame_orthonormalizes_skewed_x() {
        let f = Frame {
            origin: Vec3d::ZERO,
            z: Vec3d::new(0.0, 0.0, 5.0),
            x: Vec3d::new(2.0, 0.0, 1.0),
        };
        let t = f.to_transform().unwrap();
        assert!(close(t.apply_vector(Vec3d::X), Vec3d::X, EPS));
        assert!(close(t.apply_vector(Vec3d::Z), Vec3d::Z, EPS));
    }

    #[test]
    fn frame_covers_all_shepperd_branches() {
        let axes = [
            (Vec3d::Z, Vec3d::X),
            (Vec3d::Z, -Vec3d::X),
            (Vec3d::Y, -Vec3d::X),
            (-Vec3d::Z, Vec3d::Y),
            (Vec3d::X, Vec3d::Y),
        ];
        for (z, x) in axes {
            let t = Frame {
                origin: Vec3d::ZERO,
                z,
                x,
            }
            .to_transform()
            .unwrap();
            assert!(close(t.apply_vector(Vec3d::Z), z, EPS));
            assert!(close(t.apply_vector(Vec3d::X), x, EPS));
            assert!(close(t.apply_vector(Vec3d::Y), z.cross(x), EPS));
        }
    }

    #[test]
    fn frame_degenerate_is_none() {
        let o = Vec3d::ZERO;
        assert!(Frame {
            origin: o,
            z: Vec3d::ZERO,
            x: Vec3d::X
        }
        .to_transform()
        .is_none());
        assert!(Frame {
            origin: o,
            z: Vec3d::Z,
            x: Vec3d::ZERO
        }
        .to_transform()
        .is_none());
        assert!(Frame {
            origin: o,
            z: Vec3d::Z,
            x: Vec3d::Z
        }
        .to_transform()
        .is_none());
        assert!(Frame {
            origin: o,
            z: Vec3d::Z,
            x: -Vec3d::Z
        }
        .to_transform()
        .is_none());
    }

    #[test]
    fn serde_round_trip_camel_case() {
        let t = RigidTransform::new(
            Quat::from_axis_angle(Vec3d::Y, 0.3).unwrap(),
            Vec3d::new(1.0, 2.0, 3.0),
        );
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"rotation\"") && json.contains("\"translation\""));
        let back: RigidTransform = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
    #[test]
    fn checked_quaternions_preserve_rigid_geometry() {
        for w in [0.0, 1e-100, 1e100, 2.0, f64::NAN, f64::INFINITY] {
            assert!(Quat::try_new(w, 0.0, 0.0, 0.0).is_none());
        }
        for json in [
            r#"{"w":0,"x":0,"y":0,"z":0}"#,
            r#"{"w":2,"x":0,"y":0,"z":0}"#,
        ] {
            assert!(serde_json::from_str::<Quat>(json).is_err());
        }
        assert!(Quat::from_rotation_vector(Vec3d::new(f64::INFINITY, 0.0, 0.0)).is_none());
        let q = Quat::from_axis_angle(Vec3d::new(1.0, 2.0, 3.0), 1.2).unwrap();
        let v = Vec3d::new(3.0, -2.0, 5.0);
        assert!((q.rotate(v).norm() - v.norm()).abs() < EPS);
        let a = q.rotate(Vec3d::X);
        let b = q.rotate(Vec3d::Y);
        let c = q.rotate(Vec3d::Z);
        assert!((a.dot(b.cross(c)) - 1.0).abs() < EPS);
        assert!((q.conjugate().rotate(q.rotate(v)) - v).norm() < EPS);
        assert!((q.negated().rotate(v) - q.rotate(v)).norm() < EPS);
    }
}
