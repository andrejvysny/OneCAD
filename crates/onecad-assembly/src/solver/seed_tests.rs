//! Audit 4 / 26 and the numerical half of 3 (second review SR-06): supported
//! single-joint trees must converge outside the iteration's local basin.
//!
//! Every accepted result is re-checked with an independent oracle written
//! here from rotation matrices; it shares no code with the solver's residuals.
use uuid::Uuid;

use super::*;
use crate::joint::JointOrigin;
use crate::math::Frame;

type Mat = [[f64; 3]; 3];

fn matrix(q: Quat) -> Mat {
    let [w, x, y, z] = q.components();
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

fn mul(m: &Mat, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// A world attachment frame: origin and unit x/z axes of `frame` under `pose`.
struct World {
    origin: [f64; 3],
    x: [f64; 3],
    z: [f64; 3],
}

fn world(pose: &RigidTransform, frame: &Frame) -> World {
    let r = matrix(pose.rotation);
    let t = pose.translation;
    let at = |v: Vec3d| mul(&r, [v.x, v.y, v.z]);
    let o = at(frame.origin);
    World {
        origin: [o[0] + t.x, o[1] + t.y, o[2] + t.z],
        x: at(frame.x),
        z: at(frame.z),
    }
}

/// Largest violation of the joint's own definition, in mm or unit-vector norm.
fn independent_violation(kind: JointKind, a: &World, b: &World, driven: Option<f64>) -> f64 {
    let d = sub(b.origin, a.origin);
    let z_error = norm(sub(b.z, a.z));
    let x_error = norm(sub(b.x, a.x));
    let y_a = [
        a.z[1] * a.x[2] - a.z[2] * a.x[1],
        a.z[2] * a.x[0] - a.z[0] * a.x[2],
        a.z[0] * a.x[1] - a.z[1] * a.x[0],
    ];
    match kind {
        JointKind::Rigid => norm(d).max(z_error).max(x_error),
        JointKind::Revolute => {
            let angle = dot(b.x, y_a).atan2(dot(b.x, a.x));
            let drive = driven.map_or(0.0, |t| wrap_angle(angle - t).abs());
            norm(d).max(z_error).max(drive)
        }
        JointKind::Slider => {
            let lateral = dot(d, a.x).abs().max(dot(d, y_a).abs());
            let drive = driven.map_or(0.0, |t| (dot(d, a.z) - t).abs());
            lateral.max(z_error).max(x_error).max(drive)
        }
    }
}

fn occ(n: u128) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(n))
}

fn frame(origin: Vec3d, z: Vec3d) -> Frame {
    Frame {
        origin,
        z,
        x: Vec3d::X,
    }
}

fn joint(kind: JointKind, a: Frame, b: Frame) -> Joint {
    Joint {
        id: JointId(Uuid::from_u128(10)),
        name: "j".into(),
        kind,
        a: JointOrigin {
            occurrence: occ(1),
            frame: a,
        },
        b: JointOrigin {
            occurrence: occ(2),
            frame: b,
        },
        limits: None,
        value: 0.0,
        suppressed: false,
    }
}

fn request(joint: Joint, start: RigidTransform) -> SolveRequest {
    SolveRequest::new(
        vec![
            SolverBody {
                id: occ(1),
                pose: RigidTransform::IDENTITY,
                grounded: true,
            },
            SolverBody {
                id: occ(2),
                pose: start,
                grounded: false,
            },
        ],
        vec![joint],
    )
}

/// Solves and returns `(free body pose, remaining dof)` after proving the
/// ground did not move and the joint holds under the independent oracle.
fn solved(req: &SolveRequest, driven: Option<f64>) -> (RigidTransform, usize) {
    let outcome = solve(req);
    let SolveOutcome::Converged {
        poses,
        remaining_dof,
        ..
    } = outcome
    else {
        panic!("must converge: {outcome:?}");
    };
    assert_eq!(
        poses[0].1.rotation.components(),
        Quat::IDENTITY.components()
    );
    assert_eq!(poses[0].1.translation, Vec3d::ZERO, "ground is bit-exact");
    let j = &req.joints[0];
    let violation = independent_violation(
        j.kind,
        &world(&poses[0].1, &j.a.frame),
        &world(&poses[1].1, &j.b.frame),
        driven,
    );
    // 3e-9 aggregates the solver's 1e-9 per-component tolerance over a 3-vector.
    assert!(violation <= 3e-9, "independent violation {violation}");
    (poses[1].1, remaining_dof)
}

fn far_start(l: f64) -> RigidTransform {
    RigidTransform::new(
        Quat::from_axis_angle(Vec3d::Y, 0.4).unwrap(),
        Vec3d::new(3.0 * l, -2.0 * l, l),
    )
}

fn near_start(l: f64) -> RigidTransform {
    RigidTransform::new(
        Quat::from_axis_angle(Vec3d::Y, 5.0_f64.to_radians()).unwrap(),
        Vec3d::new(-l + 1.0, 1.0, l + 1.0),
    )
}

/// Attachment origins A=(0,0,L) on ground and B=(L,0,0) on the free body, axes
/// aligned: the exact answer is identity rotation at (−L, 0, L).
#[test]
fn lever_arm_sweep_converges_to_the_closed_form_pose() {
    for l in [1.0, 10.0, 100.0, 300.0, 1_000.0, 3_000.0, 10_000.0] {
        for start in [near_start(l), far_start(l)] {
            for (kind, driven) in [(JointKind::Rigid, None), (JointKind::Slider, Some(0.0))] {
                let mut req = request(
                    joint(
                        kind,
                        frame(Vec3d::new(0.0, 0.0, l), Vec3d::Z),
                        frame(Vec3d::new(l, 0.0, 0.0), Vec3d::Z),
                    ),
                    start,
                );
                if let Some(value) = driven {
                    req.drive.push((req.joints[0].id, value));
                }
                let (pose, dof) = solved(&req, driven);
                assert_eq!(dof, 0, "L={l} {kind:?}");
                let expected = Vec3d::new(-l, 0.0, l);
                let miss = (pose.translation - expected).norm();
                assert!(miss <= 1e-6, "L={l} {kind:?}: off by {miss} mm");
                let [w, ..] = pose.rotation.components();
                assert!(w.abs() > 1.0 - 1e-12, "L={l} {kind:?}: rotated");
            }
        }
    }
}

/// Exactly opposed joint axes are a stationary point of the rotation residual.
#[test]
fn opposed_axes_converge_from_the_stationary_start() {
    for kind in [JointKind::Rigid, JointKind::Revolute, JointKind::Slider] {
        for epsilon in [0.0, 1e-10] {
            let start = RigidTransform::new(
                Quat::from_axis_angle(Vec3d::X, epsilon).unwrap(),
                Vec3d::ZERO,
            );
            let req = request(
                joint(
                    kind,
                    frame(Vec3d::new(0.0, 0.0, 10.0), Vec3d::Z),
                    frame(Vec3d::ZERO, Vec3d::new(0.0, 0.0, -1.0)),
                ),
                start,
            );
            let (_, dof) = solved(&req, None);
            assert_eq!(dof, usize::from(kind != JointKind::Rigid), "{kind:?}");
        }
    }
}

/// The retry must not change a solve the iteration already handles.
#[test]
fn a_start_inside_the_basin_keeps_its_direct_result() {
    let req = request(
        joint(
            JointKind::Revolute,
            frame(Vec3d::new(0.0, 0.0, 5.0), Vec3d::Z),
            frame(Vec3d::new(2.0, 0.0, 0.0), Vec3d::Z),
        ),
        near_start(5.0),
    );
    let cons = build_constraints(&req).unwrap();
    let layout = build_layout(&req.bodies);
    let problem = Problem {
        cons: &cons,
        layout: &layout,
        tol: req.tolerance,
        max_iter: req.max_iterations,
    };
    let start: Vec<_> = req.bodies.iter().map(|b| b.pose).collect();
    let direct = run_lm(&start, &problem);
    assert!(direct.status == LmStatus::Converged);
    let wrapped = solve_lm(&start, &problem);
    assert_eq!(wrapped.iterations, direct.iterations);
    for (a, b) in wrapped.poses.iter().zip(&direct.poses) {
        assert_eq!(a.rotation.components(), b.rotation.components());
        assert_eq!(a.translation, b.translation);
    }
}

/// A free joint is seeded at the coordinate it already has; a limit only
/// moves it to the nearest bound. The guess never pins or drives it.
#[test]
fn free_coordinates_are_kept_and_limits_respected_by_the_guess() {
    let mut j = joint(
        JointKind::Slider,
        frame(Vec3d::new(0.0, 0.0, 2_000.0), Vec3d::Z),
        frame(Vec3d::new(2_000.0, 0.0, 0.0), Vec3d::Z),
    );
    j.limits = Some(JointLimits {
        min: -5.0,
        max: 5.0,
    });
    // Exact pose for slider value 40 mm, pushed far off in rotation.
    let start = RigidTransform::new(
        Quat::from_axis_angle(Vec3d::Y, 0.4).unwrap(),
        Vec3d::new(-2_000.0, 0.0, 2_040.0),
    );
    let (pose, dof) = solved(&request(j, start), None);
    assert_eq!(dof, 1, "the slider stays free");
    let travel = pose.translation.z - 2_000.0;
    assert!((-5.0 - 1e-9..=5.0 + 1e-9).contains(&travel), "{travel}");
}
