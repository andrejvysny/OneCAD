//! Pure solver timing only. Build with `cargo build --release -p onecad-assembly
//! --example measure_grounded_tree`, then run the executable with stdout captured.
//! Timings exclude validation and are not native or end-to-end latency.

use std::time::Instant;

use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::OccurrenceId,
    solver::{solve, SolveOutcome, SolveRequest, SolverBody},
};
use serde_json::{json, Value};
use uuid::Uuid;

const WARMUPS: usize = 5;
const SAMPLES: usize = 100;
const POSE_TOL_MM: f64 = 1e-5;
const ROT_TOL: f64 = 1e-7;

struct Case {
    name: &'static str,
    request: SolveRequest,
    expected: Vec3d,
    dof: usize,
}

fn occ(n: usize) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(n as u128 + 1))
}

fn case(name: &'static str, origin: Vec3d, scale: f64, slider: bool, driven: bool) -> Case {
    let initial = if driven {
        origin + Vec3d::Z * 0.9
    } else {
        origin
    };
    let bodies = (0..20)
        .map(|i| SolverBody {
            id: occ(i),
            pose: RigidTransform::new(Quat::IDENTITY, if i == 0 { origin } else { initial }),
            grounded: i == 0,
        })
        .collect();
    let frame = Frame {
        origin: Vec3d::X * scale,
        x: Vec3d::X,
        z: Vec3d::Z,
    };
    let joints = (0..19)
        .map(|i| Joint {
            id: JointId(Uuid::from_u128(100 + i as u128)),
            name: format!("joint-{i}"),
            kind: if slider && i == 0 {
                JointKind::Slider
            } else {
                JointKind::Rigid
            },
            a: JointOrigin {
                occurrence: occ(i),
                frame,
            },
            b: JointOrigin {
                occurrence: occ(i + 1),
                frame,
            },
            limits: (slider && i == 0).then_some(JointLimits {
                min: -2.0,
                max: 2.0,
            }),
            value: 0.0,
            suppressed: false,
        })
        .collect();
    let mut request = SolveRequest::new(bodies, joints);
    if driven {
        request.drive.push((JointId(Uuid::from_u128(100)), 1.0));
    }
    Case {
        name,
        request,
        expected: if driven { origin + Vec3d::Z } else { origin },
        dof: usize::from(slider && !driven),
    }
}

fn verify(case: &Case, outcome: SolveOutcome) -> &'static str {
    match outcome {
        SolveOutcome::Converged {
            poses,
            remaining_dof,
            max_residual,
            ..
        } => {
            assert_eq!(remaining_dof, case.dof, "{}: DOF", case.name);
            assert_eq!(poses.len(), 20);
            assert!(max_residual <= case.request.tolerance);
            for (i, (id, pose)) in poses.iter().enumerate() {
                assert_eq!(*id, occ(i));
                let expected = if i == 0 {
                    case.request.bodies[0].pose.translation
                } else {
                    case.expected
                };
                assert!(
                    (pose.translation - expected).norm() <= POSE_TOL_MM,
                    "{}: pose {i}",
                    case.name
                );
                for axis in [Vec3d::X, Vec3d::Y, Vec3d::Z] {
                    assert!(
                        (pose.rotation.rotate(axis) - axis).norm() <= ROT_TOL,
                        "{}: rotation {i}",
                        case.name
                    );
                }
            }
            if case.request.joints[0].kind == JointKind::Slider {
                let displacement = (poses[1].1.translation - poses[0].1.translation).dot(Vec3d::Z);
                let limits = case.request.joints[0].limits.unwrap();
                assert!(
                    displacement >= limits.min - POSE_TOL_MM
                        && displacement <= limits.max + POSE_TOL_MM
                );
                if !case.request.drive.is_empty() {
                    assert!((displacement - 1.0).abs() <= POSE_TOL_MM);
                }
            }
            "converged_verified"
        }
        SolveOutcome::NumericallyUncertain { .. } => "numerically_uncertain",
        SolveOutcome::NotConverged { .. } => "not_converged",
        SolveOutcome::Inconsistent { .. } => "inconsistent",
        SolveOutcome::Invalid(_) => "invalid",
    }
}

fn measure(case: &Case) -> Value {
    for _ in 0..WARMUPS {
        verify(case, solve(&case.request));
    }
    let mut times = Vec::with_capacity(SAMPLES);
    let mut outcomes = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let result = solve(std::hint::black_box(&case.request));
        times.push(start.elapsed().as_nanos() as u64);
        outcomes.push(verify(case, result));
    }
    let mut sorted = times.clone();
    sorted.sort_unstable();
    json!({"case":case.name,"durations_ns":times,"outcomes":outcomes,
        "p50_ns":sorted[49],"p95_ns":sorted[94],"p99_ns":sorted[98],"max_ns":sorted[99]})
}

fn main() {
    let cases = [
        case("rigid_origin_scale1", Vec3d::ZERO, 1.0, false, false),
        case("free_slider_origin_scale1", Vec3d::ZERO, 1.0, true, false),
        case("driven_slider_origin_scale1", Vec3d::ZERO, 1.0, true, true),
        case(
            "driven_slider_translated_scale100",
            Vec3d::new(20000.0, -20000.0, 20000.0),
            100.0,
            true,
            true,
        ),
    ];
    let output: Vec<_> = cases.iter().map(measure).collect();
    println!(
        "{}",
        json!({"tier":"pure_solver_only","occurrences":20,"joints":19,
        "warmups":WARMUPS,"samples":SAMPLES,"percentile":"nearest_rank",
        "pose_tolerance_mm":POSE_TOL_MM,"rotation_axis_tolerance":ROT_TOL,
        "solver_default_max_iterations":cases[0].request.max_iterations,
        "solver_default_tolerance":cases[0].request.tolerance,"cases":output})
    );
}
