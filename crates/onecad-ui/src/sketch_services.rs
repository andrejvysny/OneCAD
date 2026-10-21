//! Serialized sketch services preserve the shared application authoring authority.
use crate::bridge::{Backend, Display, Message};
use onecad_app::{
    api,
    dto::{SketchSessionDto, SketchUpsertDto},
};
use onecad_core::ids::SketchId;
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchAction, SketchEffect, SketchSnapshot, SketchTag},
};
use std::sync::{Arc, mpsc};
use tokio::sync::OwnedSemaphorePermit;

#[derive(Debug)]
pub enum Request {
    SelectionPreview {
        expected: api::PreviewContext,
        id: SketchId,
        sender: mpsc::Sender<Result<onecad_core::sketch::Sketch, String>>,
    },
    FacePreview {
        context: DocumentContext,
        body: onecad_core::ids::BodyId,
        identity: String,
        sender: mpsc::Sender<Result<onecad_core::sketch::SketchPlane, String>>,
    },
    FaceEntry {
        id: SketchId,
        context: DocumentContext,
        body: onecad_core::ids::BodyId,
        identity: String,
        control: onecad_app::sketch_candidate::SketchCandidateControl,
        /// The UI entry owns the request while this, or its delivery's clone, lives.
        lease: Arc<()>,
        #[cfg(test)]
        admission_gate: Option<Arc<tokio::sync::Semaphore>>,
    },
    Load {
        id: SketchId,
        context: DocumentContext,
    },
    Effect(SketchEffect),
    Candidate {
        effect: SketchEffect,
        control: onecad_app::sketch_candidate::SketchCandidateControl,
    },
}
pub struct Delivery {
    pub tag: Option<SketchTag>,
    pub display: Option<Arc<Display>>,
    pub actions: Vec<SketchAction>,
    pub error: Option<String>,
    /// A FaceEntry's lease; dropped only after the UI applied this delivery.
    pub _lease: Option<Arc<()>>,
}
impl Delivery {
    pub fn stale_actions(self) -> Vec<SketchAction> {
        let mut actions: Vec<_> = self
            .actions
            .into_iter()
            .filter(|action| matches!(action, SketchAction::BatchAccepted { .. }))
            .collect();
        if let Some(tag) = self.tag {
            actions.push(SketchAction::Failed {
                tag,
                message: "Sketch scene became stale before delivery".into(),
            });
        }
        actions
    }
}

enum Reply {
    BeginCreated(Box<onecad_core::sketch::Sketch>),
    Begin(Box<onecad_core::sketch::Sketch>),
    Created,
    Entered(Box<SketchSessionDto>, bool),
    Solved(Box<SketchUpsertDto>),
    ConstraintRefused(String),
    History(onecad_app::dto::SketchHistoryDto),
    Finished(Box<onecad_app::dto::FinishSketchDto>),
    FinishRefused {
        message: String,
        diagnostics: Vec<onecad_core::regen::Diagnostic>,
    },
    Discarded(onecad_app::dto::CancelSketchDto),
}

pub async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    if let Request::SelectionPreview {
        expected,
        id,
        sender,
    } = &request
    {
        let result = selection_preview(&backend.state, expected, *id).await;
        let _ = sender.send(result);
        backend.shared.wake();
        return;
    }
    if let Request::FacePreview {
        context,
        body,
        identity,
        sender,
    } = &request
    {
        let result = preview_face(&backend.state, context, *body, identity).await;
        let _ = sender.send(result);
        backend.shared.wake();
        return;
    }
    let tag = match &request {
        Request::Effect(effect) => Some(effect_tag(effect).clone()),
        Request::Candidate { effect, .. } => Some(effect_tag(effect).clone()),
        _ => None,
    };
    let lease = match &request {
        Request::FaceEntry { lease, .. } => Some(lease.clone()),
        _ => None,
    };
    let mut actions = Vec::new();
    let result = match run(backend, request, &mut actions).await {
        Ok(reply) => api::preview_context(&backend.state)
            .await
            .map(|context| (reply, context))
            .map_err(|error| error.to_string()),
        Err(error) => Err(error),
    };
    backend.events.finish();
    crate::document_state::refresh(backend).await;
    let mut display = None;
    let mut error = None;
    match result {
        Ok((reply, expected)) => match crate::capture::load_display(backend).await {
            Ok(captured) if captured.context != expected => {
                error = Some("Sketch head changed during final capture".into())
            }
            Ok(captured) => {
                let context = document_context(&captured);
                match feedback(backend, tag.as_ref(), reply, context).await {
                    Ok(action) => {
                        actions.push(action);
                        display = Some(Arc::new(captured));
                    }
                    Err(message) => error = Some(message),
                }
            }
            Err(message) => error = Some(message),
        },
        Err(message) => {
            error = Some(message);
            if let Ok(captured) = crate::capture::load_display(backend).await {
                display = Some(Arc::new(captured));
            }
        }
    }
    if let Some(message) = &error {
        if let Some(tag) = tag.clone() {
            actions.push(SketchAction::Failed {
                tag,
                message: message.clone(),
            });
        }
        if display.is_none() {
            crate::services::invalidate(&backend.shared, format!("Sketch: {message}"));
        }
    }
    let _ = sender.send(Message::Sketch {
        delivery: Box::new(Delivery {
            tag,
            display,
            actions,
            error,
            _lease: lease,
        }),
        _permit: permit,
    });
    backend.shared.wake();
}

async fn run(
    backend: &mut Backend,
    request: Request,
    actions: &mut Vec<SketchAction>,
) -> Result<Reply, String> {
    let state = &backend.state;
    match request {
        Request::SelectionPreview { .. } => Err("Sketch preview is read-only".into()),
        Request::FacePreview { .. } => Err("Face preview is read-only".into()),
        Request::FaceEntry {
            id,
            context,
            body,
            identity,
            control,
            #[cfg(test)]
            admission_gate,
            ..
        } => {
            check_context(state, &context).await?;
            let scope = api::preview_context(state)
                .await
                .map_err(|e| e.to_string())?;
            check_context(state, &context).await?;
            if scope.runtime_session != context.runtime_session
                || scope.revision.0 != context.revision
                || scope.snapshot_id != context.snapshot_id
            {
                return Err("Sketch support belongs to a replaced document head".into());
            }
            let (element, topo) = if onecad_core::ids::TopoKey::new(identity.clone()).is_valid() {
                (String::new(), Some(identity))
            } else {
                (identity, None)
            };
            backend.events.begin(crate::event_mailbox::Expected::Known {
                session: context.runtime_session,
                revision: context.revision + 1,
            });
            let prepared = api::prepare_sketch_on_face(
                state,
                scope,
                onecad_app::worker::wire::body_id_wire(body),
                element,
                topo,
                id.to_string(),
                "Sketch".into(),
            )
            .await
            .map_err(|e| e.to_string())?;
            crate::record_budget::check_payload(prepared.sketch())?;
            #[cfg(test)]
            if let Some(gate) = admission_gate {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(10), gate.acquire()).await;
            }
            api::commit_sketch_on_face_controlled(state, prepared, control)
                .await
                .map_err(|e| e.to_string())?;
            api::get_sketch_snapshot(state, id)
                .await
                .map(Box::new)
                .map(Reply::BeginCreated)
                .map_err(|e| e.to_string())
        }
        Request::Load { id, context } => {
            check_context(state, &context).await?;
            let sketch = api::get_sketch_snapshot(state, id)
                .await
                .map_err(|e| e.to_string())?;
            check_context(state, &context).await?;
            crate::record_budget::check_payload(&sketch)?;
            Ok(Reply::Begin(Box::new(sketch)))
        }
        Request::Effect(effect) => {
            execute_effect(backend, effect, actions, Default::default()).await
        }
        Request::Candidate { effect, control } => {
            execute_effect(backend, effect, actions, control).await
        }
    }
}

async fn selection_preview(
    state: &onecad_app::state::AppState,
    expected: &api::PreviewContext,
    id: SketchId,
) -> Result<onecad_core::sketch::Sketch, String> {
    if &api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?
        != expected
    {
        return Err("Sketch selection belongs to an obsolete document or worker head".into());
    }
    let sketch = api::get_sketch_snapshot(state, id)
        .await
        .map_err(|error| error.to_string())?;
    if &api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?
        != expected
    {
        return Err("Document changed while reading selected sketch".into());
    }
    Ok(sketch)
}

async fn preview_face(
    state: &onecad_app::state::AppState,
    context: &DocumentContext,
    body: onecad_core::ids::BodyId,
    identity: &str,
) -> Result<onecad_core::sketch::SketchPlane, String> {
    check_context(state, context).await?;
    let scope = api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?;
    let address = if onecad_core::ids::TopoKey::new(identity.to_owned()).is_valid() {
        onecad_app::worker::wire::FaceAddress::TopoKey(identity)
    } else {
        onecad_app::worker::wire::FaceAddress::ElementId(identity)
    };
    let frame = state
        .face_projection()
        .project_face_boundary_frame(context.snapshot_id, body, address)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Face is unresolved or is not planar; choose another support")?;
    check_context(state, context).await?;
    if api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?
        != scope
    {
        return Err(
            "Sketch support belongs to an obsolete worker head; select the face again".into(),
        );
    }
    Ok(onecad_core::sketch::plane_from_point_normal(
        frame.origin,
        frame.normal,
    ))
}

async fn execute_effect(
    backend: &mut Backend,
    effect: SketchEffect,
    actions: &mut Vec<SketchAction>,
    control: onecad_app::sketch_candidate::SketchCandidateControl,
) -> Result<Reply, String> {
    let tag = effect_tag(&effect).clone();
    check_context(&backend.state, &tag.document).await?;
    backend.events.begin(crate::event_mailbox::Expected::Known {
        session: tag.document.runtime_session.clone(),
        revision: tag.document.revision + 1,
    });
    let id = tag.key.sketch_id.to_string();
    let host = backend.host.clone();
    match effect {
        SketchEffect::CreateSketch { command, .. } => {
            let receipt = api::apply_edit_command_with_completion(&backend.state, host, *command)
                .await
                .map_err(|e| e.to_string())?;
            complete(backend, receipt).await?;
            Ok(Reply::Created)
        }
        SketchEffect::EnterSketch { created_here, .. } => {
            let receipt =
                api::enter_sketch_with_completion(&backend.state, host, id, Some(created_here))
                    .await
                    .map_err(|e| e.to_string())?;
            complete(backend, receipt)
                .await
                .map(Box::new)
                .map(|solve| Reply::Entered(solve, false))
        }
        SketchEffect::RefreshSketch { .. } => {
            let solve = api::get_sketch(&backend.state, id)
                .await
                .map_err(|e| e.to_string())?;
            check_context(&backend.state, &tag.document).await?;
            Ok(Reply::Entered(Box::new(solve), true))
        }
        SketchEffect::Upsert { ops, .. } => {
            let receipt = match api::author_sketch_primitive_controlled(
                &backend.state,
                host,
                id,
                ops,
                control,
            )
            .await
            {
                Ok(receipt) => receipt,
                Err(error) => return candidate_refusal(error),
            };
            admission_reply(backend, receipt, tag, actions).await
        }
        SketchEffect::History {
            edit_session,
            direction,
            ..
        } => {
            let scope = onecad_app::dto::SketchHistoryScope {
                sketch_id: tag.key.sketch_id,
                edit_session,
                runtime_session: tag.document.runtime_session.clone(),
                revision: tag.document.revision,
                snapshot_id: tag.document.snapshot_id,
            };
            let receipt =
                api::sketch_history_with_completion(&backend.state, host, scope, direction)
                    .await
                    .map_err(|error| error.to_string())?;
            complete(backend, receipt).await.map(Reply::History)
        }
        SketchEffect::AddConstraints { constraints, .. } => {
            let receipt = match api::add_sketch_constraints_controlled(
                &backend.state,
                host,
                id,
                constraints,
                control,
            )
            .await
            {
                Ok(receipt) => receipt,
                Err(error) => return candidate_refusal(error),
            };
            admission_reply(backend, receipt, tag, actions).await
        }
        SketchEffect::SetDimension {
            constraint, value, ..
        } => {
            let receipt = match api::set_sketch_dimension_controlled(
                &backend.state,
                host,
                id,
                constraint,
                value,
                control,
            )
            .await
            {
                Ok(receipt) => receipt,
                Err(error) => return candidate_refusal(error),
            };
            admission_reply(backend, receipt, tag, actions).await
        }
        SketchEffect::Repair { ops, .. } => {
            let receipt =
                match api::repair_sketch_with_completion(&backend.state, host, id, ops).await {
                    Ok(receipt) => receipt,
                    Err(error) => return candidate_refusal(error),
                };
            admission_reply(backend, receipt, tag, actions).await
        }
        SketchEffect::FinishSketch { .. } => finish(backend, id).await,
        SketchEffect::DiscardSketch { .. } => {
            let receipt = api::cancel_sketch_with_completion(&backend.state, host, id, Some(true))
                .await
                .map_err(|e| e.to_string())?;
            complete(backend, receipt).await.map(Reply::Discarded)
        }
    }
}
fn candidate_refusal(error: onecad_app::error::ApiError) -> Result<Reply, String> {
    match error {
        onecad_app::error::ApiError::InvalidCommand(_)
        | onecad_app::error::ApiError::OpFailed { .. } => {
            Ok(Reply::ConstraintRefused(error.to_string()))
        }
        _ => Err(error.to_string()),
    }
}

async fn admission_reply(
    backend: &mut Backend,
    receipt: api::CommandCompletion<onecad_app::dto::SketchConstraintAdmission>,
    tag: SketchTag,
    actions: &mut Vec<SketchAction>,
) -> Result<Reply, String> {
    if receipt
        .result
        .as_ref()
        .is_ok_and(|admission| admission.accepted)
    {
        actions.push(SketchAction::BatchAccepted { tag });
    }
    let admission = complete(backend, receipt).await?;
    if admission.accepted {
        Ok(Reply::Solved(Box::new(admission.solve)))
    } else {
        Ok(Reply::ConstraintRefused(format!(
            "Constraint refused: {:?}",
            admission.solve.status
        )))
    }
}
async fn finish(backend: &mut Backend, id: String) -> Result<Reply, String> {
    let receipt = api::cancel_sketch_with_completion(
        &backend.state,
        backend.host.clone(),
        id.clone(),
        Some(false),
    )
    .await
    .map_err(|e| e.to_string())?;
    complete(backend, receipt).await?;
    let before = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    backend.events.begin(crate::event_mailbox::Expected::Known {
        session: before.runtime_session,
        revision: before.revision + 1,
    });
    let receipt =
        match api::finish_sketch_with_completion(&backend.state, backend.host.clone(), id).await {
            Ok(receipt) => receipt,
            Err(error) => return finish_error(error),
        };
    let diagnostics = match &receipt.result {
        Err(onecad_app::error::ApiError::OpFailed { diagnostics, .. }) => diagnostics.clone(),
        _ => Vec::new(),
    };
    match complete(backend, receipt).await {
        Ok(result) => Ok(Reply::Finished(Box::new(result))),
        // The runtime commits the sketch record before deriving regions, so an
        // invalid profile must not trap the user in sketch mode: close and warn.
        Err(_) if profile_refusal(&diagnostics) => Ok(Reply::Finished(Box::new(
            onecad_app::dto::FinishSketchDto {
                region_identity_version: 0,
                regions: Vec::new(),
                diagnostics,
            },
        ))),
        Err(message) if !diagnostics.is_empty() => Ok(Reply::FinishRefused {
            message,
            diagnostics,
        }),
        Err(message) => Err(message),
    }
}
fn profile_refusal(diagnostics: &[onecad_core::regen::Diagnostic]) -> bool {
    !diagnostics.is_empty()
        && diagnostics
            .iter()
            .any(|diagnostic| diagnostic.stage.as_deref() == Some("profile"))
}

fn finish_error(error: onecad_app::error::ApiError) -> Result<Reply, String> {
    match error {
        onecad_app::error::ApiError::OpFailed {
            diagnostics,
            message,
        } => Ok(Reply::FinishRefused {
            message: format!("operation failed: {message}"),
            diagnostics,
        }),
        error => Err(error.to_string()),
    }
}

async fn complete<T: serde::Serialize>(
    backend: &mut Backend,
    receipt: api::CommandCompletion<T>,
) -> Result<T, String> {
    if receipt.regen_expected {
        backend.events.retarget(
            &receipt.projection.runtime_session,
            receipt.projection.revision,
        );
    }
    let session = receipt.projection.runtime_session.clone();
    let revision = receipt.projection.revision;
    crate::sketch_completion::finish(
        receipt,
        crate::services::wait_regen(backend, &session, revision),
    )
    .await
}

async fn feedback(
    backend: &Backend,
    tag: Option<&SketchTag>,
    reply: Reply,
    context: DocumentContext,
) -> Result<SketchAction, String> {
    check_context(&backend.state, &context).await?;
    if let Reply::BeginCreated(sketch) = reply {
        return Ok(SketchAction::BeginCreated { sketch, context });
    }
    if let Reply::Begin(sketch) = reply {
        return Ok(SketchAction::Begin {
            sketch,
            create: false,
            context,
        });
    }
    let tag = tag.ok_or("missing sketch request scope")?.clone();
    if let Reply::ConstraintRefused(message) = reply {
        return Ok(SketchAction::ConstraintRefused { tag, message });
    }
    let snapshot = match &reply {
        Reply::Entered(..) | Reply::Solved(..) | Reply::History(..) => {
            let sketch = api::get_sketch_snapshot(&backend.state, tag.key.sketch_id)
                .await
                .map_err(|e| e.to_string())?;
            check_context(&backend.state, &context).await?;
            crate::record_budget::check_payload(&sketch)?;
            let history = api::get_sketch(&backend.state, tag.key.sketch_id.to_string())
                .await
                .map_err(|error| error.to_string())?
                .history;
            check_context(&backend.state, &context).await?;
            Some(SketchSnapshot {
                history,
                document: context.clone(),
                sketch: Box::new(sketch),
            })
        }
        _ => None,
    };
    Ok(match reply {
        Reply::Created => SketchAction::Created { tag },
        Reply::Entered(solve, refresh) => {
            let snapshot = snapshot.ok_or("missing authoritative sketch snapshot")?;
            if refresh {
                SketchAction::Refreshed {
                    tag,
                    snapshot,
                    solve,
                }
            } else {
                SketchAction::Entered {
                    tag,
                    snapshot,
                    solve,
                }
            }
        }
        Reply::Solved(solve) => SketchAction::Solved {
            tag,
            snapshot: snapshot.ok_or("missing solved snapshot")?,
            solve,
        },
        Reply::History(result) => {
            let solve = api::get_sketch(&backend.state, tag.key.sketch_id.to_string())
                .await
                .map_err(|error| error.to_string())?;
            check_context(&backend.state, &context).await?;
            SketchAction::HistoryResolved {
                tag,
                snapshot: snapshot.ok_or("missing authoritative sketch history snapshot")?,
                solve: Box::new(solve),
                moved: result.moved,
                direction: result.direction,
            }
        }
        Reply::Finished(result) => SketchAction::Finished { tag, result },
        Reply::FinishRefused {
            message,
            diagnostics,
        } => SketchAction::FinishRefused {
            tag,
            message,
            diagnostics,
        },
        Reply::Discarded(result) => SketchAction::Discarded { tag, result },
        Reply::BeginCreated(_) | Reply::Begin(_) | Reply::ConstraintRefused(_) => {
            return Err("unexpected sketch load reply".into());
        }
    })
}

pub fn effect_tag(effect: &SketchEffect) -> &SketchTag {
    match effect {
        SketchEffect::CreateSketch { tag, .. }
        | SketchEffect::EnterSketch { tag, .. }
        | SketchEffect::RefreshSketch { tag }
        | SketchEffect::Upsert { tag, .. }
        | SketchEffect::AddConstraints { tag, .. }
        | SketchEffect::SetDimension { tag, .. }
        | SketchEffect::Repair { tag, .. }
        | SketchEffect::History { tag, .. }
        | SketchEffect::FinishSketch { tag, .. }
        | SketchEffect::DiscardSketch { tag } => tag,
    }
}
pub fn document_context(display: &Display) -> DocumentContext {
    DocumentContext {
        document_id: display.context.document_id.clone(),
        runtime_session: display.context.runtime_session.clone(),
        revision: display.context.revision.0,
        snapshot_id: display.context.snapshot_id,
        scene: display.scene.identity(),
    }
}
async fn check_context(
    state: &onecad_app::state::AppState,
    expected: &DocumentContext,
) -> Result<(), String> {
    let actual = api::preview_context(state)
        .await
        .map_err(|e| e.to_string())?;
    if actual.document_id != expected.document_id
        || actual.runtime_session != expected.runtime_session
        || actual.revision.0 != expected.revision
        || actual.snapshot_id != expected.snapshot_id
    {
        return Err("Sketch request belongs to an obsolete document/session/revision/head".into());
    }
    Ok(())
}

#[cfg(test)]
mod stale_tests {
    use super::*;
    use onecad_core::{
        ids::SnapshotId,
        sketch::{Sketch, SketchAttachment, WorldPlane},
    };
    use onecad_interaction::sketch::{SketchInteraction, SketchPhase};
    use onecad_scene::SceneIdentity;
    fn context(session: &str) -> DocumentContext {
        DocumentContext {
            document_id: session.into(),
            runtime_session: session.into(),
            revision: 1,
            snapshot_id: SnapshotId(1),
            scene: SceneIdentity {
                session: 1,
                scene: 1,
            },
        }
    }
    fn begin(interaction: &mut SketchInteraction, context: DocumentContext) -> SketchTag {
        interaction.dispatch(SketchAction::ContextChanged(Some(context.clone())));
        let sketch = Sketch::new(
            SketchId::new(),
            "Sketch",
            SketchAttachment::World {
                plane: WorldPlane::XY,
            },
        );
        let effects = interaction.dispatch(SketchAction::Begin {
            sketch: Box::new(sketch),
            create: true,
            context,
        });
        effect_tag(&effects[0]).clone()
    }
    #[test]
    fn stale_reply_keeps_original_scope_and_cannot_fault_new_session() {
        let mut interaction = SketchInteraction::default();
        let old = begin(&mut interaction, context("old"));
        let current = begin(&mut interaction, context("new"));
        let delivery = Delivery {
            tag: Some(old.clone()),
            display: None,
            actions: vec![SketchAction::BatchAccepted { tag: old }],
            error: None,
            _lease: None,
        };
        for action in delivery.stale_actions() {
            interaction.dispatch(action);
        }
        let session = interaction.session.unwrap();
        assert_eq!(session.request, current);
        assert_eq!(session.phase, SketchPhase::Creating);
    }
}
