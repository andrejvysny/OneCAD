use super::*;
use onecad_app::dto::{NativeGestureScope, NativeGestureSource};
use onecad_app::worker::wire::{DragKind, GestureTarget};
use onecad_core::sketch::CurvePosition;
use onecad_interaction::sketch::gesture::{GestureIntent, GestureKind};

pub(super) fn native(
    tag: &GestureTag,
    expected: &api::PreviewContext,
) -> Result<NativeGestureScope, String> {
    let scope = &tag.scope;
    if scope.key.document_id != scope.document.document_id
        || scope.key.runtime_session != scope.document.runtime_session
        || scope.capture.scene != scope.document.scene
        || expected.document_id != scope.document.document_id
        || expected.runtime_session != scope.document.runtime_session
        || expected.revision.0 != scope.document.revision
        || expected.snapshot_id != scope.document.snapshot_id
    {
        return Err("Gesture belongs to a stale document, visit or captured view".into());
    }
    Ok(NativeGestureScope {
        document_id: expected
            .document_id
            .parse()
            .map_err(|error| format!("Gesture document identity: {error}"))?,
        runtime_session: expected.runtime_session.clone(),
        sketch_id: scope.key.sketch_id,
        edit_visit: scope
            .edit_session
            .parse()
            .map_err(|error| format!("Gesture edit visit: {error}"))?,
        document_revision: expected.revision,
        worker_epoch: expected.worker_epoch,
        snapshot: Some(expected.snapshot_id),
        expected_base_hash: expected.expected_base_hash.clone(),
        source: NativeGestureSource {
            capture: scope.capture.view,
            generation: scope.key.generation,
        },
    })
}

pub(super) fn target(intent: &GestureIntent) -> Result<GestureTarget, String> {
    let kind = match intent.kind {
        GestureKind::Point => DragKind::Point,
        GestureKind::ArcEnd => DragKind::ArcEnd,
        GestureKind::Radius => DragKind::Radius,
        GestureKind::EntityBody => DragKind::EntityBody,
    };
    let role = if intent.kind == GestureKind::ArcEnd {
        match intent.role {
            Some(CurvePosition::Start) => Some("start"),
            Some(CurvePosition::End) => Some("end"),
            _ => return Err("Arc endpoint gesture requires its start or end role".into()),
        }
    } else {
        None
    };
    Ok(GestureTarget {
        kind,
        entity: intent.entity,
        role,
        grab: Some([intent.grab.x, intent.grab.y]),
    })
}

pub(super) fn owns(left: &GestureTag, right: &GestureTag) -> bool {
    left.scope == right.scope && left.gesture == right.gesture
}

pub(super) fn receipt<T>(
    handle: &api::NativeGestureHandle,
    value: &onecad_app::dto::NativeGestureReceipt<T>,
) -> Result<(), String> {
    if &value.scope != handle.scope() || value.gesture_id != handle.gesture_id() {
        return Err("Gesture receipt belongs to another document, visit or worker gesture".into());
    }
    Ok(())
}
