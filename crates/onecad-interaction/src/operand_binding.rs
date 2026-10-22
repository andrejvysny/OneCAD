//! Binds staged operands to new feature records (ONECAD-316/319). Pure: the host
//! stages through `api::stage_edge_op` or `api::stage_offset_face` and begins
//! the returned record. The checks mirror the
//! reducer's creation validation so a refused binding is explained here, by
//! name, instead of surfacing as a generic begin refusal.
use crate::{Action, DocumentContext, NumericParameter, ParameterAxis, ParameterKey};
use onecad_app::{
    api::{StagedEdgeOp, StagedOffsetFace},
    worker::wire,
};
use onecad_core::{
    document::{
        record::{
            ChamferParams, FilletParams, KnownOperation, OffsetDistanceType, OffsetFaceParams,
            Operation, OperationRecord, TransformBodyParams, TransformRotation,
            OFFSET_FACE_RESULT_POLICY_VERSION,
        },
        refs::{ElementKind, ElementRef},
        variables::Scalar,
    },
    ids::{BodyId, ElementId, RecordId, TopoKey},
    math::Vec3,
};

/// Default size a new edge or face operation starts at, in mm.
pub const DEFAULT_OPERATION_SIZE_MM: f64 = 1.0;

/// SCHEMA §7.3: fresh edge-op records freeze the full prepared closure.
const TANGENT_CLOSURE_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeOpKind {
    Fillet,
    Chamfer,
}

impl EdgeOpKind {
    /// The `stage_edge_op` mode string.
    pub fn mode(self) -> &'static str {
        match self {
            Self::Fillet => "Fillet",
            Self::Chamfer => "Chamfer",
        }
    }

    pub fn parameter(self) -> NumericParameter {
        match self {
            Self::Fillet => NumericParameter::FilletRadius,
            Self::Chamfer => NumericParameter::ChamferDistance,
        }
    }
}

/// A new edge-op record over the staged closure, in `prepared.edges` order.
/// Chamfers are equal-leg: an asymmetric leg needs promoted reference faces,
/// which this binding does not stage.
pub fn bind_edge_op(
    kind: EdgeOpKind,
    staged: &StagedEdgeOp,
    size: Scalar,
    record_id: RecordId,
    step_index: u32,
) -> Result<OperationRecord, String> {
    if let Some(refusal) = &staged.prepared.refusal {
        return Err(format!("{} refused: {}", kind.mode(), refusal.message));
    }
    if staged.refs.len() != staged.prepared.edges.len() {
        return Err(format!(
            "{} staging returned {} refs for {} closure edges; re-pick",
            kind.mode(),
            staged.refs.len(),
            staged.prepared.edges.len()
        ));
    }
    let body = wire::parse_body_id(&staged.prepared.target_body_id)?;
    let edge_ids = persistent_ids(&staged.refs, ElementKind::Edge, body)?;
    for (edge, id) in staged.prepared.edges.iter().zip(&edge_ids) {
        if edge.element_id.as_deref() != Some(id.as_str()) {
            return Err("Staged edge identity does not match its closure entry; re-pick".into());
        }
    }
    let edges = staged.refs.clone();
    let op = match kind {
        EdgeOpKind::Fillet => KnownOperation::Fillet(FilletParams {
            radius: size,
            edge_ids,
            edges,
            chain_tangent_edges: true,
            tangent_closure_version: Some(TANGENT_CLOSURE_VERSION),
            extra: Default::default(),
        }),
        EdgeOpKind::Chamfer => KnownOperation::Chamfer(ChamferParams {
            radius: size,
            distance2: None,
            angle_deg: None,
            edge_ids,
            edges,
            reference_faces: Vec::new(),
            reference_face_refs: Vec::new(),
            chain_tangent_edges: true,
            tangent_closure_version: Some(TANGENT_CLOSURE_VERSION),
            extra: Default::default(),
        }),
    };
    Ok(OperationRecord::new(
        record_id,
        step_index,
        kind.mode(),
        Operation::Known(op),
    ))
}

/// A new signed OffsetFace over the staged closure, in `prepared.faces` order.
/// The user's picks become the primary faces.
pub fn bind_offset_face(
    staged: &StagedOffsetFace,
    distance: Scalar,
    record_id: RecordId,
    step_index: u32,
) -> Result<OperationRecord, String> {
    let (prepared, refs) = (&staged.prepared, &staged.refs);
    if let Some(refusal) = &prepared.refusal {
        return Err(format!("Offset face refused: {}", refusal.message));
    }
    if refs.is_empty() || refs.len() != prepared.faces.len() {
        return Err(format!(
            "Offset face staging returned {} refs for {} closure faces; re-pick",
            refs.len(),
            prepared.faces.len()
        ));
    }
    let body = wire::parse_body_id(&prepared.target_body_id)?;
    let face_ids = persistent_ids(refs, ElementKind::Face, body)?;
    let primary_face_ids: Vec<ElementId> = prepared
        .faces
        .iter()
        .zip(&face_ids)
        .filter(|(face, _)| face.picked)
        .map(|(_, id)| id.clone())
        .collect();
    if primary_face_ids.is_empty() {
        return Err("Offset face closure names none of the picked faces; re-pick".into());
    }
    let params = OffsetFaceParams {
        face_ids,
        primary_face_ids,
        faces: refs.clone(),
        distance,
        distance_type: OffsetDistanceType::Offset,
        chain_tangent_faces: true,
        opposite_face_id: None,
        opposite_face: None,
        target_body: body,
        result_policy_version: Some(OFFSET_FACE_RESULT_POLICY_VERSION),
        extra: Default::default(),
    };
    params.validate()?;
    Ok(OperationRecord::new(
        record_id,
        step_index,
        "Offset face",
        Operation::Known(KnownOperation::OffsetFace(params)),
    ))
}

/// Each ref must be a promoted persistent element of `kind` on `body`.
pub(crate) fn persistent_ids(
    refs: &[ElementRef],
    kind: ElementKind,
    body: BodyId,
) -> Result<Vec<ElementId>, String> {
    refs.iter()
        .map(|reference| -> Result<ElementId, String> {
            let primary = reference
                .primary
                .as_ref()
                .ok_or("Staged operand has no promoted identity; re-pick")?;
            if primary.kind != kind || primary.body != body {
                return Err("Staged operand body or kind does not match the operation".into());
            }
            if primary.element.as_str().is_empty()
                || TopoKey::new(primary.element.as_str()).is_valid()
            {
                return Err("Staged operand is a snapshot TopoKey, not a persistent id".into());
            }
            Ok(primary.element.clone())
        })
        .collect()
}

/// A new Move / Rotate of one body about its bounds centre, at the identity.
pub fn transform_record(
    body: BodyId,
    center: [f64; 3],
    record_id: RecordId,
    step_index: u32,
) -> OperationRecord {
    let params = TransformBodyParams {
        targets: vec![body],
        translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: TransformRotation {
            center: Vec3::new_unchecked(center[0], center[1], center[2]),
            ..Default::default()
        },
        copy: false,
        extra: Default::default(),
    };
    OperationRecord::new(
        record_id,
        step_index,
        "Move / Rotate",
        Operation::Known(KnownOperation::TransformBody(params)),
    )
}

/// Every exact-entry field of a new Move / Rotate; X takes focus.
pub fn transform_fields() -> Vec<ParameterKey> {
    vec![
        NumericParameter::TransformTranslation(ParameterAxis::X).into(),
        NumericParameter::TransformTranslation(ParameterAxis::Y).into(),
        NumericParameter::TransformTranslation(ParameterAxis::Z).into(),
        NumericParameter::TransformRotationAngle.into(),
    ]
}

/// The begin action for a bound record: its first registered field, focused.
pub fn begin_bound(
    record: OperationRecord,
    gesture: u64,
    context: DocumentContext,
) -> Result<Action, String> {
    let fields = crate::declared_keys(&record.op);
    let focus = fields.first().copied();
    Ok(Action::BeginOperation {
        record: Box::new(record),
        fields,
        focus,
        gesture,
        context,
    })
}
