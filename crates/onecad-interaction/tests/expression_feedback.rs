#[allow(dead_code)]
mod support;

use onecad_app::{api, document_runtime::DocumentRuntime, state::AppState, worker::PendingBackend};
use onecad_interaction::{Action, DraftStatus, Effect, Interaction, NumericParameter};
use std::sync::Arc;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn application() -> AppState {
    let state = AppState::default();
    let backend = Arc::new(PendingBackend);
    *state.runtime.lock().await = Some(DocumentRuntime::new_blank(
        backend.clone(),
        backend.clone(),
        backend,
    ));
    state
}

fn interaction(parameter: NumericParameter) -> Interaction {
    let (mut state, _) = support::started();
    if parameter != NumericParameter::ExtrudeDistance {
        let record = state.tool.as_ref().unwrap().record.clone();
        state.dispatch(Action::BeginParameterEdit {
            record: Box::new(record),
            parameter,
            gesture: 10,
            context: support::context(),
        });
    }
    state
}

async fn evaluate(app: &AppState, state: &mut Interaction, expression: &str) -> Vec<Effect> {
    let tag = state
        .dispatch(Action::DraftChanged(expression.into()))
        .into_iter()
        .find_map(|effect| match effect {
            Effect::EvaluateExpression { tag, .. } => Some(tag),
            _ => None,
        })
        .unwrap();
    let site = state.tool.as_ref().unwrap().parameter.site();
    let result = api::evaluate_expression(app, expression.into(), site.into())
        .await
        .map_err(|error| error.to_string());
    state.dispatch(Action::Evaluated { tag, result })
}

#[test]
fn actual_application_length_results_accept_scalar_and_length() {
    runtime().block_on(async {
        let app = application().await;
        for expression in ["10", "6", "2 * 3", "6mm"] {
            let mut state = interaction(NumericParameter::ExtrudeDistance);
            let effects = evaluate(&app, &mut state, expression).await;
            assert!(matches!(effects.as_slice(), [Effect::Preview { .. }]));
            let DraftStatus::Valid(scalar) = &state.tool.as_ref().unwrap().draft_status else {
                panic!("valid application result rejected: {expression}");
            };
            assert_eq!(scalar.value, if expression == "10" { 10.0 } else { 6.0 });
            assert_eq!(scalar.expr.as_deref(), Some(expression));
        }
    });
}

#[test]
fn actual_application_angle_results_accept_scalar_and_angle() {
    runtime().block_on(async {
        let app = application().await;
        for expression in ["6", "2 * 3", "6deg"] {
            let mut state = interaction(NumericParameter::ExtrudeDraftAngle);
            assert!(evaluate(&app, &mut state, expression).await.is_empty());
            assert!(matches!(
                &state.tool.as_ref().unwrap().draft_status,
                DraftStatus::Valid(scalar) if scalar.value == 6.0
            ));
        }
    });
}

#[test]
fn actual_application_errors_cannot_preview_or_commit() {
    runtime().block_on(async {
        let app = application().await;
        for (parameter, expression) in [
            (NumericParameter::ExtrudeDistance, "6deg"),
            (NumericParameter::ExtrudeDraftAngle, "6mm"),
            (NumericParameter::ExtrudeDistance, "1 / 0"),
            (NumericParameter::ExtrudeDistance, "1e309"),
            (NumericParameter::ExtrudeDistance, "2 +"),
            (NumericParameter::ExtrudeDistance, ""),
        ] {
            let mut state = interaction(parameter);
            assert!(evaluate(&app, &mut state, expression).await.is_empty());
            assert!(matches!(
                state.tool.as_ref().unwrap().draft_status,
                DraftStatus::Invalid(_)
            ));
            assert!(state.dispatch(Action::Confirm).is_empty());
        }
    });
}
