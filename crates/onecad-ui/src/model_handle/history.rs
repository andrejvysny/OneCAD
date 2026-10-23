//! Original operands have their own read lifetime, independent of draft sequences.
use super::{ModelHandle, operands::Operands};
use crate::{app::App, bridge::Command};
use onecad_app::api::{EditOperandContext, EditOperandRead};
use onecad_interaction::RequestTag;
use std::sync::mpsc::{self, TryRecvError};

pub(super) struct History {
    tag: RequestTag,
    signature: serde_json::Value,
    receiver: mpsc::Receiver<Result<EditOperandRead, String>>,
    read: Option<EditOperandRead>,
    context: Option<EditOperandContext>,
    failed: bool,
    operand_changed: bool,
}
impl History {
    fn current(&self, tag: &RequestTag) -> bool {
        self.tag.record_id == tag.record_id
            && self.tag.gesture == tag.gesture
            && self.tag.context == tag.context
    }
}

pub(crate) fn poll(app: &mut App, context: &egui::Context) {
    let Some(tag) = current_tag(app) else {
        app.doc.model_handle.history = None;
        return;
    };
    if app
        .doc
        .model_handle
        .history
        .as_ref()
        .is_none_or(|h| !h.current(&tag))
    {
        begin(app, tag);
    }
    check_operand_changes(app);
    let Some(history) = app
        .doc
        .model_handle
        .history
        .as_mut()
        .filter(|h| !h.failed && h.context.is_none())
    else {
        return;
    };
    match receive(history) {
        Some(Ok(context)) => publish(app, context),
        Some(Err(reason)) => {
            history.failed = true;
            app.doc.status = format!(
                "Historical handles unavailable: {reason}. Numeric editing remains available."
            );
        }
        None => context.request_repaint_after(std::time::Duration::from_millis(10)),
    }
}

fn current_tag(app: &App) -> Option<RequestTag> {
    let tool = app
        .doc
        .interaction
        .tool
        .as_ref()
        .filter(|tool| tool.editing_existing)?;
    let display = app.doc.display.as_ref()?;
    (display.scene.identity() == tool.tag.context.scene).then(|| tool.tag.clone())
}

fn begin(app: &mut App, tag: RequestTag) {
    app.doc.model_handle.operands = None;
    app.doc.model_handle.history = None;
    let (sender, receiver) = mpsc::channel();
    let request = crate::operand_services::Request::EditContext {
        context: tag.context.clone(),
        record: tag.record_id,
        sender,
    };
    if app
        .doc
        .bridge
        .submit(Command::Operands(Box::new(request)))
        .is_err()
    {
        return;
    }
    let signature = app
        .doc
        .display
        .as_ref()
        .and_then(|display| {
            display
                .records
                .iter()
                .find(|record| record.record_id == tag.record_id)
        })
        .map(|record| signature(&record.op))
        .unwrap_or(serde_json::Value::Null);
    app.doc.model_handle.history = Some(History {
        tag,
        signature,
        receiver,
        read: None,
        context: None,
        failed: false,
        operand_changed: false,
    });
}

fn check_operand_changes(app: &mut App) {
    let Some(tool) = app.doc.interaction.tool.as_ref() else {
        return;
    };
    if let Some(history) = app.doc.model_handle.history.as_mut() {
        let changed = history.signature != signature(&tool.record.op);
        if changed && !history.operand_changed {
            app.doc.status = "Historical handles unavailable after changing operands. Numeric editing remains available.".into();
        }
        history.operand_changed = changed;
    }
}

fn publish(app: &mut App, context: EditOperandContext) {
    let current = app
        .doc
        .display
        .as_ref()
        .is_some_and(|d| d.context == context.fence)
        && app
            .doc
            .model_handle
            .history
            .as_ref()
            .is_some_and(|h| context.record == h.tag.record_id);
    if !current {
        app.doc.model_handle.history = None;
        return;
    }
    let Some(tool) = app.doc.interaction.tool.as_ref() else {
        app.doc.model_handle.history = None;
        return;
    };
    let Some(original) = app.doc.display.as_ref().and_then(|display| {
        display
            .records
            .iter()
            .find(|record| record.record_id == tool.tag.record_id)
    }) else {
        app.doc.model_handle.history = None;
        return;
    };
    let operands = Operands::from_history(tool, &context, &original.op);
    if operands.anchor_missing(&tool.record.op) {
        let reason = context
            .refusals
            .first()
            .map_or("operative input has no supported anchor", String::as_str);
        app.doc.status =
            format!("Historical handle unavailable: {reason}. Numeric editing remains available.");
    }
    if let Some(history) = app.doc.model_handle.history.as_mut() {
        history.context = Some(context);
        app.doc.model_handle.operands = Some(operands);
        app.draw_dirty = true;
    }
}

fn receive(history: &mut History) -> Option<Result<EditOperandContext, String>> {
    if history.read.is_none() {
        match history.receiver.try_recv() {
            Ok(Ok(read)) => history.read = Some(read),
            Ok(Err(error)) => return Some(Err(error)),
            Err(TryRecvError::Disconnected) => return Some(Err("operand request stopped".into())),
            Err(TryRecvError::Empty) => return None,
        }
    }
    let read = history.read.as_mut()?;
    match read.receiver.try_recv() {
        Ok(result) => Some(result.map_err(|error| error.to_string())),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
            Some(Err("operand request cancelled".into()))
        }
        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
    }
}

pub(super) fn ready(handles: &ModelHandle, tool: &onecad_interaction::ModelSession) -> bool {
    handles.history.as_ref().is_some_and(|history| {
        history.current(&tool.tag)
            && history.context.is_some()
            && history.signature == signature(&tool.record.op)
    })
}
pub(crate) fn eligible_bodies(app: &App) -> Option<&[onecad_core::ids::BodyId]> {
    let tool = app
        .doc
        .interaction
        .tool
        .as_ref()
        .filter(|tool| tool.editing_existing)?;
    Some(
        app.doc
            .model_handle
            .history
            .as_ref()
            .filter(|h| h.current(&tool.tag))
            .and_then(|h| h.context.as_ref())
            .map_or(&[], |context| context.bodies.as_slice()),
    )
}

// Scalar drafts keep the original prefix; changing an operand suppresses its old anchor.
fn signature(operation: &onecad_core::document::record::Operation) -> serde_json::Value {
    use onecad_core::document::record::{KnownOperation as Op, Operation};
    use serde_json::json;
    match operation {
        Operation::Known(Op::Shell(p)) => json!(["Shell", p.target_body, p.faces, p.open_faces]),
        Operation::Known(Op::OffsetFace(p)) => {
            json!(["OffsetFace", p.target_body, p.faces, p.opposite_face])
        }
        Operation::Known(Op::Fillet(p)) => json!(["Fillet", p.edges, p.edge_ids]),
        Operation::Known(Op::Chamfer(p)) => {
            json!(["Chamfer", p.edges, p.edge_ids, p.reference_face_refs])
        }
        Operation::Known(Op::Hole(p)) => json!(["Hole", p.target_body, p.face]),
        Operation::Known(Op::Revolve(p)) => json!(["Revolve", p.profile, p.axis, p.target_body]),
        Operation::Known(Op::Extrude(p)) => json!([
            "Extrude",
            p.profile,
            p.target_body,
            p.target_face,
            p.target_face2
        ]),
        Operation::Known(Op::MirrorBody(p)) => json!(["MirrorBody", p.source_body]),
        Operation::Known(Op::LinearPattern(p)) => json!(["LinearPattern", p.source_body]),
        Operation::Known(Op::CircularPattern(p)) => json!(["CircularPattern", p.source_body]),
        Operation::Known(Op::Gear(p)) => json!(["Gear", p.placement.face]),
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operand_signature_ignores_scalars_but_fences_inputs_and_axes() {
        let operation = |params| {
            serde_json::from_value(serde_json::json!({
                "opType": "Shell", "params": params
            }))
            .unwrap()
        };
        let a = operation(serde_json::json!({"thickness": 1, "openFaces": ["el_a"]}));
        let scalar = operation(serde_json::json!({"thickness": 2, "openFaces": ["el_a"]}));
        let changed = operation(serde_json::json!({"thickness": 1, "openFaces": ["el_b"]}));
        assert_eq!(signature(&a), signature(&scalar));
        assert_ne!(signature(&a), signature(&changed));
        let axis = serde_json::json!({"opType":"Revolve", "params":{
            "angleDeg":360, "profile":null, "axis":null, "booleanMode":"NewBody"
        }});
        let mut changed = axis.clone();
        changed["params"]["axis"] = serde_json::json!({"kind":"edge",
            "bodyId": onecad_core::ids::BodyId::new(), "edgeId":"el_axis"});
        let a: onecad_core::document::record::Operation = serde_json::from_value(axis).unwrap();
        let b: onecad_core::document::record::Operation = serde_json::from_value(changed).unwrap();
        assert_ne!(signature(&a), signature(&b));
    }
}
