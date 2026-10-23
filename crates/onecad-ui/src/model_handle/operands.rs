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
    pub surface: Option<super::surface::Surface>,
    body_bounds: Vec<(BodyId, onecad_scene::Bounds)>,
}
impl Operands {
    pub(super) fn from_history(
        tool: &ModelSession,
        context: &onecad_app::api::EditOperandContext,
        original: &Operation,
    ) -> Self {
        let mut operands = Self {
            record: tool.tag.record_id,
            context: tool.tag.context.clone(),
            face: None,
            edge: None,
            axis: None,
            surface: None,
            body_bounds: context
                .body_bounds
                .iter()
                .filter_map(|value| {
                    Some((
                        value.body,
                        onecad_scene::Bounds::new(
                            DVec3::from_array(value.min),
                            DVec3::from_array(value.max),
                        )
                        .ok()?,
                    ))
                })
                .collect(),
        };
        let Some(input) = resolved_primary(original, &context.operands) else {
            return operands;
        };
        let point = DVec3::from_array(input.point);
        let direction = DVec3::from_array(input.direction);
        match input.kind {
            onecad_core::document::refs::ElementKind::Face => {
                operands.face = Some((point, direction));
                operands.surface =
                    super::surface::Surface::from(anchors::core(point), &input.classification);
            }
            onecad_core::document::refs::ElementKind::Edge => {
                operands.edge = Some((point, direction));
                operands.axis = input.classification.frame.as_ref().and_then(|frame| {
                    (input.classification.curve_type == "line").then_some(())?;
                    Some((
                        DVec3::from_array(frame.origin),
                        DVec3::from_array(frame.axis?).try_normalize()?,
                    ))
                });
            }
            _ => {}
        }
        operands
    }

    pub(super) fn bounds(&self, body: BodyId) -> Option<onecad_scene::Bounds> {
        self.body_bounds
            .iter()
            .find(|(id, _)| *id == body)
            .map(|(_, bounds)| *bounds)
    }
    pub(super) fn anchor_missing(&self, operation: &Operation) -> bool {
        if let Operation::Known(Op::OffsetFace(p)) = operation
            && p.distance_type != onecad_core::document::record::OffsetDistanceType::Offset
        {
            return self
                .surface
                .and_then(|surface| surface.geometry(p.distance_type))
                .is_none();
        }
        let body = match operation {
            Operation::Known(Op::MirrorBody(p)) => p.source_body,
            Operation::Known(Op::LinearPattern(p)) => p.source_body,
            Operation::Known(Op::CircularPattern(p)) => p.source_body,
            _ => None,
        };
        body.is_some_and(|body| self.bounds(body).is_none())
            || operative(operation).is_some() && self.face.is_none() && self.edge.is_none()
    }
    pub(super) fn matches(&self, record: RecordId, context: &DocumentContext) -> bool {
        self.record == record && self.context == *context
    }
}
fn resolved_primary<'a>(
    operation: &Operation,
    inputs: &'a [onecad_app::worker::OperandGeometry],
) -> Option<&'a onecad_app::worker::OperandGeometry> {
    let (index, primary) = operative(operation)?;
    inputs.iter().find(|input| {
        input.input == index
            && input.body == primary.body
            && input.element == primary.element
            && input.kind == primary.kind
    })
}
fn operative(operation: &Operation) -> Option<(usize, &onecad_core::document::refs::PrimaryRef)> {
    let Operation::Known(op) = operation else {
        return None;
    };
    let (index, reference) = match op {
        Op::Shell(p) => (0, p.faces.first()?),
        Op::OffsetFace(p) => (0, p.faces.first()?),
        Op::Fillet(p) => (0, p.edges.first()?),
        Op::Chamfer(p) => (0, p.edges.first()?),
        Op::Hole(p) => (1, &p.face),
        Op::Gear(p) => (0, p.placement.face.as_ref()?),
        Op::Revolve(p) => match p.axis.as_ref()? {
            AxisRef::Element {
                edge_ref: Some(reference),
                ..
            } => (0, reference),
            _ => return None,
        },
        _ => return None,
    };
    Some((index, reference.primary.as_ref()?))
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
        surface: None,
        body_bounds: Vec::new(),
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
    fn refused_primary_never_substitutes_a_resolved_secondary_input() {
        use onecad_core::document::refs::ElementKind;
        let body = BodyId::new();
        let operation: Operation = serde_json::from_value(serde_json::json!({
            "opType":"Shell","params":{"thickness":1,"targetBodyId":body,
                "openFaces":["el_first","el_second"],"faces":[
                    {"primary":{"bodyId":body,"elementId":"el_first","kind":"face"}},
                    {"primary":{"bodyId":body,"elementId":"el_second","kind":"face"}}]}
        }))
        .unwrap();
        let mut input = onecad_app::worker::OperandGeometry {
            input: 1,
            body,
            element: ElementId::new("el_second"),
            kind: ElementKind::Face,
            point: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
            tangent: None,
            classification: onecad_app::dto::ClassifyElementDto {
                kind: "face".into(),
                surface_type: "plane".into(),
                curve_type: String::new(),
                frame: None,
            },
        };
        assert!(resolved_primary(&operation, std::slice::from_ref(&input)).is_none());
        input.input = 0;
        assert!(resolved_primary(&operation, std::slice::from_ref(&input)).is_none());
        input.element = ElementId::new("el_first");
        assert!(resolved_primary(&operation, std::slice::from_ref(&input)).is_some());
        input.body = BodyId::new();
        assert!(resolved_primary(&operation, std::slice::from_ref(&input)).is_none());
    }
    #[test]
    fn historical_mirror_uses_prefix_body_center_instead_of_final_scene_bounds() {
        use onecad_core::document::record::KnownOperation;
        let scene = super::super::test_support::scene();
        let record = super::super::test_support::records(&scene)
            .into_iter()
            .find(|(record, _)| {
                matches!(record.op, Operation::Known(KnownOperation::MirrorBody(_)))
            })
            .unwrap()
            .0;
        let state = super::super::test_support::begin(record);
        let tool = state.tool.as_ref().unwrap();
        let Operation::Known(KnownOperation::MirrorBody(p)) = &tool.record.op else {
            panic!()
        };
        let body = p.source_body.unwrap();
        let bounds = onecad_scene::Bounds::new(
            DVec3::new(100.0, 200.0, 300.0),
            DVec3::new(110.0, 220.0, 330.0),
        )
        .unwrap();
        let handles = ModelHandle {
            operands: Some(Operands {
                record: tool.tag.record_id,
                context: tool.tag.context.clone(),
                face: None,
                edge: None,
                axis: None,
                surface: None,
                body_bounds: vec![(body, bounds)],
            }),
            ..Default::default()
        };
        let context = Context {
            scene: &scene,
            records: &[],
            profiles: &[],
            profile_anchors: &[],
        };
        let targets = super::super::target::for_tool(tool, &context, &handles);
        let super::super::Geometry::Axis { base, .. } = targets[0].geometry else {
            panic!()
        };
        let center = (bounds.min + bounds.max) * 0.5;
        let normal = anchors::vector(p.plane_normal).normalize();
        let expected = center - normal * (center - anchors::vector(p.plane_point)).dot(normal);
        assert!(base.distance(expected) < 1e-9);
    }
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
                surface: None,
                body_bounds: Vec::new(),
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
