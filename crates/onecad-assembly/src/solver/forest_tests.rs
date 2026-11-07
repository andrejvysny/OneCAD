use super::*;

fn source_tree() -> SolveRequest {
    serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/robonine-grounded-tree.json"
    ))
    .unwrap()
}

#[test]
fn source_grounded_tree_has_three_independent_joint_dofs() {
    let request = source_tree();
    let outcome = solve(&request);
    assert!(
        matches!(
            outcome,
            SolveOutcome::Converged {
                remaining_dof: 3,
                redundant_constraints: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn source_tree_finite_differences_expose_small_column_guard() {
    let request = source_tree();
    let cons = build_constraints(&request).unwrap();
    let layout = build_layout(&request.bodies);
    let problem = Problem {
        cons: &cons,
        layout: &layout,
        tol: request.tolerance,
        max_iter: request.max_iterations,
    };
    let poses: Vec<_> = request.bodies.iter().map(|body| body.pose).collect();
    let rows = residuals(&poses, &cons).len();
    let jac = jacobian_with_step(&poses, &problem, rows, FD_STEP);
    let tiny: Vec<_> = (0..layout.unknowns)
        .filter_map(|col| {
            let scale = (0..rows)
                .map(|row| jac[row * layout.unknowns + col].abs())
                .fold(0.0_f64, f64::max);
            (scale > 0.0 && scale < RANK_REL_TOL * 10.0).then_some((col, scale))
        })
        .collect();
    assert!(
        !tiny.is_empty(),
        "expected captured finite-difference scale issue"
    );
    assert_eq!(numerical_rank(jac, rows, layout.unknowns), None);
    eprintln!("source tree small finite-difference columns: {tiny:?}");
}

fn bounded_slider(max: f64) -> SolveRequest {
    let mut request = source_tree();
    request.bodies.truncate(2);
    for body in &mut request.bodies {
        body.pose = RigidTransform::IDENTITY;
    }
    request.joints.truncate(1);
    let frame = crate::math::Frame {
        origin: Vec3d::ZERO,
        z: Vec3d::Z,
        x: Vec3d::X,
    };
    request.joints[0].a.frame = frame;
    request.joints[0].b.frame = frame;
    request.joints[0].limits = Some(JointLimits { min: 0.0, max });
    request
}

#[test]
fn equality_limit_removes_motion_dof_even_when_already_at_limit() {
    let outcome = solve(&bounded_slider(0.0));
    assert!(
        matches!(
            outcome,
            SolveOutcome::Converged {
                remaining_dof: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn one_sided_limit_endpoint_retains_inward_motion_dof() {
    let outcome = solve(&bounded_slider(1.0));
    assert!(
        matches!(
            outcome,
            SolveOutcome::Converged {
                remaining_dof: 1,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn temporary_limit_clamp_retains_inward_motion_dof() {
    for start in [-1.0, 2.0] {
        let mut request = bounded_slider(1.0);
        request.bodies[1].pose.translation.z = start;
        let outcome = solve(&request);
        let SolveOutcome::Converged {
            poses,
            remaining_dof,
            ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        let endpoint = if start < 0.0 { 0.0 } else { 1.0 };
        assert!((poses[1].1.translation.z - endpoint).abs() <= request.tolerance);
        assert_eq!(remaining_dof, 1, "temporary clamp at {endpoint}");
        request.bodies[1].pose = copy_pose(&poses[1].1);
        assert!(matches!(
            solve(&request),
            SolveOutcome::Converged {
                remaining_dof: 1,
                ..
            }
        ));
    }
}

#[test]
fn translated_slider_and_revolute_clamps_preserve_authored_mobility() {
    for kind in [JointKind::Slider, JointKind::Revolute] {
        for start in [-1.0, 2.0] {
            let mut request = bounded_slider(1.0);
            request.joints[0].kind = kind;
            for body in &mut request.bodies {
                body.pose.translation = Vec3d::new(20_000.0, -20_000.0, 20_000.0);
            }
            if kind == JointKind::Slider {
                request.bodies[1].pose.translation.z += start;
            } else {
                request.bodies[1].pose.rotation = Quat::from_axis_angle(Vec3d::Z, start).unwrap();
            }
            let outcome = solve(&request);
            let SolveOutcome::Converged {
                poses,
                remaining_dof,
                ..
            } = outcome
            else {
                panic!("{kind:?} at {start}: {outcome:?}");
            };
            assert_eq!(remaining_dof, 1, "{kind:?} at {start}");
            request.bodies[1].pose = copy_pose(&poses[1].1);
            assert!(matches!(
                solve(&request),
                SolveOutcome::Converged {
                    remaining_dof: 1,
                    ..
                }
            ));
            request.drive.push((request.joints[0].id, 0.5));
            assert!(matches!(
                solve(&request),
                SolveOutcome::Converged {
                    remaining_dof: 0,
                    ..
                }
            ));
        }
    }
}

#[test]
fn equality_limits_still_remove_dof_after_motion() {
    for kind in [JointKind::Slider, JointKind::Revolute] {
        let mut request = bounded_slider(0.0);
        request.joints[0].kind = kind;
        request.bodies[1].pose = if kind == JointKind::Slider {
            RigidTransform {
                translation: Vec3d::Z,
                ..RigidTransform::IDENTITY
            }
        } else {
            RigidTransform {
                rotation: Quat::from_axis_angle(Vec3d::Z, 1.0).unwrap(),
                ..RigidTransform::IDENTITY
            }
        };
        let outcome = solve(&request);
        assert!(
            matches!(
                outcome,
                SolveOutcome::Converged {
                    remaining_dof: 0,
                    ..
                }
            ),
            "{outcome:?}"
        );
    }
}

fn structural_report(request: &SolveRequest) -> Option<(usize, usize)> {
    let cons = build_constraints(request).unwrap();
    let layout = build_layout(&request.bodies);
    let poses: Vec<_> = request.bodies.iter().map(|body| body.pose).collect();
    forest::dof(
        &poses,
        &Problem {
            cons: &cons,
            layout: &layout,
            tol: request.tolerance,
            max_iter: request.max_iterations,
        },
    )
}

#[test]
fn forest_certificate_handles_floating_roots_and_refuses_cycles_or_two_grounds() {
    let mut request = source_tree();
    assert_eq!(structural_report(&request), Some((3, 0)));
    request.bodies[0].grounded = false;
    assert_eq!(structural_report(&request), Some((9, 0)));
    request.bodies[0].grounded = true;
    request.bodies[1].grounded = true;
    assert_eq!(structural_report(&request), None);
    request.bodies[1].grounded = false;
    request.joints.push(request.joints[0].clone());
    assert_eq!(structural_report(&request), None);
}

#[test]
fn loose_tolerance_does_not_certify_antiparallel_rotation_chart() {
    let mut request = bounded_slider(1.0);
    request.joints[0].kind = JointKind::Revolute;
    request.bodies[1].pose.rotation =
        Quat::from_axis_angle(Vec3d::Y, std::f64::consts::PI).unwrap();
    request.tolerance = 3.0;
    assert_eq!(structural_report(&request), None);
}

#[test]
fn source_tree_driven_pi_seam_is_regular() {
    let mut request = source_tree();
    request.bodies[3].pose.rotation =
        Quat::from_axis_angle(Vec3d::Z, std::f64::consts::PI).unwrap();
    let gear = request
        .joints
        .iter()
        .find(|joint| joint.kind == JointKind::Revolute)
        .unwrap();
    request.drive.push((gear.id, std::f64::consts::PI));
    assert_eq!(structural_report(&request), Some((2, 0)));
}

#[test]
fn source_tree_simultaneous_half_turn_and_slider_endpoints_converge() {
    let request: SolveRequest = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/robonine-endpoint-tree.json"
    ))
    .unwrap();
    let outcome = solve(&request);
    assert!(
        matches!(
            outcome,
            SolveOutcome::Converged {
                remaining_dof: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn half_turn_drive_derivative_uses_the_circular_difference() {
    for target in [-std::f64::consts::PI, std::f64::consts::PI] {
        let mut request = bounded_slider(1.0);
        request.joints[0].kind = JointKind::Revolute;
        request.joints[0].limits = None;
        request.drive.push((request.joints[0].id, target));
        let cons = build_constraints(&request).unwrap();
        let layout = build_layout(&request.bodies);
        let poses: Vec<_> = request.bodies.iter().map(|body| body.pose).collect();
        let problem = Problem {
            cons: &cons,
            layout: &layout,
            tol: request.tolerance,
            max_iter: request.max_iterations,
        };
        let jac = jacobian(&poses, &problem, 6);
        assert!(
            (jac[5 * 6 + 2] - 1.0).abs() < 1e-8,
            "target {target}, derivative {}",
            jac[5 * 6 + 2]
        );
    }
}
