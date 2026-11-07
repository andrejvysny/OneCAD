//! Occurrence-instance table (proposed ADR 0021, instanced assembly geometry).
//!
//! Models are rigid f64 transforms; ray math runs in the mesh's local frame.
//! Caps are provisional; application projection into these instances is separate.

use crate::{BodyId, Bounds, DMat4, DVec3, Ray};
use glam::DMat3;

/// Provisional cap on instances per table (ADR 0021).
pub const MAX_INSTANCES: usize = 2000;
/// Provisional cap on occurrence path depth (ADR 0021).
pub const MAX_DEPTH: usize = 8;
const RIGID_TOL: f64 = 1e-9;

/// Opaque occurrence identity; deliberately not the assembly crate's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OccurrenceKey(pub u128);

/// Root-to-leaf occurrence chain.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OccurrencePath(pub Vec<OccurrenceKey>);

#[derive(Clone, Debug)]
pub struct SceneInstance {
    pub mesh: BodyId,
    /// Local-to-world rigid transform.
    pub model: DMat4,
    pub path: OccurrencePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InstanceError {
    #[error("instance table full")]
    TooManyInstances,
    #[error("occurrence path deeper than the cap")]
    PathTooDeep,
    #[error("instance model contains non-finite values")]
    NonFinite,
    #[error("instance model is not a proper rigid transform")]
    NotRigid,
}

#[derive(Debug, Clone, Default)]
pub struct InstanceTable {
    instances: Vec<SceneInstance>,
}

impl InstanceTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn instances(&self) -> &[SceneInstance] {
        &self.instances
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn push(&mut self, instance: SceneInstance) -> Result<(), InstanceError> {
        if self.instances.len() >= MAX_INSTANCES {
            return Err(InstanceError::TooManyInstances);
        }
        if instance.path.0.len() > MAX_DEPTH {
            return Err(InstanceError::PathTooDeep);
        }
        check_rigid(&instance.model)?;
        self.instances.push(instance);
        Ok(())
    }

    /// Broadphase only: instances whose world-space oriented bounds the ray
    /// enters, as `(index, t_enter)` sorted by `t_enter` then index.
    pub fn instance_pick_candidates(
        &self,
        ray: Ray,
        bounds_of: impl Fn(BodyId) -> Option<Bounds>,
    ) -> Vec<(usize, f64)> {
        let mut out: Vec<(usize, f64)> = self
            .instances
            .iter()
            .enumerate()
            .filter_map(|(i, inst)| {
                let b = bounds_of(inst.mesh)?;
                let (o, d) = ray_to_local(ray.origin, ray.direction, &inst.model);
                slab_enter(o, d, b).map(|t| (i, t))
            })
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }
}

pub(crate) fn check_rigid(m: &DMat4) -> Result<(), InstanceError> {
    if !m.to_cols_array().iter().all(|v| v.is_finite()) {
        return Err(InstanceError::NonFinite);
    }
    let bottom = m.row(3);
    if (bottom - glam::DVec4::new(0.0, 0.0, 0.0, 1.0))
        .abs()
        .max_element()
        > RIGID_TOL
    {
        return Err(InstanceError::NotRigid);
    }
    let r = DMat3::from_mat4(*m);
    let gram = r.transpose() * r;
    let dev = (gram - DMat3::IDENTITY).to_cols_array();
    if dev.iter().any(|v| v.abs() > RIGID_TOL) || r.determinant() <= 0.0 {
        return Err(InstanceError::NotRigid);
    }
    Ok(())
}

/// Axis-aligned bounds of a local box under `model` (Arvo's method), exact
/// for any affine transform.
pub fn transformed_aabb(local_min: DVec3, local_max: DVec3, model: &DMat4) -> (DVec3, DVec3) {
    let t = model.w_axis.truncate();
    let (mut lo, mut hi) = (t, t);
    let cols = [model.x_axis, model.y_axis, model.z_axis];
    for (axis, col) in cols.iter().enumerate() {
        let c = col.truncate();
        let a = c * local_min[axis];
        let b = c * local_max[axis];
        lo += a.min(b);
        hi += a.max(b);
    }
    (lo, hi)
}

/// Rigid inverse of `model` (rotation transposed, translation back-rotated).
fn rigid_inverse(model: &DMat4) -> DMat4 {
    let rt = DMat3::from_mat4(*model).transpose();
    let t = -(rt * model.w_axis.truncate());
    DMat4::from_cols(
        rt.x_axis.extend(0.0),
        rt.y_axis.extend(0.0),
        rt.z_axis.extend(0.0),
        t.extend(1.0),
    )
}

/// World ray to the mesh's local frame; direction length is preserved so `t`
/// is the same parameter in both frames.
pub fn ray_to_local(origin: DVec3, dir: DVec3, model: &DMat4) -> (DVec3, DVec3) {
    let inv = rigid_inverse(model);
    (inv.transform_point3(origin), inv.transform_vector3(dir))
}

pub fn hit_to_world(local_point: DVec3, model: &DMat4) -> DVec3 {
    model.transform_point3(local_point)
}

/// Slab test returning the entry parameter (clamped to >= 0), or `None`.
fn slab_enter(origin: DVec3, dir: DVec3, b: Bounds) -> Option<f64> {
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for axis in 0..3 {
        if dir[axis] == 0.0 {
            if origin[axis] < b.min[axis] || origin[axis] > b.max[axis] {
                return None;
            }
        } else {
            let a = (b.min[axis] - origin[axis]) / dir[axis];
            let c = (b.max[axis] - origin[axis]) / dir[axis];
            near = near.max(a.min(c));
            far = far.min(a.max(c));
            if near > far {
                return None;
            }
        }
    }
    Some(near)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn inst(model: DMat4, depth: usize) -> SceneInstance {
        SceneInstance {
            mesh: BodyId::new(),
            model,
            path: OccurrencePath((0..depth as u128).map(OccurrenceKey).collect()),
        }
    }

    fn unit_bounds() -> Bounds {
        Bounds {
            min: DVec3::ZERO,
            max: DVec3::ONE,
        }
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        (a - b).abs().max_element() < 1e-12
    }

    #[test]
    fn identity_round_trip() {
        let m = DMat4::IDENTITY;
        let (o, d) = ray_to_local(DVec3::new(1.0, 2.0, 3.0), DVec3::Z, &m);
        assert!(close(o, DVec3::new(1.0, 2.0, 3.0)) && close(d, DVec3::Z));
        let mut t = InstanceTable::new();
        assert!(t.push(inst(m, 1)).is_ok());
    }

    #[test]
    fn rotation_translation_round_trip() {
        let m =
            DMat4::from_translation(DVec3::new(5.0, -2.0, 1.0)) * DMat4::from_rotation_z(FRAC_PI_2);
        let p = DVec3::new(0.3, 0.7, 0.2);
        let w = hit_to_world(p, &m);
        assert!(close(w, DVec3::new(4.3, -1.7, 1.2)));
        let (back, _) = ray_to_local(w, DVec3::X, &m);
        assert!(close(back, p));
    }

    #[test]
    fn refuses_reflection_and_bad_models() {
        let mut t = InstanceTable::new();
        let mirror = DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0));
        assert_eq!(t.push(inst(mirror, 0)), Err(InstanceError::NotRigid));
        let scaled = DMat4::from_scale(DVec3::splat(2.0));
        assert_eq!(t.push(inst(scaled, 0)), Err(InstanceError::NotRigid));
        let nan = DMat4::from_translation(DVec3::new(f64::NAN, 0.0, 0.0));
        assert_eq!(t.push(inst(nan, 0)), Err(InstanceError::NonFinite));
        assert!(t.is_empty());
    }

    #[test]
    fn refuses_depth_and_cap() {
        let mut t = InstanceTable::new();
        assert!(t.push(inst(DMat4::IDENTITY, MAX_DEPTH)).is_ok());
        assert_eq!(
            t.push(inst(DMat4::IDENTITY, MAX_DEPTH + 1)),
            Err(InstanceError::PathTooDeep)
        );
        for _ in 1..MAX_INSTANCES {
            assert!(t.push(inst(DMat4::IDENTITY, 0)).is_ok());
        }
        assert_eq!(
            t.push(inst(DMat4::IDENTITY, 0)),
            Err(InstanceError::TooManyInstances)
        );
    }

    #[test]
    fn aabb_of_rotated_cube_is_exact() {
        let m =
            DMat4::from_translation(DVec3::new(10.0, 0.0, 0.0)) * DMat4::from_rotation_z(FRAC_PI_2);
        let (lo, hi) = transformed_aabb(DVec3::ZERO, DVec3::ONE, &m);
        assert!(close(lo, DVec3::new(9.0, 0.0, 0.0)));
        assert!(close(hi, DVec3::new(10.0, 1.0, 1.0)));
    }

    #[test]
    fn candidates_sorted_by_t_then_index() {
        let mut t = InstanceTable::new();
        let far = DMat4::from_translation(DVec3::new(0.0, 0.0, 10.0));
        let near = DMat4::from_translation(DVec3::new(0.0, 0.0, 2.0));
        let miss = DMat4::from_translation(DVec3::new(50.0, 0.0, 0.0));
        for m in [far, near, miss, near] {
            assert!(t.push(inst(m, 0)).is_ok());
        }
        let ray = Ray {
            origin: DVec3::new(0.5, 0.5, -5.0),
            direction: DVec3::Z,
        };
        let c = t.instance_pick_candidates(ray, |_| Some(unit_bounds()));
        let order: Vec<usize> = c.iter().map(|x| x.0).collect();
        assert_eq!(order, vec![1, 3, 0]);
        assert!((c[0].1 - 7.0).abs() < 1e-12);
        assert!(t.instance_pick_candidates(ray, |_| None).is_empty());
    }
}
