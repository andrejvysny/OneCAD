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
    Load {
        id: SketchId,
        context: DocumentContext,
    },
    Effect(SketchEffect),
}
pub struct Delivery {
    pub tag: Option<SketchTag>,
    pub display: Option<Arc<Display>>,
    pub actions: Vec<SketchAction>,
    pub error: Option<String>,
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
    Begin(Box<onecad_core::sketch::Sketch>),
    Created,
    Entered(Box<SketchSessionDto>, bool),
    Solved(Box<SketchUpsertDto>),
    Finished(Box<onecad_app::dto::FinishSketchDto>),
    Discarded(onecad_app::dto::CancelSketchDto),
}

pub async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let tag = match &request {
        Request::Effect(effect) => Some(effect_tag(effect).clone()),
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
        Request::Load { id, context } => {
            check_context(state, &context).await?;
            let sketch = api::get_sketch_snapshot(state, id)
                .await
                .map_err(|e| e.to_string())?;
            check_context(state, &context).await?;
            crate::record_budget::check_payload(&sketch)?;
            Ok(Reply::Begin(Box::new(sketch)))
        }
        Request::Effect(effect) => execute_effect(backend, effect, actions).await,
    }
}

async fn execute_effect(
    backend: &mut Backend,
    effect: SketchEffect,
    actions: &mut Vec<SketchAction>,
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
            let receipt = api::sketch_upsert_with_completion(&backend.state, host, id, ops)
                .await
                .map_err(|e| e.to_string())?;
            if receipt.result.is_ok() {
                actions.push(SketchAction::BatchAccepted { tag });
            }
            complete(backend, receipt)
                .await
                .map(Box::new)
                .map(Reply::Solved)
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
    let receipt = api::finish_sketch_with_completion(&backend.state, backend.host.clone(), id)
        .await
        .map_err(|e| e.to_string())?;
    complete(backend, receipt)
        .await
        .map(Box::new)
        .map(Reply::Finished)
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
    if let Reply::Begin(sketch) = reply {
        return Ok(SketchAction::Begin {
            sketch,
            create: false,
            context,
        });
    }
    let tag = tag.ok_or("missing sketch request scope")?.clone();
    let snapshot = match &reply {
        Reply::Entered(..) | Reply::Solved(..) => {
            let sketch = api::get_sketch_snapshot(&backend.state, tag.key.sketch_id)
                .await
                .map_err(|e| e.to_string())?;
            check_context(&backend.state, &context).await?;
            crate::record_budget::check_payload(&sketch)?;
            Some(SketchSnapshot {
                document: context,
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
        Reply::Finished(result) => SketchAction::Finished { tag, result },
        Reply::Discarded(result) => SketchAction::Discarded { tag, result },
        Reply::Begin(_) => return Err("unexpected sketch load reply".into()),
    })
}

pub fn effect_tag(effect: &SketchEffect) -> &SketchTag {
    match effect {
        SketchEffect::CreateSketch { tag, .. }
        | SketchEffect::EnterSketch { tag, .. }
        | SketchEffect::RefreshSketch { tag }
        | SketchEffect::Upsert { tag, .. }
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
        };
        for action in delivery.stale_actions() {
            interaction.dispatch(action);
        }
        let session = interaction.session.unwrap();
        assert_eq!(session.request, current);
        assert_eq!(session.phase, SketchPhase::Creating);
    }
}
