//! Executes the shared reducer's effects against the real app and worker, the
//! way the native host's services do, plus geometry probes for assertions.
use super::host::Harness;
use onecad_app::{
    api,
    dto::{DocumentProjection, ElementInfoDto},
    worker::wire::body_id_wire,
};
use onecad_core::{ids::BodyId, regen::Lod};
use onecad_interaction::*;
use onecad_scene::{MeshLimits, MeshView, SceneIdentity};
use std::collections::VecDeque;

pub async fn context(harness: &Harness) -> DocumentContext {
    let head = api::preview_context(&harness.state).await.unwrap();
    DocumentContext {
        document_id: head.document_id,
        runtime_session: head.runtime_session,
        revision: head.revision.0,
        snapshot_id: head.snapshot_id,
        scene: SceneIdentity {
            session: 1,
            scene: head.snapshot_id.0,
        },
    }
}

/// Adopts the current head, as the host does after every published display.
pub async fn adopt(harness: &Harness, state: &mut Interaction) -> DocumentContext {
    let context = context(harness).await;
    state.dispatch(Action::ContextChanged(Some(context.clone())));
    context
}

/// Runs `effects` and every follow-up to quiescence; returns the settled head
/// when an operation was committed.
pub async fn drive(
    harness: &Harness,
    state: &mut Interaction,
    effects: Vec<Effect>,
) -> Option<DocumentProjection> {
    let mut queue: VecDeque<Effect> = effects.into();
    let mut committed = None;
    while let Some(effect) = queue.pop_front() {
        let next = match effect {
            Effect::EvaluateExpression {
                tag,
                expression,
                site,
            } => {
                let result = api::evaluate_expression(&harness.state, expression, site.into())
                    .await
                    .map_err(|error| error.to_string());
                state.dispatch(Action::Evaluated { tag, result })
            }
            Effect::Preview {
                tag,
                operation,
                sketch_id,
                expected_snapshot,
            } => {
                let result = api::preview_operation(
                    &harness.state,
                    *operation,
                    tag.record_id,
                    sketch_id.map(|id| id.to_string()),
                    Some(expected_snapshot),
                    Lod::Fine,
                )
                .await
                .map_err(|error| error.to_string());
                state.dispatch(Action::PreviewFinished { tag, result })
            }
            Effect::PublishPreview { .. } | Effect::ClearPreview { .. } => Vec::new(),
            Effect::PreviewExistingEdit { tag, operation } => {
                let fence = api::preview_context(&harness.state).await.unwrap();
                let result = api::preview_existing_edit(
                    &harness.state,
                    fence,
                    tag.record_id,
                    *operation,
                    Lod::Fine,
                )
                .await;
                state.dispatch(Action::EditPreviewFinished { tag, result })
            }
            Effect::ApplyOperation { tag, command, .. } => {
                let (projection, effects) = commit(harness, state, tag, *command).await;
                committed = Some(projection);
                effects
            }
        };
        queue.extend(next);
    }
    committed
}

async fn commit(
    harness: &Harness,
    state: &mut Interaction,
    tag: RequestTag,
    command: onecad_core::edit::EditCommand,
) -> (DocumentProjection, Vec<Effect>) {
    let response = api::apply_operation(&harness.state, harness.host.clone(), vec![command], None)
        .await
        .unwrap();
    let mut effects = state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: response.projection.revision,
        rollback_token: response.rollback_token.clone(),
    });
    let report = harness.terminal(&response.projection).await;
    effects.extend(state.dispatch(Action::RegenFinished {
        tag,
        completion: Box::new(report),
    }));
    // The host closes a completed session the same way (app.rs Terminal).
    if state
        .tool
        .as_ref()
        .is_some_and(|tool| tool.phase == SessionPhase::Completed)
    {
        effects.extend(state.dispatch(Action::Cancel));
    }
    (harness.settled(&response.projection).await, effects)
}

/// Typed numeric entry into one declared field.
pub async fn typed(harness: &Harness, state: &mut Interaction, key: ParameterKey, text: &str) {
    let effects = state.dispatch(Action::ParameterChanged {
        key,
        text: text.into(),
    });
    drive(harness, state, effects).await;
}

pub async fn confirm(harness: &Harness, state: &mut Interaction) -> DocumentProjection {
    let effects = state.dispatch(Action::Confirm);
    drive(harness, state, effects)
        .await
        .expect("Confirm commits exactly one operation")
}

pub async fn volume(harness: &Harness, body: BodyId) -> f64 {
    api::query_mass_properties(&harness.state, body_id_wire(body), None)
        .await
        .unwrap()
        .volume
}

/// The exact (OCCT) volume centroid of the head body.
pub async fn centroid(harness: &Harness, body: BodyId) -> [f64; 3] {
    api::query_mass_properties(&harness.state, body_id_wire(body), None)
        .await
        .unwrap()
        .centroid
}

/// Every face or edge of the head body, by snapshot TopoKey.
pub async fn elements(harness: &Harness, body: BodyId, prefix: &str) -> Vec<ElementInfoDto> {
    let snapshot = api::preview_context(&harness.state)
        .await
        .unwrap()
        .snapshot_id
        .0;
    let mut out = Vec::new();
    for ordinal in 1..=512 {
        let key = format!("{prefix}:{ordinal}");
        match api::element_info(
            &harness.state,
            snapshot,
            body_id_wire(body),
            String::new(),
            Some(key),
            None,
        )
        .await
        {
            Ok(Some(info)) => out.push(info),
            _ => break,
        }
    }
    assert!(!out.is_empty(), "no {prefix} elements on the head body");
    out
}

/// Axis-aligned bounds of the head mesh (f32 vertex positions).
pub async fn bounds(harness: &Harness, body: BodyId) -> ([f64; 3], [f64; 3]) {
    let session = harness.projection().await.runtime_session;
    let bytes = api::get_mesh(
        &harness.state,
        body.to_string(),
        "fine".into(),
        None,
        Some(session),
    )
    .await
    .unwrap();
    let mesh = MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for index in 0..mesh.header().vertex_count as usize {
        let position = mesh.position(index).unwrap();
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(position[axis]));
            max[axis] = max[axis].max(f64::from(position[axis]));
        }
    }
    (min, max)
}

/// `actual` within `relative` of `expected`, measured against `scale`.
pub fn close(actual: f64, expected: f64, scale: f64, relative: f64, what: &str) {
    let error = (actual - expected).abs() / scale.abs();
    assert!(
        error <= relative,
        "{what}: {actual} vs {expected} (relative error {error:e} > {relative:e})"
    );
}
