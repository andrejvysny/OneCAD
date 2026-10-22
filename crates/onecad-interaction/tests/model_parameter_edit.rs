mod parameter_support;
#[allow(dead_code)]
mod support;
use onecad_core::document::{record::*, variables::VariableTable};
use onecad_interaction::*;
use parameter_support::*;
#[test]
fn all_descriptor_fields_edit_exactly_one_scalar_preserve_typed_record() {
    let mut seen = Vec::new();
    for mut operation in operations() {
        for (name, _, scalar) in operation.scalars_mut() {
            scalar.expr = Some(format!("untouched_{name}"));
        }
        if let KnownOperation::Fillet(p) = &mut operation {
            p.extra.insert("futureScalarPolicy".into(), true.into());
        }
        let mut before = record(operation);
        before
            .extra
            .insert("futureRecordPolicy".into(), "preserve".into());
        for descriptor in numeric_parameters(&before.op) {
            seen.push(descriptor.parameter);
            let value = if descriptor.parameter == NumericParameter::HoleCountersinkAngle {
                100.0
            } else {
                descriptor.scalar.value + 0.1
            };
            let mut after = edit(before.clone(), descriptor.parameter, value);
            let (Operation::Known(prior), Operation::Known(next)) = (&before.op, &mut after.op)
            else {
                panic!()
            };
            let mut prior = prior.clone();
            let mut changes = 0;
            for ((name, dimension, old), (other_name, other_dimension, new)) in
                prior.scalars_mut().into_iter().zip(next.scalars_mut())
            {
                assert_eq!((name, dimension), (other_name, other_dimension));
                if old != new {
                    changes += 1;
                    assert_eq!(*old, descriptor.scalar);
                    assert_eq!(new.value, value);
                    assert_eq!(new.expr, Some(format!("  {value}  ")));
                }
                *new = old.clone();
            }
            assert_eq!(changes, 1);
            assert_eq!(after, before);
        }
    }
    assert!(seen.contains(&NumericParameter::HoleThreadPitch));
    assert!(seen.contains(&NumericParameter::FeaturePatternAngle));
    assert!(seen.contains(&NumericParameter::TransformTranslation(ParameterAxis::Z)));
}

#[test]
fn radian_literal_uses_angle_evaluator_degrees_without_second_conversion() {
    let op = operations()
        .into_iter()
        .find(|op| matches!(op, KnownOperation::Revolve(_)))
        .unwrap();
    let (mut state, _) = begin(record(op), NumericParameter::RevolveAngle);
    let expression = "  1.5707963267948966rad  ";
    state.dispatch(Action::DraftChanged(expression.into()));
    let tag = state.tool.as_ref().unwrap().tag.clone();
    let result = onecad_core::regen::variables::evaluate_expression(
        expression.trim(),
        onecad_core::expr::Dimension::Angle,
        &VariableTable::default(),
    )
    .unwrap();
    assert!((result.number - 90.0).abs() < 1e-10);
    assert!(matches!(
        evaluate(&mut state, tag.clone(), "angle", result.number).as_slice(),
        [Effect::PreviewExistingEdit { .. }]
    ));
    complete_edit_preview(&mut state, &tag);
    state.dispatch(Action::Confirm);
    let Operation::Known(KnownOperation::Revolve(p)) = &state.tool.unwrap().record.op else {
        panic!()
    };
    assert!((p.angle_deg.value - 90.0).abs() < 1e-10);
    assert_eq!(p.angle_deg.expr.as_deref(), Some(expression));
}

#[test]
fn late_failed_regen_and_missing_completion_never_duplicate_history() {
    for unavailable in [false, true] {
        let (mut state, tag, done) = accepted_filleted_edit(!unavailable);
        if unavailable {
            state.dispatch(Action::CompletionUnavailable {
                tag: tag.clone(),
                message: "Completion stream closed".into(),
            });
            assert!(matches!(
                state.status,
                CommitStatus::CompletionUnavailable {
                    accepted_revision: 5,
                    ..
                }
            ));
        } else {
            assert!(matches!(
                state.status,
                CommitStatus::Completed {
                    outcome: FeatureOutcome::Failed(_),
                    ..
                }
            ));
        }
        assert!(state.dispatch(Action::Confirm).is_empty());
        state.dispatch(Action::RegenFinished {
            tag,
            completion: Box::new(done),
        });
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn explicit_length_units_evaluate_to_millimetres_and_keep_source_expression() {
    let op = operations()
        .into_iter()
        .find(|op| matches!(op, KnownOperation::TransformBody(_)))
        .unwrap();
    let parameter = NumericParameter::TransformTranslation(ParameterAxis::X);
    let (mut state, _) = begin(record(op), parameter);
    let expression = "1in + 2mm";
    state.dispatch(Action::DraftChanged(expression.into()));
    let tag = state.tool.as_ref().unwrap().tag.clone();
    let result = onecad_core::regen::variables::evaluate_expression(
        expression,
        onecad_core::expr::Dimension::Length,
        &VariableTable::default(),
    )
    .unwrap();
    assert!((result.number - 27.4).abs() < 1e-10);
    assert!(matches!(
        evaluate(&mut state, tag.clone(), "length", result.number).as_slice(),
        [Effect::PreviewExistingEdit { .. }]
    ));
    complete_edit_preview(&mut state, &tag);
    state.dispatch(Action::Confirm);
    let Operation::Known(KnownOperation::TransformBody(p)) = state.tool.unwrap().record.op else {
        panic!()
    };
    assert!((p.translate[0].value - 27.4).abs() < 1e-10);
    assert_eq!(p.translate[0].expr.as_deref(), Some(expression));
    assert_eq!(p.translate[1].value, -3.0);
}
