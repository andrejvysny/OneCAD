#[allow(dead_code)]
mod support;

use onecad_interaction::*;

const DISTANCE: ParameterKey = ParameterKey::Scalar(NumericParameter::ExtrudeDistance);

/// The engine holds each exact request while more input arrives; no wall clock
/// or real worker is needed to reproduce a request slower than pointer motion.
struct DelayedEngine(RequestTag);

impl DelayedEngine {
    fn finish(self, state: &mut Interaction) -> Vec<Effect> {
        state.dispatch(Action::PreviewFinished {
            result: Ok(support::preview(&self.0)),
            tag: self.0,
        })
    }
}

fn evaluate(state: &mut Interaction, effects: Vec<Effect>) -> Vec<Effect> {
    let Some((tag, expression)) = effects.iter().find_map(|effect| match effect {
        Effect::EvaluateExpression {
            tag, expression, ..
        } => Some((tag.clone(), expression.clone())),
        _ => None,
    }) else {
        return effects;
    };
    state.dispatch(Action::Evaluated {
        tag,
        result: Ok(EvaluatedExpressionDto {
            value: expression.parse().unwrap(),
            dimension: "length".into(),
            error: None,
        }),
    })
}

fn request(effects: &[Effect]) -> DelayedEngine {
    DelayedEngine(
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Preview { tag, .. } => Some(tag.clone()),
                _ => None,
            })
            .expect("one exact request must make progress"),
    )
}

#[test]
fn delayed_engine_displays_progress_during_continuous_input_without_certifying_latest() {
    let (mut state, tag) = support::started();
    let initial = state.dispatch(Action::Evaluated {
        tag,
        result: Ok(EvaluatedExpressionDto {
            value: 10.0,
            dimension: "length".into(),
            error: None,
        }),
    });
    let mut engine = request(&initial);
    for round in 0..3 {
        for value in (20 + round * 10)..(30 + round * 10) {
            let changed = state.dispatch(Action::ParameterChanged {
                key: DISTANCE,
                text: value.to_string(),
            });
            assert!(
                !changed.iter().any(|effect| matches!(
                    effect,
                    Effect::Preview { .. } | Effect::PreviewExistingEdit { .. }
                )),
                "one submitted preview owns the lane until terminal"
            );
            let evaluated = evaluate(&mut state, changed);
            assert!(!evaluated.iter().any(|effect| matches!(
                effect,
                Effect::Preview { .. } | Effect::PreviewExistingEdit { .. }
            )));
        }
        let submitted = engine.0.clone();
        let finished = engine.finish(&mut state);
        assert!(
            finished.iter().any(
                |effect| matches!(effect, Effect::PublishPreview { tag, .. } if tag == &submitted)
            ),
            "a completed submitted request must display progress while a newer desired draft is pending"
        );
        assert!(
            !state.tool.as_ref().unwrap().ready(),
            "intermediate geometry must not certify the latest draft"
        );
        assert!(state.dispatch(Action::Confirm).is_empty());
        let next = evaluate(&mut state, finished);
        engine = request(&next);
    }
    let latest = engine.0.clone();
    let finished = engine.finish(&mut state);
    assert!(finished
        .iter()
        .any(|effect| matches!(effect, Effect::PublishPreview { tag, .. } if tag == &latest)));
    assert!(state.tool.as_ref().unwrap().ready());
    assert_eq!(
        state
            .dispatch(Action::Confirm)
            .iter()
            .filter(|effect| matches!(effect, Effect::ApplyOperation { .. }))
            .count(),
        1
    );
}

fn value(state: &mut Interaction, number: f64, dim: onecad_core::expr::Dimension) -> Vec<Effect> {
    state.dispatch(Action::ParameterDrag(DragPhase::Value {
        key: DISTANCE,
        text: number.to_string(),
        value: onecad_core::expr::Value { number, dim },
    }))
}

#[test]
fn dimensioned_handles_skip_evaluation_and_reject_nonfinite_or_wrong_units() {
    use onecad_core::expr::Dimension;
    for (number, dim) in [
        (f64::NAN, Dimension::Length),
        (f64::INFINITY, Dimension::Length),
        (0.0, Dimension::Length),
        (8.0, Dimension::Angle),
    ] {
        let (mut state, _) = support::started();
        let effects = value(&mut state, number, dim);
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            Effect::EvaluateExpression { .. } | Effect::Preview { .. }
        )));
        assert!(matches!(
            state.tool.as_ref().unwrap().draft_status,
            DraftStatus::Invalid(_)
        ));
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
    let (mut state, _) = support::started();
    let effects = value(&mut state, 12.5, Dimension::Length);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Preview { .. })));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::EvaluateExpression { .. })));
}

#[test]
fn pending_scalar_keeps_display_and_hard_context_loss_rejects_exact_terminal() {
    let (mut state, tag) = support::started();
    let first = request(&support::evaluate(&mut state, &tag, 10.0));
    first.finish(&mut state);
    let shown = state.tool.as_ref().unwrap().preview_revisions()[2];
    let effects = value(&mut state, 12.0, onecad_core::expr::Dimension::Length);
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClearPreview { .. })));
    assert_eq!(state.tool.as_ref().unwrap().preview_revisions()[2], shown);
    let engine = request(&effects);
    state.dispatch(Action::ContextChanged(None));
    assert!(engine.finish(&mut state).is_empty());
    assert!(state.tool.is_none());
}

#[test]
fn drag_cancel_restores_expression_and_never_revives_cancelled_value() {
    let (mut state, _) = support::started();
    state.dispatch(Action::DraftChanged("Depth + 2mm".into()));
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    let engine = request(&value(
        &mut state,
        14.0,
        onecad_core::expr::Dimension::Length,
    ));
    let effects = state.dispatch(Action::ParameterDrag(DragPhase::Cancel));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClearPreview { .. })));
    assert_eq!(state.tool.as_ref().unwrap().draft, "Depth + 2mm");
    assert!(engine.finish(&mut state).is_empty());
    assert!(state.dispatch(Action::Confirm).is_empty());
}
