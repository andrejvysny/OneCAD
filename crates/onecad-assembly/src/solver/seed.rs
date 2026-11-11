//! Closed-form initial guess for joint graphs: propagate poses outward from
//! placed bodies, one joint at a time.
//!
//! For a joint between a placed body `P` and an unplaced body `Q` the relation
//! `T_b · l_b = T_a · l_a · J(q)` fixes `Q` exactly once the joint coordinate
//! `q` is chosen (`J` is a rotation about, or translation along, the joint z).
//! On a tree every body is reached through exactly one joint, so the result
//! satisfies every constraint and the iterative solver only has to verify it.
//!
//! This is a GUESS, never an answer: the caller still runs the full residual,
//! limit and rank checks on whatever it converges to. A free joint keeps the
//! coordinate its current pose already has (clamped into its limits); nothing
//! is pinned and no constraint is added.
use super::*;

/// The joint transform `J(q)` in the joint frame.
fn joint_motion(kind: JointKind, value: f64) -> Option<RigidTransform> {
    Some(match kind {
        JointKind::Rigid => RigidTransform::new(Quat::IDENTITY, Vec3d::ZERO),
        JointKind::Revolute => {
            RigidTransform::new(Quat::from_axis_angle(Vec3d::Z, value)?, Vec3d::ZERO)
        }
        JointKind::Slider => RigidTransform::new(Quat::IDENTITY, Vec3d::new(0.0, 0.0, value)),
    })
}

/// The coordinate a free joint currently has, measured without assuming the
/// joint is already satisfied: the twist of the relative rotation about z for
/// a revolute (well defined even for opposed axes), the z offset for a slider.
fn current_coordinate(poses: &[RigidTransform], c: &Constraint) -> f64 {
    let (fa, fb) = joint_frames(poses, c);
    match c.kind {
        JointKind::Rigid => 0.0,
        JointKind::Revolute => {
            let [w, _, _, z] = (fa.rotation.conjugate() * fb.rotation).components();
            if w == 0.0 && z == 0.0 {
                0.0
            } else {
                wrap_angle(2.0 * z.atan2(w))
            }
        }
        JointKind::Slider => (fb.translation - fa.translation).dot(fa.rotation.rotate(Vec3d::Z)),
    }
}

fn seed_coordinate(poses: &[RigidTransform], c: &Constraint) -> f64 {
    if let Some(target) = c.target {
        return target;
    }
    let value = current_coordinate(poses, c);
    match c.limits {
        Some(limits) if value.is_finite() => nearest_limit(value, c.kind, limits).0,
        _ => value,
    }
}

/// Places the unplaced side of `c` from its placed side. `None` when the
/// arithmetic leaves the rigid domain.
fn place(poses: &[RigidTransform], c: &Constraint, forward: bool) -> Option<RigidTransform> {
    let motion = joint_motion(c.kind, seed_coordinate(poses, c))?;
    let pose = if forward {
        poses[c.a]
            .compose(&c.la)
            .compose(&motion)
            .compose(&c.lb.inverse())
    } else {
        poses[c.b]
            .compose(&c.lb)
            .compose(&motion.inverse())
            .compose(&c.la.inverse())
    };
    let rotation = pose.rotation.normalized()?;
    let pose = RigidTransform::new(rotation, pose.translation);
    pose.is_rigid().then_some(pose)
}

/// Poses with every body reachable from a placed one re-seated on its joint.
/// Grounded bodies are never moved. A component without ground is rooted at
/// its first body in input order, which keeps its start pose.
pub(super) fn tree_seed(start: &[RigidTransform], problem: &Problem) -> Vec<RigidTransform> {
    let mut poses = copy_poses(start);
    let mut placed: Vec<bool> = problem.layout.offsets.iter().map(Option::is_none).collect();
    let mut used = vec![false; problem.cons.len()];
    loop {
        let mut progressed = propagate(&mut poses, &mut placed, &mut used, problem.cons);
        if !progressed {
            // Root the next floating component at its lowest-index body.
            let root = problem
                .cons
                .iter()
                .zip(&used)
                .filter(|(_, used)| !**used)
                .flat_map(|(c, _)| [c.a, c.b])
                .filter(|body| !placed[*body])
                .min();
            if let Some(root) = root {
                placed[root] = true;
                progressed = true;
            }
        }
        if !progressed {
            return poses;
        }
    }
}

/// One sweep in joint input order; returns whether any body was placed.
fn propagate(
    poses: &mut [RigidTransform],
    placed: &mut [bool],
    used: &mut [bool],
    cons: &[Constraint],
) -> bool {
    let mut progressed = false;
    for (index, c) in cons.iter().enumerate() {
        if used[index] || placed[c.a] == placed[c.b] {
            // Both placed closes a loop the guess cannot honour; it is left
            // for the iterative solve. Neither placed waits for a later sweep.
            used[index] |= placed[c.a] && placed[c.b];
            continue;
        }
        let forward = placed[c.a];
        let moving = if forward { c.b } else { c.a };
        if let Some(pose) = place(poses, c, forward) {
            poses[moving] = pose;
        }
        placed[moving] = true;
        used[index] = true;
        progressed = true;
    }
    progressed
}
