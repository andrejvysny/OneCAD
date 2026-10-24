//! Explicit replacement selection on the healthy partial head.
use crate::{
    app::App,
    bridge::{Command, Display},
    operand_services::Request,
};
use onecad_app::{
    api,
    dto::{FeatureDto, FeatureStatus},
};
use onecad_core::{document::refs::ElementKind, ids::RecordId};
use onecad_interaction::{
    Action, DocumentContext,
    selection_set::{SelectionItem, TopologyId},
};
use std::sync::mpsc::{self, TryRecvError};

pub(crate) struct Pending {
    context: DocumentContext,
    selection: u64,
    reply: mpsc::Receiver<Result<api::InputRepair, String>>,
}

pub(crate) fn button(
    app: &mut App,
    ui: &mut egui::Ui,
    display: &Display,
    index: usize,
    feature: &FeatureDto,
) {
    if feature.status != FeatureStatus::NeedsRepair
        || display
            .projection
            .features
            .iter()
            .position(|feature| feature.status == FeatureStatus::NeedsRepair)
            != Some(index)
    {
        return;
    }
    let prepared = selection(app, display, feature);
    let enabled = prepared.is_ok()
        && app.doc.input_repair.is_none()
        && app.doc.interaction.tool.is_none()
        && !app.busy()
        && app.viewport_ready()
        && app.doc.bridge.valid()
        && !app.sketch_active()
        && !crate::operation_start::pending(app);
    let response = ui.add_enabled(enabled, egui::Button::new("Repair using selection"));
    if let Err(reason) = &prepared {
        ui.add(egui::Label::new(reason).wrap());
    }
    if response.clicked()
        && let Ok((context, record, pick)) = prepared
    {
        let (sender, reply) = mpsc::channel();
        let request = Request::InputRepair {
            context: context.clone(),
            record,
            pick,
            sender,
        };
        match app.doc.bridge.submit(Command::Operands(Box::new(request))) {
            Ok(()) => {
                app.doc.input_repair = Some(Pending {
                    context,
                    selection: app.doc.selection.revision(),
                    reply,
                });
                app.doc.status = "Preparing explicit input repair…".into();
            }
            Err(error) => app.doc.status = error,
        }
    }
}

fn selection(
    app: &App,
    display: &Display,
    feature: &FeatureDto,
) -> Result<(DocumentContext, RecordId, api::StagePick), String> {
    let record = display
        .records
        .iter()
        .find(|record| record.record_id.to_string() == feature.id)
        .ok_or("Repair feature is unavailable")?;
    let target = api::input_repair_target(record)?;
    let context = app
        .doc
        .interaction
        .context
        .clone()
        .ok_or("Repair head is unavailable")?;
    let [item] = app.doc.selection.items() else {
        return Err(format!(
            "Select one replacement {} on the original body.",
            if target.kind == ElementKind::Edge {
                "edge"
            } else {
                "face"
            }
        ));
    };
    let pick = match (item, target.kind) {
        (SelectionItem::Edge(pick), ElementKind::Edge)
        | (SelectionItem::Face(pick), ElementKind::Face) => pick,
        _ => return Err("Replacement selection has the wrong kind".into()),
    };
    if pick.body != target.body || pick.scene != context.scene {
        return Err("Select the replacement on the original body".into());
    }
    let address = match &pick.id {
        TopologyId::Snapshot(key) => api::StageAddress::TopoKey(key.as_str().into()),
        TopologyId::Element(id) => api::StageAddress::Element(id.as_str().into()),
    };
    Ok((
        context,
        record.record_id,
        api::StagePick {
            body: target.body,
            kind: target.kind,
            address,
        },
    ))
}

pub(crate) fn poll(app: &mut App) {
    let Some(pending) = &app.doc.input_repair else {
        return;
    };
    let result = match pending.reply.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("Repair staging stopped; select again".into()),
    };
    let Some(pending) = app.doc.input_repair.take() else {
        return;
    };
    if app.doc.interaction.context.as_ref() != Some(&pending.context)
        || app.doc.selection.revision() != pending.selection
        || app.doc.interaction.tool.is_some()
    {
        app.doc.status = "Model or selection changed; select again".into();
        return;
    }
    match result {
        Ok(repair) => {
            app.doc.gesture += 1;
            app.dispatch(Action::BeginInputRepair {
                repair: Box::new(repair),
                gesture: app.doc.gesture,
                context: pending.context,
            });
        }
        Err(error) => app.doc.status = error,
    }
}
