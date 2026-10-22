use super::*;
use onecad_interaction::{
    Action,
    sketch::{SketchAction, gesture::GestureAction},
};

pub(crate) fn model(action: &Action) -> Value {
    let name = match action {
        Action::ContextChanged(_) => "ContextChanged",
        Action::SelectProfile(_) => "SelectProfile",
        Action::ClearSelection => "ClearSelection",
        Action::BeginExtrude { .. } => "BeginExtrude",
        Action::BeginOperation { .. } => "BeginOperation",
        Action::BeginEdit { .. } => "BeginEdit",
        Action::BeginParameterEdit { .. } => "BeginParameterEdit",
        Action::DraftChanged(_) => "DraftChanged",
        Action::ParameterChanged { .. } => "ParameterChanged",
        Action::FocusParameter(_) => "FocusParameter",
        Action::ParameterDrag(_) => "ParameterDrag",
        Action::ExtrudeModeChanged(_) => "ExtrudeModeChanged",
        Action::ExtrudeTargetChanged { .. } => "ExtrudeTargetChanged",
        Action::FlipExtrude => "FlipExtrude",
        Action::TransformDraftChanged { .. } => "TransformDraftChanged",
        Action::Evaluated { .. } => "Evaluated",
        Action::PreviewFinished { .. } => "PreviewFinished",
        Action::Confirm => "Confirm",
        Action::Cancel => "Cancel",
        Action::Republish => "Republish",
        Action::CommitAccepted { .. } => "CommitAccepted",
        Action::CommitRejected { .. } => "CommitRejected",
        Action::CompletionUnavailable { .. } => "CompletionUnavailable",
        Action::RegenFinished { .. } => "RegenFinished",
    };
    json!({"action": name})
}

pub(crate) fn sketch(action: &SketchAction) -> Value {
    let name = match action {
        SketchAction::ContextChanged(_) => "ContextChanged",
        SketchAction::Begin { .. } => "Begin",
        SketchAction::BeginCreated { .. } => "BeginCreated",
        SketchAction::Created { .. } => "Created",
        SketchAction::Entered { .. } => "Entered",
        SketchAction::Refreshed { .. } => "Refreshed",
        SketchAction::SetTool(_) => "SetTool",
        SketchAction::SetConstructionMode { .. } => "SetConstructionMode",
        SketchAction::SetTangentArc { .. } => "SetTangentArc",
        SketchAction::Move(_) => "Move",
        SketchAction::Click { .. } => "Click",
        SketchAction::ClickInferred { .. } => "ClickInferred",
        SketchAction::CommitStroke { .. } => "CommitStroke",
        SketchAction::BeginPointNumeric { .. } => "BeginPointNumeric",
        SketchAction::PreviewNumeric { .. } => "PreviewNumeric",
        SketchAction::CreateNumeric { .. } => "CreateNumeric",
        SketchAction::CreateNumericAuthored { .. } => "CreateNumericAuthored",
        SketchAction::Escape => "Escape",
        SketchAction::Enter => "Enter",
        SketchAction::Finish => "Finish",
        SketchAction::Discard => "Discard",
        SketchAction::AcceptView { .. } => "AcceptView",
        SketchAction::SelectionDrag { .. } => "SelectionDrag",
        SketchAction::Select { .. } => "Select",
        SketchAction::AddConstraints { .. } => "AddConstraints",
        SketchAction::SetDimension { .. } => "SetDimension",
        SketchAction::Repair { .. } => "Repair",
        SketchAction::History(_) => "History",
        SketchAction::HistoryResolved { .. } => "HistoryResolved",
        SketchAction::ConstraintRefused { .. } => "ConstraintRefused",
        SketchAction::BatchAccepted { .. } => "BatchAccepted",
        SketchAction::Solved { .. } => "Solved",
        SketchAction::Failed { .. } => "Failed",
        SketchAction::ReenterForRepair { .. } => "ReenterForRepair",
        SketchAction::FinishRefused { .. } => "FinishRefused",
        SketchAction::Finished { .. } => "Finished",
        SketchAction::Discarded { .. } => "Discarded",
    };
    let mut detail = json!({"action": name});
    match action {
        SketchAction::Move(point)
        | SketchAction::Click { point, .. }
        | SketchAction::ClickInferred { point, .. }
        | SketchAction::CommitStroke { point, .. } => {
            detail["point"] = json!([point.point.x, point.point.y]);
            detail["capture_view"] = json!(point.capture.view);
        }
        SketchAction::Select { targets, .. } => detail["targets"] = json!(targets.len()),
        SketchAction::SetTool(tool) => detail["tool"] = json!(format!("{tool:?}")),
        _ => {}
    }
    detail
}

pub(crate) fn gesture(action: &GestureAction) -> Value {
    let (name, tag) = match action {
        GestureAction::Begin { scope, grab, .. } => {
            return json!({"action": "Begin",
            "capture_view": scope.capture.view, "point": [grab.x, grab.y]});
        }
        GestureAction::Move { tag, .. } => ("Move", Some(tag)),
        GestureAction::End { tag, .. } => ("End", Some(tag)),
        GestureAction::Cancel { tag } => ("Cancel", Some(tag)),
        GestureAction::Ready { tag, .. } => ("Ready", Some(tag)),
        GestureAction::Updated { tag, .. } => ("Updated", Some(tag)),
        GestureAction::Completed { tag, .. } => ("Completed", Some(tag)),
        GestureAction::Cancelled { tag } => ("Cancelled", Some(tag)),
        GestureAction::Failed { tag, .. } => ("Failed", Some(tag)),
        GestureAction::Sync => ("Sync", None),
    };
    let mut detail = json!({"action": name});
    if let Some(tag) = tag {
        detail["gesture"] = json!(tag.gesture);
        detail["sequence"] = json!(tag.sequence);
        detail["capture_view"] = json!(tag.scope.capture.view);
    }
    match action {
        GestureAction::Move { point, .. } | GestureAction::End { point, .. } => {
            detail["point"] = json!([point.x, point.y])
        }
        GestureAction::Failed { message, .. } => detail["refusal"] = json!(bounded_id(message)),
        _ => {}
    }
    detail
}

pub(crate) fn message(message: &crate::bridge::Message) -> Value {
    use crate::bridge::Message;
    match message {
        Message::Gesture { delivery, .. } => {
            json!({"type": "Gesture", "gesture": delivery.tag.gesture,
            "sequence": delivery.tag.sequence, "capture_view": delivery.tag.scope.capture.view,
            "accepted": delivery.accepted, "revision": delivery.accepted_revision,
            "error": delivery.error.as_deref().map(bounded_id)})
        }
        Message::Sketch { delivery, .. } => {
            json!({"type": "Sketch", "sequence": delivery.tag.as_ref().map(|tag| tag.sequence),
            "error": delivery.error.as_deref().map(bounded_id)})
        }
        Message::Document { result, .. } => json!({"type": "Document", "ok": result.is_ok()}),
        Message::Items {
            sequence, result, ..
        } => json!({"type": "Items", "sequence": sequence, "ok": result.is_ok()}),
        Message::StartupFailed(error) => {
            json!({"type": "StartupFailed", "error": bounded_id(error)})
        }
        Message::Stopped => json!({"type": "Stopped"}),
        Message::Prepared { result, .. } => json!({"type": "Prepared", "ok": result.is_ok()}),
        Message::Changes { changes, .. } => json!({"type": "Changes", "count": changes.count}),
        Message::Accepted { revision, tag, .. } => {
            json!({"type": "Accepted", "revision": revision, "gesture": tag.gesture, "sequence": tag.sequence, "runtime_session": bounded_id(&tag.context.runtime_session)})
        }
        Message::Terminal {
            command,
            result,
            tag,
            ..
        } => {
            json!({"type": "Terminal", "command": command, "ok": result.is_ok(), "gesture": tag.as_ref().map(|tag| tag.gesture), "sequence": tag.as_ref().map(|tag| tag.sequence)})
        }
    }
}
