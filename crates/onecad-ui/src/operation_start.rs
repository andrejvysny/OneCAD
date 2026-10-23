//! Fillet, Chamfer and Offset Face start from the current selection (ONECAD-316/319).
//! The picks stage on the Command lane without dirtying the document; the reply
//! binds through the shared binder and begins in the shared reducer at its
//! default size, so the feature card, preview and commit are the generic ones.
use crate::{
    app::App,
    bridge::Command,
    diagnostics::{Source, State},
    operand_services::Request,
};
use onecad_app::{api, worker::wire::body_id_wire};
use onecad_core::{
    document::{record::OperationRecord, variables::Scalar},
    ids::RecordId,
};
use onecad_interaction::{
    CommitStatus, DocumentContext,
    operand_binding::{self, DEFAULT_OPERATION_SIZE_MM, EdgeOpKind},
    selection_set::{SelectionItem, TopologyId, TopologyRef},
};
use std::sync::mpsc::{self, TryRecvError};

enum Reply {
    Elements(
        ElementOperation,
        mpsc::Receiver<Result<Vec<onecad_core::document::refs::ElementRef>, String>>,
    ),
    Hole(
        mpsc::Receiver<
            Result<
                (
                    onecad_core::document::refs::ElementRef,
                    onecad_core::math::Vec3,
                ),
                String,
            >,
        >,
    ),
    Edge(
        EdgeOpKind,
        mpsc::Receiver<Result<api::StagedEdgeOp, String>>,
    ),
    Offset(mpsc::Receiver<Result<api::StagedOffsetFace, String>>),
}

enum ElementOperation {
    Shell,
    Revolve(onecad_core::document::refs::SketchRegionRef),
}

/// A staging request in flight for the operation the user chose.
pub(crate) struct Pending {
    context: DocumentContext,
    /// Selection revision the picks came from; a change means other operands.
    selection: u64,
    step: u32,
    reply: Reply,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operand {
    Edge,
    Face,
}

/// Every selected item is an `operand` of one body, picked in the current scene.
fn operands(app: &App, operand: Operand) -> Option<(DocumentContext, Vec<TopologyRef>)> {
    let context = app.doc.interaction.context.clone()?;
    app.doc.selection.single_body()?;
    let picks: Option<Vec<TopologyRef>> = app
        .doc
        .selection
        .items()
        .iter()
        .map(|item| match (item, operand) {
            (SelectionItem::Edge(pick), Operand::Edge)
            | (SelectionItem::Face(pick), Operand::Face)
                if pick.scene == context.scene =>
            {
                Some(pick.clone())
            }
            _ => None,
        })
        .collect();
    picks
        .filter(|picks| !picks.is_empty())
        .map(|picks| (context, picks))
}

pub(crate) fn edges_ready(app: &App) -> bool {
    operands(app, Operand::Edge).is_some()
}

pub(crate) fn faces_ready(app: &App) -> bool {
    operands(app, Operand::Face).is_some()
}

/// A staging request is in flight; no other model tool may start meanwhile.
pub(crate) fn pending(app: &App) -> bool {
    app.doc.operation_start.is_some()
}

/// MESH1 names a pick by snapshot TopoKey or by promoted ElementId: (topo, element).
fn address(id: &TopologyId) -> (Option<String>, Option<String>) {
    match id {
        TopologyId::Snapshot(key) => (Some(key.as_str().to_owned()), None),
        TopologyId::Element(id) => (None, Some(id.as_str().to_owned())),
    }
}

pub(crate) fn begin_edge_op(app: &mut App, kind: EdgeOpKind) {
    let Some((context, picks)) = operands(app, Operand::Edge) else {
        app.doc.status = "Select edges of one body first".into();
        return;
    };
    let picks = picks
        .iter()
        .map(|pick| {
            let (topo_key, element_id) = address(&pick.id);
            api::EdgeOpPickInput {
                body_id: Some(body_id_wire(pick.body)),
                topo_key,
                element_id,
            }
        })
        .collect();
    let (sender, receiver) = mpsc::channel();
    let request = Request::EdgeOp {
        context: context.clone(),
        mode: kind.mode().into(),
        picks,
        chain_tangent: true,
        sender,
    };
    let label = kind.mode().to_lowercase();
    submit(app, request, context, Reply::Edge(kind, receiver), &label);
}

pub(crate) fn begin_offset_face(app: &mut App) {
    let Some((context, picks)) = operands(app, Operand::Face) else {
        app.doc.status = "Select faces of one body first".into();
        return;
    };
    let picks = picks
        .iter()
        .map(|pick| {
            let (topo_key, element_id) = address(&pick.id);
            api::OffsetFacePickInput {
                body_id: Some(body_id_wire(pick.body)),
                topo_key,
                element_id,
            }
        })
        .collect();
    let (sender, receiver) = mpsc::channel();
    let request = Request::OffsetFace {
        context: context.clone(),
        picks,
        sender,
    };
    submit(
        app,
        request,
        context,
        Reply::Offset(receiver),
        "offset face",
    );
}

fn submit(app: &mut App, request: Request, context: DocumentContext, reply: Reply, label: &str) {
    if app.doc.documents.running || app.doc.documents.confirm.is_some() {
        app.doc.status = "Finish the file action before editing the document".into();
        return;
    }
    let step = app
        .doc
        .display
        .as_ref()
        .map_or(0, |display| display.projection.applied_ops as u32);
    match app.doc.bridge.submit(Command::Operands(Box::new(request))) {
        Ok(()) => {
            app.doc.operation_start = Some(Pending {
                context,
                selection: app.doc.selection.revision(),
                step,
                reply,
            });
            app.doc.status = format!("Preparing {label}…");
        }
        Err(error) => refuse(app, error),
    }
}

/// Binds a staging reply that arrived, then begins the bound operation.
pub(crate) fn poll(app: &mut App) {
    let Some(pending) = app.doc.operation_start.as_ref() else {
        return;
    };
    let size = Scalar::new(DEFAULT_OPERATION_SIZE_MM);
    let mut surface = None;
    let bound = match &pending.reply {
        Reply::Elements(kind, receiver) => match receiver.try_recv() {
            Ok(result) => result.and_then(|refs| match kind {
                ElementOperation::Shell => {
                    onecad_interaction::modeling_records::shell(refs, RecordId::new(), pending.step)
                }
                ElementOperation::Revolve(profile) => {
                    let edge = refs
                        .into_iter()
                        .next()
                        .ok_or("Axis staging returned no edge")?;
                    onecad_interaction::modeling_records::revolve(
                        profile.clone(),
                        edge,
                        RecordId::new(),
                        pending.step,
                    )
                }
            }),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(stopped()),
        },
        Reply::Hole(receiver) => match receiver.try_recv() {
            Ok(result) => result.and_then(|(face, point)| {
                onecad_interaction::modeling_records::hole(
                    face,
                    point,
                    RecordId::new(),
                    pending.step,
                )
            }),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(stopped()),
        },
        Reply::Edge(kind, receiver) => match receiver.try_recv() {
            Ok(staged) => staged.and_then(|staged| {
                operand_binding::bind_edge_op(*kind, &staged, size, RecordId::new(), pending.step)
            }),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(stopped()),
        },
        Reply::Offset(receiver) => match receiver.try_recv() {
            Ok(staged) => staged.and_then(|staged| {
                surface = staged.surface.clone();
                operand_binding::bind_offset_face(&staged, size, RecordId::new(), pending.step)
            }),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(stopped()),
        },
    };
    let Some(pending) = app.doc.operation_start.take() else {
        return;
    };
    match bound {
        Ok(record) => start(app, record, pending, surface),
        Err(message) => refuse(app, message),
    }
}

fn stopped() -> String {
    "Operand staging stopped before answering; select again".into()
}

/// Staged refs are valid only under the head they were staged against, and
/// only for the selection the user chose the tool on.
fn start(
    app: &mut App,
    record: OperationRecord,
    pending: Pending,
    surface: Option<onecad_app::dto::ClassifyElementDto>,
) {
    let context = pending.context;
    if app.doc.interaction.context.as_ref() != Some(&context) || app.doc.interaction.tool.is_some()
    {
        refuse(
            app,
            "The document changed while preparing; select again".into(),
        );
        return;
    }
    if app.doc.selection.revision() != pending.selection {
        refuse(
            app,
            "The selection changed while preparing; choose the tool again".into(),
        );
        return;
    }
    crate::model_handle::capture_operands(app, &record, &context);
    crate::model_handle::capture_surface(app, &record, &context, surface.as_ref());
    app.doc.gesture += 1;
    match operand_binding::begin_bound(record, app.doc.gesture, context) {
        Ok(action) => app.dispatch(action),
        Err(message) => return refuse(app, message),
    }
    if app.doc.interaction.tool.is_some() {
        app.doc.status = "Enter a value, then Create".into();
    } else if let CommitStatus::Rejected(reason) = &app.doc.interaction.status {
        app.doc.status = reason.clone();
    }
    app.draw_dirty = true;
}

fn refuse(app: &mut App, message: String) {
    app.doc
        .diagnostics
        .record(Source::Model, State::Rejected, &message);
    app.doc.status = message;
}

fn stage_pick(
    pick: &TopologyRef,
    kind: onecad_core::document::refs::ElementKind,
) -> api::StagePick {
    api::StagePick {
        body: pick.body,
        kind,
        address: match &pick.id {
            TopologyId::Snapshot(key) => api::StageAddress::TopoKey(key.as_str().into()),
            TopologyId::Element(id) => api::StageAddress::Element(id.as_str().into()),
        },
    }
}

pub(crate) fn begin_shell(app: &mut App) {
    let Some((context, picks)) = operands(app, Operand::Face) else {
        return;
    };
    let picks = picks
        .iter()
        .map(|pick| stage_pick(pick, onecad_core::document::refs::ElementKind::Face))
        .collect();
    let (sender, receiver) = mpsc::channel();
    let request = Request::Elements {
        context: context.clone(),
        picks,
        sender,
    };
    submit(
        app,
        request,
        context,
        Reply::Elements(ElementOperation::Shell, receiver),
        "shell",
    );
}

pub(crate) fn hole_ready(app: &App) -> bool {
    operands(app, Operand::Face).is_some_and(|(_, picks)| picks.len() == 1)
}

pub(crate) fn begin_hole(app: &mut App) {
    let Some((context, picks)) = operands(app, Operand::Face).filter(|(_, picks)| picks.len() == 1)
    else {
        return;
    };
    let pick = stage_pick(&picks[0], onecad_core::document::refs::ElementKind::Face);
    let (sender, receiver) = mpsc::channel();
    let request = Request::Hole {
        context: context.clone(),
        pick,
        sender,
    };
    submit(app, request, context, Reply::Hole(receiver), "hole");
}

fn revolve_operands(app: &App) -> Option<(onecad_interaction::ProfileSelection, TopologyRef)> {
    let context = app.doc.interaction.context.as_ref()?;
    let mut profile = None;
    let mut edge = None;
    for item in app.doc.selection.items() {
        match item {
            SelectionItem::Profile(p) if profile.is_none() && &p.context == context => {
                profile = Some((**p).clone())
            }
            SelectionItem::Edge(p) if edge.is_none() && p.scene == context.scene => {
                edge = Some(p.clone())
            }
            _ => return None,
        }
    }
    Some((profile?, edge?))
}

pub(crate) fn revolve_ready(app: &App) -> bool {
    revolve_operands(app).is_some() || crate::revolve_ui::ready(app)
}

pub(crate) fn begin_revolve(app: &mut App) {
    let Some((profile, edge)) = revolve_operands(app) else {
        crate::revolve_ui::begin(app);
        return;
    };
    let context = profile.context;
    let picks = vec![stage_pick(
        &edge,
        onecad_core::document::refs::ElementKind::Edge,
    )];
    let (sender, receiver) = mpsc::channel();
    let request = Request::Elements {
        context: context.clone(),
        picks,
        sender,
    };
    submit(
        app,
        request,
        context,
        Reply::Elements(ElementOperation::Revolve(profile.profile), receiver),
        "revolve",
    );
}
