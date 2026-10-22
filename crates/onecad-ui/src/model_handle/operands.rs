//! New-operation anchors retain the exact, fenced selection that staging promoted.
use super::{
    ModelHandle,
    anchors::{self, Context},
};
use crate::app::App;
use onecad_core::{
    document::{
        record::{KnownOperation as Op, Operation, OperationRecord},
        refs::AxisRef,
    },
    ids::{BodyId, ElementId, RecordId},
};
use onecad_interaction::{
    DocumentContext, ModelSession,
    selection_set::{SelectionItem, TopologyId, TopologyRef},
};
use onecad_scene::DVec3;

pub(super) struct Operands {
    record: RecordId,
    context: DocumentContext,
    pub face: Option<(DVec3, DVec3)>,
    pub edge: Option<(DVec3, DVec3)>,
    pub axis: Option<(DVec3, DVec3)>,
}
impl ModelHandle {
    pub(super) fn operands(&self, tool: &ModelSession) -> Option<&Operands> {
        self.operands
            .as_ref()
            .filter(|o| o.record == tool.tag.record_id && o.context == tool.tag.context)
    }
}
pub(crate) fn capture(app: &mut App, record: &OperationRecord, context: &DocumentContext) {
    app.doc.model_handle.operands = None;
    let Some(display) = &app.doc.display else {
        return;
    };
    if display.scene.identity() != context.scene {
        return;
    }
    let Some((body, face)) = binding(record) else {
        return;
    };
    let Some(picked) = selected(app, body, face, context) else {
        return;
    };
    let id = match &picked.id {
        TopologyId::Element(id) => id.as_str(),
        TopologyId::Snapshot(key) => key.as_str(),
    };
    let geometry = Context {
        scene: &display.scene,
        records: &display.records,
        profiles: &display.profiles,
        profile_anchors: &display.profile_anchors,
    };
    let mut operands = Operands {
        record: record.record_id,
        context: context.clone(),
        face: None,
        edge: None,
        axis: None,
    };
    if face {
        operands.face = anchors::face(&geometry, body, &ElementId::new(id));
    } else {
        operands.edge = anchors::edge_geometry(&geometry, body, id);
        operands.axis = anchors::axis(
            &geometry,
            &AxisRef::Element {
                body,
                edge: ElementId::new(id),
                edge_ref: None,
                extra: Default::default(),
            },
        );
    }
    app.doc.model_handle.operands = Some(operands);
}

fn binding(record: &OperationRecord) -> Option<(BodyId, bool)> {
    let (body, face) = match &record.op {
        Operation::Known(Op::Shell(p)) => (p.target_body, true),
        Operation::Known(Op::Hole(p)) => (Some(p.target_body), true),
        Operation::Known(Op::OffsetFace(p)) => (Some(p.target_body), true),
        Operation::Known(Op::Fillet(p)) => (
            p.edges
                .first()
                .and_then(|r| r.primary.as_ref())
                .map(|p| p.body),
            false,
        ),
        Operation::Known(Op::Chamfer(p)) => (
            p.edges
                .first()
                .and_then(|r| r.primary.as_ref())
                .map(|p| p.body),
            false,
        ),
        Operation::Known(Op::Revolve(p)) => (
            match p.axis.as_ref() {
                Some(AxisRef::Element { body, .. }) => Some(*body),
                _ => None,
            },
            false,
        ),
        _ => return None,
    };
    Some((body?, face))
}
fn selected<'a>(
    app: &'a App,
    body: BodyId,
    face: bool,
    context: &DocumentContext,
) -> Option<&'a TopologyRef> {
    app.doc
        .selection
        .items()
        .iter()
        .find_map(|item| match item {
            SelectionItem::Face(p) if face => Some(p),
            SelectionItem::Edge(p) if !face => Some(p),
            _ => None,
        })
        .filter(|p| p.body == body && p.scene == context.scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_operands_never_cross_record_runtime_or_scene_fences() {
        let scene = super::super::test_support::scene();
        let record = onecad_interaction::modeling_records::mirror(
            scene.bodies()[0].body_id(),
            RecordId::new(),
            0,
        );
        let mut state = super::super::test_support::begin(record);
        let tool = state.tool.as_mut().unwrap();
        let handles = ModelHandle {
            operands: Some(Operands {
                record: tool.tag.record_id,
                context: tool.tag.context.clone(),
                face: Some((DVec3::ZERO, DVec3::Z)),
                edge: None,
                axis: None,
            }),
            ..Default::default()
        };
        assert!(handles.operands(tool).is_some());
        let original = tool.tag.clone();
        tool.tag.record_id = RecordId::new();
        assert!(handles.operands(tool).is_none());
        tool.tag = original.clone();
        tool.tag.context.runtime_session.push('x');
        assert!(handles.operands(tool).is_none());
        tool.tag = original.clone();
        tool.tag.context.scene.scene += 1;
        assert!(handles.operands(tool).is_none());
    }
}
