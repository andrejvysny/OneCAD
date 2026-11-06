//! Deterministic rigid-body joint solver (Levenberg-Marquardt) with DOF report.
//!
//! Unknowns are a 6-vector per non-grounded body: a left-perturbation
//! `pose' = (exp(dθ), dt) ∘ pose`. Bodies and joints are visited in input order;
//! no hash iteration, so identical input gives bit-identical output.
//!
//! The Jacobian uses central finite differences. At ≤120 unknowns (20
//! occurrences) the cost is trivial and it cannot disagree with the residuals;
//! analytic derivatives are a later optimisation.
//!
//! Non-convergence never partially moves the assembly: failure outcomes carry
//! no poses, so the caller keeps its input poses (ONECAD-401).

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::joint::{Joint, JointError, JointId, JointKind, JointLimits};
use crate::math::{Quat, RigidTransform, Vec3d};
use crate::occurrence::OccurrenceId;

const FD_STEP: f64 = 1e-7;
const RANK_REL_TOL: f64 = 1e-8;
const LAMBDA_INIT: f64 = 1e-3;
const LAMBDA_MIN: f64 = 1e-12;
const LAMBDA_MAX: f64 = 1e12;
const GRAD_STALL: f64 = 1e-12;
const STEP_STALL: f64 = 1e-13;

fn default_max_iterations() -> u32 {
    50
}

fn default_tolerance() -> f64 {
    1e-9
}

/// A body handed to the solver; `pose` is occurrence-to-world.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolverBody {
    pub id: OccurrenceId,
    pub pose: RigidTransform,
    pub grounded: bool,
}

/// Solve input. `drive` pins a joint's value (rad/mm); the last entry per joint wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveRequest {
    pub bodies: Vec<SolverBody>,
    pub joints: Vec<Joint>,
    #[serde(default)]
    pub drive: Vec<(JointId, f64)>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
}

impl SolveRequest {
    #[must_use]
    pub fn new(bodies: Vec<SolverBody>, joints: Vec<Joint>) -> Self {
        Self {
            bodies,
            joints,
            drive: Vec::new(),
            max_iterations: default_max_iterations(),
            tolerance: default_tolerance(),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum SolverError {
    #[error("joint {joint:?} invalid: {source}")]
    InvalidJoint {
        joint: JointId,
        #[source]
        source: JointError,
    },
    #[error("joint {0:?} references an occurrence that is not a solver body")]
    UnknownOccurrence(JointId),
    #[error("duplicate solver body {0:?}")]
    DuplicateBody(OccurrenceId),
    #[error("body {0:?} has a non-finite pose")]
    NonFinitePose(OccurrenceId),
    #[error("drive references unknown joint {0:?}")]
    UnknownDriveJoint(JointId),
    #[error("rigid joint {0:?} cannot be driven")]
    DriveOnRigid(JointId),
    #[error("drive value for joint {0:?} is non-finite or outside its limits")]
    BadDriveValue(JointId),
    #[error("tolerance must be finite and positive, max_iterations nonzero")]
    BadSettings,
}

#[derive(Debug, Clone)]
pub enum SolveOutcome {
    Converged {
        poses: Vec<(OccurrenceId, RigidTransform)>,
        iterations: u32,
        max_residual: f64,
        remaining_dof: usize,
        redundant_constraints: usize,
    },
    NotConverged {
        max_residual: f64,
        iterations: u32,
    },
    /// Residual stalled above tolerance (no descent step exists): contradictory joints.
    Inconsistent {
        max_residual: f64,
    },
    Invalid(SolverError),
}

/// One active (non-suppressed) joint with frames resolved to local transforms.
struct Constraint {
    kind: JointKind,
    a: usize,
    b: usize,
    la: RigidTransform,
    lb: RigidTransform,
    /// Fixed joint value (drive, or limit clamp); `None` = free.
    target: Option<f64>,
    limits: Option<JointLimits>,
}

struct Layout {
    /// Start offset of each body's 6 unknowns, `None` when grounded.
    offsets: Vec<Option<usize>>,
    unknowns: usize,
}

struct Problem<'a> {
    cons: &'a [Constraint],
    layout: &'a Layout,
    tol: f64,
    max_iter: u32,
}

#[derive(PartialEq, Eq)]
enum LmStatus {
    Converged,
    Exhausted,
    Stalled,
}

struct LmOutcome {
    poses: Vec<RigidTransform>,
    iterations: u32,
    max_residual: f64,
    status: LmStatus,
}

/// Solve the assembly. Pure and deterministic.
#[must_use]
pub fn solve(req: &SolveRequest) -> SolveOutcome {
    match solve_inner(req) {
        Ok(outcome) => outcome,
        Err(err) => SolveOutcome::Invalid(err),
    }
}

fn solve_inner(req: &SolveRequest) -> Result<SolveOutcome, SolverError> {
    if !(req.tolerance.is_finite() && req.tolerance > 0.0) || req.max_iterations == 0 {
        return Err(SolverError::BadSettings);
    }
    check_bodies(&req.bodies)?;
    let mut cons = build_constraints(req)?;
    let layout = build_layout(&req.bodies);
    let start: Vec<RigidTransform> = req.bodies.iter().map(|b| copy_pose(&b.pose)).collect();

    let (tol, max_iter) = (req.tolerance, req.max_iterations);
    let problem = Problem {
        cons: &cons,
        layout: &layout,
        tol,
        max_iter,
    };
    if layout.unknowns == 0 {
        return Ok(solve_fixed(req, &problem, &start));
    }
    let first = run_lm(&start, &problem);
    let clamped =
        first.status == LmStatus::Converged && apply_limit_clamps(&mut cons, &first.poses, tol);
    let problem = Problem {
        cons: &cons,
        layout: &layout,
        tol,
        max_iter,
    };
    if !clamped {
        let iterations = first.iterations;
        return Ok(finish(req, &problem, first, iterations));
    }
    let second = run_lm(&first.poses, &problem);
    let iterations = first.iterations + second.iterations;
    Ok(finish(req, &problem, second, iterations))
}

fn check_bodies(bodies: &[SolverBody]) -> Result<(), SolverError> {
    for (i, body) in bodies.iter().enumerate() {
        if bodies[..i].iter().any(|prev| prev.id == body.id) {
            return Err(SolverError::DuplicateBody(body.id));
        }
        if !body.pose.is_finite() {
            return Err(SolverError::NonFinitePose(body.id));
        }
    }
    Ok(())
}

fn body_index(bodies: &[SolverBody], id: OccurrenceId) -> Option<usize> {
    bodies.iter().position(|b| b.id == id)
}

fn build_constraints(req: &SolveRequest) -> Result<Vec<Constraint>, SolverError> {
    check_drives(req)?;
    let mut cons = Vec::new();
    for joint in req.joints.iter().filter(|j| !j.suppressed) {
        let invalid = |source| SolverError::InvalidJoint {
            joint: joint.id,
            source,
        };
        joint.validate().map_err(invalid)?;
        let (Some(a), Some(b)) = (
            body_index(&req.bodies, joint.a.occurrence),
            body_index(&req.bodies, joint.b.occurrence),
        ) else {
            return Err(SolverError::UnknownOccurrence(joint.id));
        };
        let (Some(la), Some(lb)) = (joint.a.frame.to_transform(), joint.b.frame.to_transform())
        else {
            return Err(invalid(JointError::DegenerateFrame { side: "a/b" }));
        };
        let target = req
            .drive
            .iter()
            .rev()
            .find(|(id, _)| *id == joint.id)
            .map(|(_, v)| *v);
        cons.push(Constraint {
            kind: joint.kind,
            a,
            b,
            la,
            lb,
            target,
            limits: joint.limits,
        });
    }
    Ok(cons)
}

fn check_drives(req: &SolveRequest) -> Result<(), SolverError> {
    for (id, value) in &req.drive {
        let joint = req
            .joints
            .iter()
            .find(|j| j.id == *id)
            .ok_or(SolverError::UnknownDriveJoint(*id))?;
        if joint.kind == JointKind::Rigid {
            return Err(SolverError::DriveOnRigid(*id));
        }
        let in_limits = !joint.limits.is_some_and(|l| !l.contains(*value));
        if !value.is_finite() || !in_limits {
            return Err(SolverError::BadDriveValue(*id));
        }
    }
    Ok(())
}

fn build_layout(bodies: &[SolverBody]) -> Layout {
    let mut unknowns = 0;
    let offsets = bodies
        .iter()
        .map(|b| {
            if b.grounded {
                None
            } else {
                let at = unknowns;
                unknowns += 6;
                Some(at)
            }
        })
        .collect();
    Layout { offsets, unknowns }
}

fn copy_pose(p: &RigidTransform) -> RigidTransform {
    RigidTransform::new(p.rotation, p.translation)
}

fn copy_poses(poses: &[RigidTransform]) -> Vec<RigidTransform> {
    poses.iter().map(copy_pose).collect()
}

/// All bodies grounded: nothing can move, so only report the residual.
fn solve_fixed(req: &SolveRequest, problem: &Problem, start: &[RigidTransform]) -> SolveOutcome {
    let r = residuals(start, problem.cons);
    let max_residual = max_abs(&r);
    if max_residual > problem.tol {
        return SolveOutcome::Inconsistent { max_residual };
    }
    SolveOutcome::Converged {
        poses: pose_pairs(&req.bodies, start),
        iterations: 0,
        max_residual,
        remaining_dof: 0,
        redundant_constraints: r.len(),
    }
}

fn pose_pairs(
    bodies: &[SolverBody],
    poses: &[RigidTransform],
) -> Vec<(OccurrenceId, RigidTransform)> {
    bodies
        .iter()
        .zip(poses)
        .map(|(b, p)| (b.id, copy_pose(p)))
        .collect()
}

fn finish(req: &SolveRequest, problem: &Problem, fin: LmOutcome, iterations: u32) -> SolveOutcome {
    match fin.status {
        LmStatus::Converged => {
            let (remaining_dof, redundant_constraints) = dof_report(&fin.poses, problem);
            SolveOutcome::Converged {
                poses: pose_pairs(&req.bodies, &fin.poses),
                iterations,
                max_residual: fin.max_residual,
                remaining_dof,
                redundant_constraints,
            }
        }
        LmStatus::Exhausted => SolveOutcome::NotConverged {
            max_residual: fin.max_residual,
            iterations,
        },
        LmStatus::Stalled => SolveOutcome::Inconsistent {
            max_residual: fin.max_residual,
        },
    }
}

/// Pin free joints whose converged value left their limits. Returns true if any changed.
fn apply_limit_clamps(cons: &mut [Constraint], poses: &[RigidTransform], tol: f64) -> bool {
    let mut changed = false;
    for c in cons.iter_mut() {
        let (None, Some(limits)) = (c.target, c.limits) else {
            continue;
        };
        let value = joint_value(poses, c);
        if value < limits.min - tol {
            c.target = Some(limits.min);
            changed = true;
        } else if value > limits.max + tol {
            c.target = Some(limits.max);
            changed = true;
        }
    }
    changed
}

fn dof_report(poses: &[RigidTransform], problem: &Problem) -> (usize, usize) {
    let r = residuals(poses, problem.cons);
    let (m, n) = (r.len(), problem.layout.unknowns);
    let jac = jacobian(poses, problem, m);
    let rank = numerical_rank(jac, m, n);
    (n - rank, m - rank)
}

// ---------------------------------------------------------------- residuals

fn joint_frames(poses: &[RigidTransform], c: &Constraint) -> (RigidTransform, RigidTransform) {
    (poses[c.a].compose(&c.la), poses[c.b].compose(&c.lb))
}

fn wrap_angle(x: f64) -> f64 {
    x - TAU * (x / TAU).round()
}

/// Rotation of frame b about a's z axis, valid when the z axes are aligned.
fn relative_angle(ra: Quat, rb: Quat) -> f64 {
    let xb_in_a = (ra.conjugate() * rb).rotate(Vec3d::X);
    xb_in_a.y.atan2(xb_in_a.x)
}

fn relative_rotation_vector(ra: Quat, rb: Quat) -> Vec3d {
    (ra.conjugate() * rb).to_rotation_vector()
}

fn push_vec3(out: &mut Vec<f64>, v: Vec3d) {
    out.extend([v.x, v.y, v.z]);
}

/// Current value of a joint (rad or mm); 0 for rigid.
fn joint_value(poses: &[RigidTransform], c: &Constraint) -> f64 {
    let (fa, fb) = joint_frames(poses, c);
    match c.kind {
        JointKind::Rigid => 0.0,
        JointKind::Revolute => relative_angle(fa.rotation, fb.rotation),
        JointKind::Slider => (fb.translation - fa.translation).dot(fa.rotation.rotate(Vec3d::Z)),
    }
}

fn push_joint_residuals(poses: &[RigidTransform], c: &Constraint, out: &mut Vec<f64>) {
    let (fa, fb) = joint_frames(poses, c);
    let (ra, rb) = (fa.rotation, fb.rotation);
    let d = fb.translation - fa.translation;
    let (xa, ya, za) = (
        ra.rotate(Vec3d::X),
        ra.rotate(Vec3d::Y),
        ra.rotate(Vec3d::Z),
    );
    match c.kind {
        JointKind::Rigid => {
            push_vec3(out, d);
            push_vec3(out, relative_rotation_vector(ra, rb));
        }
        JointKind::Revolute => {
            push_vec3(out, d);
            let tilt = za.cross(rb.rotate(Vec3d::Z));
            out.push(tilt.dot(xa));
            out.push(tilt.dot(ya));
            if let Some(t) = c.target {
                out.push(wrap_angle(relative_angle(ra, rb) - t));
            }
        }
        JointKind::Slider => {
            out.push(d.dot(xa));
            out.push(d.dot(ya));
            push_vec3(out, relative_rotation_vector(ra, rb));
            if let Some(t) = c.target {
                out.push(d.dot(za) - t);
            }
        }
    }
}

fn residuals(poses: &[RigidTransform], cons: &[Constraint]) -> Vec<f64> {
    let mut out = Vec::new();
    for c in cons {
        push_joint_residuals(poses, c, &mut out);
    }
    out
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0, |acc, x| {
        if x.abs() > acc || x.is_nan() {
            x.abs()
        } else {
            acc
        }
    })
}

fn sum_squares(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

// ------------------------------------------------------------------ stepping

/// Apply the increment `dx` (6 per free body) as a left perturbation.
fn apply_step(poses: &[RigidTransform], layout: &Layout, dx: &[f64]) -> Vec<RigidTransform> {
    poses
        .iter()
        .zip(&layout.offsets)
        .map(|(pose, offset)| match offset {
            None => copy_pose(pose),
            Some(o) => {
                let dtheta = Vec3d::new(dx[*o], dx[o + 1], dx[o + 2]);
                let dt = Vec3d::new(dx[o + 3], dx[o + 4], dx[o + 5]);
                let moved =
                    RigidTransform::new(Quat::from_rotation_vector(dtheta), dt).compose(pose);
                let q = moved.rotation.normalized().unwrap_or(moved.rotation);
                RigidTransform::new(q, moved.translation)
            }
        })
        .collect()
}

/// Row-major `m x n` central-difference Jacobian of the residuals.
fn jacobian(poses: &[RigidTransform], problem: &Problem, m: usize) -> Vec<f64> {
    let n = problem.layout.unknowns;
    let mut jac = vec![0.0; m * n];
    let mut dx = vec![0.0; n];
    for col in 0..n {
        dx[col] = FD_STEP;
        let plus = residuals(&apply_step(poses, problem.layout, &dx), problem.cons);
        dx[col] = -FD_STEP;
        let minus = residuals(&apply_step(poses, problem.layout, &dx), problem.cons);
        dx[col] = 0.0;
        for row in 0..m {
            jac[row * n + col] = (plus[row] - minus[row]) / (2.0 * FD_STEP);
        }
    }
    jac
}

/// `(JᵀJ, Jᵀr)` for row-major `J` of size `m x n`.
fn normal_equations(jac: &[f64], m: usize, n: usize, r: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut a = vec![0.0; n * n];
    let mut g = vec![0.0; n];
    for row in 0..m {
        let jr = &jac[row * n..(row + 1) * n];
        for i in 0..n {
            g[i] += jr[i] * r[row];
            for k in 0..n {
                a[i * n + k] += jr[i] * jr[k];
            }
        }
    }
    (a, g)
}

/// Solve `(A + λ diag(A)) dx = -g`; `None` if singular or non-finite.
fn lm_step(a: &[f64], g: &[f64], n: usize, lambda: f64) -> Option<Vec<f64>> {
    let mut damped = a.to_vec();
    for i in 0..n {
        damped[i * n + i] += lambda * a[i * n + i].max(1e-9);
    }
    let rhs: Vec<f64> = g.iter().map(|v| -v).collect();
    solve_linear(damped, rhs, n).filter(|dx| dx.iter().all(|v| v.is_finite()))
}

struct Trial {
    poses: Vec<RigidTransform>,
    residual: Vec<f64>,
}

/// Raise λ until a step lowers the cost; `None` when no descent step exists.
fn try_step(
    poses: &[RigidTransform],
    r: &[f64],
    normal: (&[f64], &[f64]),
    problem: &Problem,
    lambda: &mut f64,
) -> Option<Trial> {
    let n = problem.layout.unknowns;
    let cost = sum_squares(r);
    while *lambda <= LAMBDA_MAX {
        if let Some(dx) = lm_step(normal.0, normal.1, n, *lambda) {
            if max_abs(&dx) < STEP_STALL {
                return None;
            }
            let trial = apply_step(poses, problem.layout, &dx);
            let residual = residuals(&trial, problem.cons);
            if sum_squares(&residual) < cost {
                *lambda = (*lambda / 10.0).max(LAMBDA_MIN);
                return Some(Trial {
                    poses: trial,
                    residual,
                });
            }
        }
        *lambda *= 10.0;
    }
    None
}

fn run_lm(start: &[RigidTransform], problem: &Problem) -> LmOutcome {
    let n = problem.layout.unknowns;
    let mut poses = copy_poses(start);
    let mut r = residuals(&poses, problem.cons);
    let mut lambda = LAMBDA_INIT;
    let mut iterations = 0;
    while iterations < problem.max_iter {
        let max_residual = max_abs(&r);
        if max_residual <= problem.tol {
            return LmOutcome {
                poses,
                iterations,
                max_residual,
                status: LmStatus::Converged,
            };
        }
        let jac = jacobian(&poses, problem, r.len());
        let (a, g) = normal_equations(&jac, r.len(), n, &r);
        let stalled = LmOutcome {
            poses: Vec::new(),
            iterations,
            max_residual,
            status: LmStatus::Stalled,
        };
        if max_abs(&g) < GRAD_STALL {
            return stalled;
        }
        let Some(trial) = try_step(&poses, &r, (&a, &g), problem, &mut lambda) else {
            return stalled;
        };
        poses = trial.poses;
        r = trial.residual;
        iterations += 1;
    }
    let max_residual = max_abs(&r);
    let status = if max_residual <= problem.tol {
        LmStatus::Converged
    } else {
        LmStatus::Exhausted
    };
    LmOutcome {
        poses,
        iterations,
        max_residual,
        status,
    }
}

// ------------------------------------------------------------------- linalg

/// Gaussian elimination with partial pivoting on a row-major `n x n` system.
fn solve_linear(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let piv =
            (col..n).max_by(|&i, &k| a[i * n + col].abs().total_cmp(&a[k * n + col].abs()))?;
        let pivot = a[piv * n + col];
        // NaN must count as singular, hence the explicit is_nan check.
        if pivot.is_nan() || pivot.abs() <= 1e-300 {
            return None;
        }
        if piv != col {
            for k in 0..n {
                a.swap(piv * n + k, col * n + k);
            }
            b.swap(piv, col);
        }
        for row in col + 1..n {
            let f = a[row * n + col] / a[col * n + col];
            for k in col..n {
                a[row * n + k] -= f * a[col * n + k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let tail: f64 = (row + 1..n).map(|k| a[row * n + k] * x[k]).sum();
        x[row] = (b[row] - tail) / a[row * n + row];
    }
    Some(x)
}

/// Rank of a row-major `m x n` matrix by elimination with complete pivoting.
///
/// The threshold is relative to the largest entry so finite-difference noise
/// (≈ eps·|r|/h) on large-coordinate scenes is not counted as rank.
fn numerical_rank(mut j: Vec<f64>, m: usize, n: usize) -> usize {
    let scale = max_abs(&j);
    if scale == 0.0 || !scale.is_finite() {
        return 0;
    }
    let tol = RANK_REL_TOL * scale;
    let mut rank = 0;
    while rank < m.min(n) {
        let (mut pr, mut pc, mut best) = (rank, rank, 0.0);
        for r in rank..m {
            for c in rank..n {
                let v = j[r * n + c].abs();
                if v > best {
                    (pr, pc, best) = (r, c, v);
                }
            }
        }
        if best <= tol {
            break;
        }
        for c in 0..n {
            j.swap(pr * n + c, rank * n + c);
        }
        for r in 0..m {
            j.swap(r * n + pc, r * n + rank);
        }
        for r in rank + 1..m {
            let f = j[r * n + rank] / j[rank * n + rank];
            for c in rank..n {
                j[r * n + c] -= f * j[rank * n + c];
            }
        }
        rank += 1;
    }
    rank
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use uuid::Uuid;

    use super::*;
    use crate::joint::JointOrigin;
    use crate::math::Frame;

    fn occ(n: u128) -> OccurrenceId {
        OccurrenceId(Uuid::from_u128(n))
    }

    fn frame_at(origin: Vec3d) -> Frame {
        Frame {
            origin,
            z: Vec3d::Z,
            x: Vec3d::X,
        }
    }

    fn pose(rot: Quat, t: Vec3d) -> RigidTransform {
        RigidTransform::new(rot, t)
    }

    fn rot_z(angle: f64) -> Quat {
        Quat::from_axis_angle(Vec3d::Z, angle).unwrap()
    }

    fn body(n: u128, pose: RigidTransform, grounded: bool) -> SolverBody {
        SolverBody {
            id: occ(n),
            pose,
            grounded,
        }
    }

    fn joint(n: u128, kind: JointKind, fa: Frame, fb: Frame, a: u128, b: u128) -> Joint {
        Joint {
            id: JointId(Uuid::from_u128(n)),
            name: format!("j{n}"),
            kind,
            a: JointOrigin {
                occurrence: occ(a),
                frame: fa,
            },
            b: JointOrigin {
                occurrence: occ(b),
                frame: fb,
            },
            limits: None,
            value: 0.0,
            suppressed: false,
        }
    }

    fn two_bodies(b_pose: RigidTransform) -> Vec<SolverBody> {
        vec![
            body(1, RigidTransform::IDENTITY, true),
            body(2, b_pose, false),
        ]
    }

    fn converged_pose(out: &SolveOutcome, n: u128) -> RigidTransform {
        let SolveOutcome::Converged { poses, .. } = out else {
            panic!("expected Converged, got {out:?}");
        };
        let (_, p) = poses.iter().find(|(id, _)| *id == occ(n)).unwrap();
        copy_pose(p)
    }

    #[test]
    fn rigid_joint_pulls_b_onto_a() {
        let fa = frame_at(Vec3d::new(1.0, 0.0, 0.0));
        let j = joint(10, JointKind::Rigid, fa, frame_at(Vec3d::ZERO), 1, 2);
        let a_pose = pose(Quat::IDENTITY, Vec3d::new(10.0, 0.0, 0.0));
        let bodies = vec![
            body(1, copy_pose(&a_pose), true),
            body(2, pose(rot_z(0.4), Vec3d::new(0.0, 5.0, 7.0)), false),
        ];
        let out = solve(&SolveRequest::new(bodies, vec![j]));
        let expected = pose(Quat::IDENTITY, Vec3d::new(11.0, 0.0, 0.0));
        assert!(converged_pose(&out, 2).approx_eq(&expected, 1e-7, 1e-7));
        assert!(converged_pose(&out, 1).approx_eq(&a_pose, 1e-12, 1e-12));
        let SolveOutcome::Converged { remaining_dof, .. } = out else {
            panic!()
        };
        assert_eq!(remaining_dof, 0);
    }

    #[test]
    fn revolute_driven_to_quarter_turn() {
        let j = joint(
            10,
            JointKind::Revolute,
            frame_at(Vec3d::ZERO),
            frame_at(Vec3d::ZERO),
            1,
            2,
        );
        let mut req = SolveRequest::new(
            two_bodies(pose(rot_z(0.3), Vec3d::new(5.0, 3.0, 2.0))),
            vec![j],
        );
        req.drive.push((JointId(Uuid::from_u128(10)), FRAC_PI_2));
        let out = solve(&req);
        let expected = pose(rot_z(FRAC_PI_2), Vec3d::ZERO);
        assert!(converged_pose(&out, 2).approx_eq(&expected, 1e-7, 1e-7));
    }

    #[test]
    fn slider_driven_ten_mm() {
        let j = joint(
            10,
            JointKind::Slider,
            frame_at(Vec3d::ZERO),
            frame_at(Vec3d::ZERO),
            1,
            2,
        );
        let mut req = SolveRequest::new(
            two_bodies(pose(rot_z(0.2), Vec3d::new(1.0, 2.0, 3.0))),
            vec![j],
        );
        req.drive.push((JointId(Uuid::from_u128(10)), 10.0));
        let out = solve(&req);
        let expected = pose(Quat::IDENTITY, Vec3d::new(0.0, 0.0, 10.0));
        assert!(converged_pose(&out, 2).approx_eq(&expected, 1e-7, 1e-7));
    }

    #[test]
    fn free_revolute_reports_one_dof() {
        let j = joint(
            10,
            JointKind::Revolute,
            frame_at(Vec3d::ZERO),
            frame_at(Vec3d::ZERO),
            1,
            2,
        );
        let req = SolveRequest::new(
            two_bodies(pose(rot_z(0.1), Vec3d::new(1.0, 0.5, -0.5))),
            vec![j],
        );
        let SolveOutcome::Converged {
            remaining_dof,
            redundant_constraints,
            ..
        } = solve(&req)
        else {
            panic!("expected Converged");
        };
        assert_eq!(remaining_dof, 1);
        assert_eq!(redundant_constraints, 0);
    }

    #[test]
    fn slider_limit_clamps_free_value() {
        let mut j = joint(
            10,
            JointKind::Slider,
            frame_at(Vec3d::ZERO),
            frame_at(Vec3d::ZERO),
            1,
            2,
        );
        j.limits = Some(JointLimits { min: 0.0, max: 5.0 });
        let req = SolveRequest::new(
            two_bodies(pose(Quat::IDENTITY, Vec3d::new(0.0, 0.0, 8.0))),
            vec![j],
        );
        let out = solve(&req);
        let expected = pose(Quat::IDENTITY, Vec3d::new(0.0, 0.0, 5.0));
        assert!(converged_pose(&out, 2).approx_eq(&expected, 1e-7, 1e-7));
    }

    #[test]
    fn contradictory_rigid_joints_fail_without_poses() {
        let id = frame_at(Vec3d::ZERO);
        let j1 = joint(10, JointKind::Rigid, frame_at(Vec3d::ZERO), id, 1, 2);
        let j2 = joint(
            11,
            JointKind::Rigid,
            frame_at(Vec3d::ZERO),
            frame_at(Vec3d::new(5.0, 0.0, 0.0)),
            1,
            2,
        );
        let out = solve(&SolveRequest::new(
            two_bodies(RigidTransform::IDENTITY),
            vec![j1, j2],
        ));
        assert!(
            matches!(
                out,
                SolveOutcome::Inconsistent { .. } | SolveOutcome::NotConverged { .. }
            ),
            "got {out:?}"
        );
    }

    #[test]
    fn unknown_occurrence_is_invalid() {
        let j = joint(
            10,
            JointKind::Rigid,
            frame_at(Vec3d::ZERO),
            frame_at(Vec3d::ZERO),
            1,
            99,
        );
        let out = solve(&SolveRequest::new(
            two_bodies(RigidTransform::IDENTITY),
            vec![j],
        ));
        assert!(
            matches!(
                out,
                SolveOutcome::Invalid(SolverError::UnknownOccurrence(_))
            ),
            "got {out:?}"
        );
    }
}
