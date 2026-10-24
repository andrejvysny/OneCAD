//! Explicit single-input repair. Only this service can mint a staged repair token.
use super::edit_preview::{ensure_context, ensure_runtime_fence, CancelOnDrop};
use super::{stage_elements, ApplyOperationResponse, EditPreview, PreviewContext, StagePick};
use crate::document_runtime::{
    edit_preview::{drive_edit_preview, EditPreviewPrep},
    DocumentRuntime,
};
use crate::{
    error::ApiError,
    host::{AppEvent, HostContext},
    state::AppState,
};
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord},
        refs::{ElementKind, ElementRef},
    },
    edit::{EditCommand, InputPath, InputRef},
    ids::{BodyId, RecordId},
    regen::Lod,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct InputRepairTarget {
    pub body: BodyId,
    pub kind: ElementKind,
    path: InputPath,
}

/// Immutable, non-serializable evidence captured from the current healthy head.
#[derive(Clone, Debug)]
pub struct InputRepair {
    fence: PreviewContext,
    original: OperationRecord,
    target: InputRepairTarget,
    reference: ElementRef,
    preview_ready: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}
impl InputRepair {
    pub fn record(&self) -> &OperationRecord {
        &self.original
    }
    pub fn fence(&self) -> &PreviewContext {
        &self.fence
    }
    /// Revokes only this transient repair; the document and undo history are untouched.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.preview_ready.store(false, Ordering::Release);
    }
    fn command(&self) -> EditCommand {
        EditCommand::EditOperationInput {
            record: self.original.record_id,
            path: self.target.path.clone(),
            reference: InputRef::Element(self.reference.clone()),
        }
    }
}

/// Native eligibility only; the server also checks the authoritative repair gate.
pub fn input_repair_target(record: &OperationRecord) -> Result<InputRepairTarget, String> {
    let unsupported = "Repair supports one edge in Fillet/equal-leg Chamfer or one face in Shell/Offset Face. Multi-input, asymmetric, whole-body, Hole and sketch repairs require a separate repair workflow.";
    let (reference, kind, path) = match &record.op {
        Operation::Known(KnownOperation::Fillet(p))
            if p.edge_ids.len() == 1 && p.edges.len() == 1 =>
        {
            (
                &p.edges[0],
                ElementKind::Edge,
                InputPath::FilletEdges { index: 0 },
            )
        }
        Operation::Known(KnownOperation::Chamfer(p))
            if !p.is_asymmetric()
                && p.edge_ids.len() == 1
                && p.edges.len() == 1
                && p.reference_faces.is_empty()
                && p.reference_face_refs.is_empty() =>
        {
            (
                &p.edges[0],
                ElementKind::Edge,
                InputPath::FilletEdges { index: 0 },
            )
        }
        Operation::Known(KnownOperation::Shell(p))
            if p.open_faces.len() == 1 && p.faces.len() == 1 =>
        {
            (
                &p.faces[0],
                ElementKind::Face,
                InputPath::ShellOpenFaces { index: 0 },
            )
        }
        Operation::Known(KnownOperation::OffsetFace(p))
            if p.face_ids.len() == 1
                && p.faces.len() == 1
                && p.opposite_face.is_none()
                && p.opposite_face_id.is_none() =>
        {
            (
                &p.faces[0],
                ElementKind::Face,
                InputPath::OffsetFaceFace { index: 0 },
            )
        }
        _ => return Err(unsupported.into()),
    };
    let primary = reference
        .primary
        .as_ref()
        .filter(|p| p.kind == kind)
        .ok_or(unsupported)?;
    if record.suppressed {
        return Err("Suppressed features cannot be repaired".into());
    }
    Ok(InputRepairTarget {
        body: primary.body,
        kind,
        path,
    })
}

fn checked_record(
    runtime: &DocumentRuntime,
    record: RecordId,
) -> Result<OperationRecord, ApiError> {
    if runtime.is_read_only() {
        return Err(invalid("Repair document is read-only"));
    }
    let first = runtime
        .repair_items()
        .iter()
        .map(|item| item.step_index)
        .min()
        .ok_or_else(|| invalid("There is no halted repair step"))?;
    let projection = runtime.projection();
    if projection
        .features
        .get(first)
        .is_none_or(|feature| feature.id != record.to_string())
        || runtime
            .repair_items()
            .iter()
            .filter(|item| item.step_index == first)
            .count()
            != 1
    {
        return Err(invalid(
            "Repair requires the first halted step with exactly one unresolved input",
        ));
    }
    let original = runtime
        .operation_record(record)
        .ok_or_else(|| invalid("Repair record is absent"))?;
    let target = input_repair_target(&original).map_err(invalid)?;
    if !runtime.head_body_ids().contains(&target.body) {
        return Err(invalid(
            "Repair target body is absent from the healthy partial head",
        ));
    }
    Ok(original)
}

fn validate(runtime: &DocumentRuntime, repair: &InputRepair) -> Result<(), ApiError> {
    ensure_runtime_fence(runtime, &repair.fence)?;
    if repair.cancelled.load(Ordering::Acquire) {
        return Err(ApiError::Cancelled("Repair was cancelled".into()));
    }
    if checked_record(runtime, repair.original.record_id)? != repair.original {
        return Err(invalid("Repair record changed after staging"));
    }
    let primary = repair
        .reference
        .primary
        .as_ref()
        .ok_or_else(|| invalid("Missing staged primary"))?;
    if primary.body != repair.target.body || primary.kind != repair.target.kind {
        return Err(invalid(
            "Replacement body or kind differs from the repair input",
        ));
    }
    Ok(())
}

pub async fn stage_input_repair(
    state: &AppState,
    fence: PreviewContext,
    record: RecordId,
    pick: StagePick,
) -> Result<InputRepair, ApiError> {
    ensure_context(state, &fence).await?;
    let original = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| invalid("No repair document"))?;
        ensure_runtime_fence(runtime, &fence)?;
        checked_record(runtime, record)?
    };
    let target = input_repair_target(&original).map_err(invalid)?;
    if pick.body != target.body || pick.kind != target.kind {
        return Err(invalid(
            "Select exactly one replacement of the expected body and kind",
        ));
    }
    let reference = stage_replacement(state, &fence, &original, pick).await?;
    let repair = InputRepair {
        fence,
        original,
        target,
        reference,
        preview_ready: Arc::new(AtomicBool::new(false)),
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    ensure_context(state, &repair.fence).await?;
    let guard = state.runtime.lock().await;
    validate(
        guard
            .as_ref()
            .ok_or_else(|| invalid("No repair document"))?,
        &repair,
    )?;
    Ok(repair)
}

pub async fn preview_input_repair(
    state: &AppState,
    repair: &InputRepair,
    lod: Lod,
) -> Result<EditPreview, ApiError> {
    repair.preview_ready.store(false, Ordering::Release);
    ensure_context(state, &repair.fence).await?;
    let job = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| invalid("No repair document"))?;
        validate(runtime, repair)?;
        match runtime.plan_input_repair_preview(repair.original.record_id, repair.command(), lod)? {
            EditPreviewPrep::Done(preview) => {
                return Ok(EditPreview {
                    snapshot_id: repair.fence.snapshot_id,
                    ..*preview
                })
            }
            EditPreviewPrep::Run(job) => job,
        }
    };
    let _abandon = CancelOnDrop(job.cancel_token());
    let run = drive_edit_preview(&job, state.preview().as_ref()).await;
    let preview = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| invalid("No repair document"))?;
        validate(runtime, repair)?;
        let mut preview = runtime.finish_edit_preview(*job, run)?;
        if preview.outcome == super::EditPreviewOutcome::Ready {
            if let Some(step) = runtime
                .repair_items()
                .iter()
                .map(|item| item.step_index)
                .filter(|step| preview.edited_step.is_some_and(|edited| *step > edited))
                .min()
            {
                let projection = runtime.projection();
                if let Some(record) = projection
                    .features
                    .get(step)
                    .and_then(|feature| feature.id.parse().ok())
                {
                    preview.outcome =
                        super::EditPreviewOutcome::DownstreamNeedsRepair { step, record };
                }
            }
        }
        preview
    };
    ensure_context(state, &repair.fence).await?;
    repair
        .preview_ready
        .store(repair_preview_ready(&preview), Ordering::Release);
    Ok(preview)
}

pub async fn commit_input_repair(
    state: &AppState,
    app: HostContext,
    repair: &InputRepair,
) -> Result<ApplyOperationResponse, ApiError> {
    ensure_context(state, &repair.fence).await?;
    if !repair.preview_ready.load(Ordering::Acquire) {
        return Err(invalid("Repair requires a successful current preview"));
    }
    let (outcome, projection, rollback_token) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| invalid("No repair document"))?;
        validate(runtime, repair)?;
        let prior = runtime.undo_position();
        let outcome = runtime.apply_transaction("Repair input", vec![repair.command()])?;
        let rollback_token = runtime.arm_operation_rollback(vec![repair.original.record_id], prior);
        (outcome, runtime.projection(), rollback_token)
    };
    let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    if let Some(scheduler) = state.scheduler.get() {
        scheduler.handle(&outcome);
    }
    state.note_mutation();
    Ok(ApplyOperationResponse {
        projection,
        rollback_token,
    })
}
fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::InvalidCommand(message.into())
}

/// A later repair gate is preserved; earlier failures and isolated replay are refused.
pub fn repair_preview_ready(preview: &EditPreview) -> bool {
    !preview.isolated
        && preview.edited_step.is_some_and(|edited| {
            preview
                .step_signatures
                .iter()
                .any(|(step, _)| *step == edited)
                && match preview.outcome {
                    super::EditPreviewOutcome::Ready => true,
                    super::EditPreviewOutcome::DownstreamNeedsRepair { step, .. } => step > edited,
                    _ => false,
                }
        })
}

async fn stage_replacement(
    state: &AppState,
    fence: &PreviewContext,
    original: &OperationRecord,
    pick: StagePick,
) -> Result<ElementRef, ApiError> {
    let body_id = Some(crate::worker::wire::body_id_wire(pick.body));
    let (topo_key, element_id) = match &pick.address {
        super::StageAddress::TopoKey(key) => (Some(key.clone()), None),
        super::StageAddress::Element(id) => (None, Some(id.clone())),
    };
    let mut refs = match &original.op {
        Operation::Known(KnownOperation::Fillet(p)) => {
            super::stage_edge_op(
                state,
                fence,
                "Fillet",
                vec![super::EdgeOpPickInput {
                    body_id,
                    topo_key,
                    element_id,
                }],
                p.chain_tangent_edges,
            )
            .await?
            .refs
        }
        Operation::Known(KnownOperation::Chamfer(p)) => {
            super::stage_edge_op(
                state,
                fence,
                "Chamfer",
                vec![super::EdgeOpPickInput {
                    body_id,
                    topo_key,
                    element_id,
                }],
                p.chain_tangent_edges,
            )
            .await?
            .refs
        }
        Operation::Known(KnownOperation::OffsetFace(p)) => {
            super::stage_offset_face(
                state,
                fence,
                vec![super::OffsetFacePickInput {
                    body_id,
                    topo_key,
                    element_id,
                }],
                p.chain_tangent_faces,
            )
            .await?
            .refs
        }
        _ => stage_elements(state, fence, vec![pick]).await?,
    };
    if refs.len() != 1 {
        return Err(invalid("Replacement tangent closure must contain exactly one input; multi-input repair is unsupported"));
    }
    Ok(refs.remove(0))
}

#[cfg(test)]
#[path = "input_repair_tests.rs"]
mod tests;
