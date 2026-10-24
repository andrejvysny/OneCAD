mod parameter_support;
#[allow(dead_code)]
mod support;
use onecad_core::{document::record::*, ids::BodyId};
use onecad_interaction::*;
use parameter_support::*;
use support::context;
#[test]
fn typed_authoring_bounds_refuse_without_clamping_or_committing() {
    for (parameter, value) in [
        (NumericParameter::FilletRadius, 0.099),
        (NumericParameter::ChamferSecondDistance, 0.099),
        (NumericParameter::RevolveAngle, -30.0),
        (NumericParameter::RevolveAngle, 0.49),
        (NumericParameter::CircularPatternAngle, 0.99),
        (NumericParameter::HoleDiameter, 0.009),
        (NumericParameter::ChamferAngle, 180.0),
        (NumericParameter::TransformRotationAngle, f64::INFINITY),
    ] {
        let before = operations()
            .into_iter()
            .map(record)
            .find(|r| {
                numeric_parameters(&r.op)
                    .iter()
                    .any(|d| d.parameter == parameter)
            })
            .unwrap();
        let (mut state, _) = begin(before.clone(), parameter);
        let tag = typed(&mut state, &value.to_string());
        evaluate(&mut state, tag, parameter.site(), value);
        assert!(
            matches!(
                state.tool.as_ref().unwrap().draft_status,
                DraftStatus::Invalid(_)
            ),
            "{parameter:?}"
        );
        assert!(state.dispatch(Action::Confirm).is_empty());
        assert_eq!(state.tool.unwrap().record, before);
    }
}

#[test]
fn shell_drag_floor_does_not_restrict_typed_edit_and_spacing_stays_signed() {
    for (parameter, value) in [
        (NumericParameter::ShellThickness, -0.02),
        (NumericParameter::LinearPatternSpacing, -3.0),
    ] {
        let before = operations()
            .into_iter()
            .map(record)
            .find(|r| {
                numeric_parameters(&r.op)
                    .iter()
                    .any(|d| d.parameter == parameter)
            })
            .unwrap();
        let after = edit(before, parameter, value);
        assert_eq!(
            numeric_parameters(&after.op)
                .into_iter()
                .find(|d| d.parameter == parameter)
                .unwrap()
                .scalar
                .value,
            value
        );
    }
}

#[test]
fn optional_and_mode_inactive_parameters_are_explicitly_unsupported() {
    let base = record(KnownOperation::Hole(hole(BodyId::new())));
    assert!(!numeric_parameters(&base.op)
        .iter()
        .any(|d| d.parameter == NumericParameter::HoleCountersinkDiameter));
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    assert!(state
        .dispatch(Action::BeginParameterEdit {
            record: Box::new(base),
            parameter: NumericParameter::HoleCountersinkDiameter,
            gesture: 5,
            context: context()
        })
        .is_empty());
    assert!(matches!(state.status, CommitStatus::Rejected(_)));
    assert!(state.tool.is_none());
    let (old, _) = support::started();
    let mut extrusion = old.tool.unwrap().record;
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut extrusion.op {
        p.mode = ExtrudeMode::ThroughAll;
    }
    assert!(!numeric_parameters(&extrusion.op)
        .iter()
        .any(|d| d.parameter == NumericParameter::ExtrudeDistance));
    state.dispatch(Action::BeginEdit {
        record: Box::new(extrusion),
        gesture: 6,
        context: context(),
    });
    assert!(matches!(state.status, CommitStatus::Rejected(_)));
    assert!(state.tool.is_none());
}

#[test]
fn core_candidate_validation_rejects_cross_field_hole_and_transform_limits() {
    let hole = record(KnownOperation::Hole(hole(BodyId::new())));
    let (mut state, _) = begin(hole.clone(), NumericParameter::HoleThreadPitch);
    let tag = typed(&mut state, "6");
    evaluate(&mut state, tag, "length", 6.0);
    assert!(matches!(
        state.tool.as_ref().unwrap().draft_status,
        DraftStatus::Invalid(_)
    ));
    assert_eq!(state.tool.unwrap().record, hole);
    let op = operations()
        .into_iter()
        .find(|op| matches!(op,KnownOperation::Hole(p) if p.hole_type==HoleType::Counterbore))
        .unwrap();
    let (mut state, _) = begin(record(op), NumericParameter::HoleDiameter);
    let tag = typed(&mut state, "10");
    evaluate(&mut state, tag, "length", 10.0);
    assert!(matches!(
        state.tool.as_ref().unwrap().draft_status,
        DraftStatus::Invalid(_)
    ));
}

#[test]
fn evaluation_site_errors_and_stale_results_cannot_change_valid_parameter() {
    let op = operations()
        .into_iter()
        .find(|op| matches!(op, KnownOperation::Revolve(_)))
        .unwrap();
    let (mut state, stale) = begin(record(op), NumericParameter::RevolveAngle);
    state.dispatch(Action::DraftChanged("90deg".into()));
    assert!(evaluate(&mut state, stale, "angle", 90.0).is_empty());
    assert_eq!(
        state.tool.as_ref().unwrap().draft_status,
        DraftStatus::Pending
    );
    let current = state.tool.as_ref().unwrap().tag.clone();
    evaluate(&mut state, current, "length", 90.0);
    assert!(matches!(
        state.tool.as_ref().unwrap().draft_status,
        DraftStatus::Invalid(_)
    ));
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn second_extrude_leg_preserves_first_leg_and_disables_unproven_draft_mode() {
    let (initial, _) = support::started();
    let mut before = initial.tool.unwrap().record;
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut before.op {
        p.two_directions = true;
        p.distance2 = onecad_core::document::variables::Scalar::new(8.0);
    }
    let descriptors = numeric_parameters(&before.op);
    assert!(descriptors
        .iter()
        .any(|d| d.parameter == NumericParameter::ExtrudeSecondDistance));
    assert!(!descriptors
        .iter()
        .any(|d| d.parameter == NumericParameter::ExtrudeDraftAngle));
    let after = edit(
        before.clone(),
        NumericParameter::ExtrudeSecondDistance,
        12.0,
    );
    let Operation::Known(KnownOperation::Extrude(p)) = after.op else {
        panic!()
    };
    assert_eq!(p.distance.value, 10.0);
    assert_eq!(p.distance2.value, 12.0);
    let (mut state, _) = begin(before, NumericParameter::ExtrudeSecondDistance);
    let tag = typed(&mut state, "-3");
    evaluate(&mut state, tag, "length", -3.0);
    assert!(matches!(
        state.tool.unwrap().draft_status,
        DraftStatus::Invalid(_)
    ));
}

#[test]
fn inactive_draft_and_through_all_hole_depth_never_appear_as_edit_controls() {
    let (initial, _) = support::started();
    let mut extrusion = initial.tool.unwrap().record;
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut extrusion.op {
        p.mode = ExtrudeMode::ThroughAll;
    }
    assert!(!numeric_parameters(&extrusion.op)
        .iter()
        .any(|d| d.parameter == NumericParameter::ExtrudeDraftAngle));
    let mut params = hole(BodyId::new());
    params.depth = None;
    let before = record(KnownOperation::Hole(params));
    assert!(!numeric_parameters(&before.op)
        .iter()
        .any(|d| d.parameter == NumericParameter::HoleDepth));
}

#[test]
fn draft_angle_and_rotation_axis_use_their_own_validation_sites() {
    let (initial, _) = support::started();
    let (mut state, _) = begin(
        initial.tool.unwrap().record,
        NumericParameter::ExtrudeDraftAngle,
    );
    let tag = typed(&mut state, "90");
    evaluate(&mut state, tag, "angle", 90.0);
    assert!(matches!(
        state.tool.unwrap().draft_status,
        DraftStatus::Invalid(_)
    ));
    let mut op = operations()
        .into_iter()
        .find(|op| matches!(op, KnownOperation::TransformBody(_)))
        .unwrap();
    if let KnownOperation::TransformBody(p) = &mut op {
        p.rotate.axis = onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 0.0);
    }
    let (mut state, _) = begin(record(op), NumericParameter::TransformRotationAngle);
    let tag = typed(&mut state, "45");
    evaluate(&mut state, tag, "angle", 45.0);
    assert!(matches!(
        state.tool.unwrap().draft_status,
        DraftStatus::Invalid(_)
    ));
}

#[test]
fn thread_pitch_uses_core_positive_domain_instead_of_hole_dimension_floor() {
    let before = record(KnownOperation::Hole(hole(BodyId::new())));
    let after = edit(before, NumericParameter::HoleThreadPitch, 0.001);
    let Operation::Known(KnownOperation::Hole(p)) = after.op else {
        panic!()
    };
    assert_eq!(p.thread.unwrap().pitch_mm.value, 0.001);
}

#[test]
fn unsupported_parameter_switch_cancels_old_valid_draft_before_confirmation() {
    let before = record(edge_operations(BodyId::new()).remove(0));
    let (mut state, old_tag) = begin(before.clone(), NumericParameter::FilletRadius);
    evaluate(&mut state, old_tag.clone(), "length", 4.0);
    assert!(matches!(
        state.tool.as_ref().unwrap().draft_status,
        DraftStatus::Valid(_)
    ));
    let effects = state.dispatch(Action::BeginParameterEdit {
        record: Box::new(before),
        parameter: NumericParameter::HoleDiameter,
        gesture: 43,
        context: context(),
    });
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClearPreview { gesture: 42 })));
    assert!(matches!(state.status, CommitStatus::Rejected(_)));
    assert!(state.tool.is_none());
    assert!(state.dispatch(Action::Confirm).is_empty());
    assert!(evaluate(&mut state, old_tag, "length", 9.0).is_empty());
}

#[test]
fn canonical_handle_values_share_typed_domains_and_never_request_expression_evaluation() {
    for (parameter, number, valid) in [
        (NumericParameter::FilletRadius, 4.0, true),
        (NumericParameter::FilletRadius, 0.099, false),
        (NumericParameter::ChamferDistance, 4.0, true),
        (NumericParameter::ChamferSecondDistance, 0.099, false),
        (NumericParameter::ChamferAngle, 60.0, true),
        (NumericParameter::ChamferAngle, 180.0, false),
        (NumericParameter::OffsetFaceDistance, -4.0, true),
        (NumericParameter::RevolveAngle, 150.0, true),
        (NumericParameter::RevolveAngle, -30.0, false),
    ] {
        let before = operations()
            .into_iter()
            .map(record)
            .find(|record| {
                numeric_parameters(&record.op)
                    .iter()
                    .any(|descriptor| descriptor.parameter == parameter)
            })
            .unwrap();
        let (mut state, _) = begin(before, parameter);
        let effects = state.dispatch(Action::ParameterDrag(DragPhase::Value {
            key: parameter.into(),
            text: number.to_string(),
            value: onecad_core::expr::Value {
                number,
                dim: parameter.dimension(),
            },
        }));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::EvaluateExpression { .. })),
            "{parameter:?}"
        );
        assert_eq!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::PreviewExistingEdit { .. })),
            valid,
            "{parameter:?}"
        );
        assert_eq!(
            matches!(
                state.tool.as_ref().unwrap().draft_status,
                DraftStatus::Invalid(_)
            ),
            !valid,
            "{parameter:?}"
        );
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}
