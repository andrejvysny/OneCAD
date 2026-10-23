//! Strict, bounded operand context carried by a non-publishing prefix replay.
use super::{wire, PlanPreview, PreviewPlanRequest};
use crate::dto::ClassifyElementDto;
use onecad_core::{
    document::{record::Operation, refs::ElementKind},
    ids::{BodyId, ElementId, RecordId},
};
use serde_json::Value;
use std::collections::HashSet;

pub const EDIT_OPERAND_CAPABILITY: &str = "editOperandContext";
pub const MAX_EDIT_OPERAND_INPUTS: usize = 64;

#[derive(Debug, Clone)]
pub struct EditOperandRequest {
    pub prefix: PreviewPlanRequest,
    pub record: RecordId,
    pub operation: Operation,
    pub(crate) serialization: std::sync::Arc<tokio::sync::Mutex<()>>,
}
#[derive(Debug, Clone)]
pub struct OperandGeometry {
    pub input: usize,
    pub body: BodyId,
    pub element: ElementId,
    pub kind: ElementKind,
    pub point: [f64; 3],
    pub direction: [f64; 3],
    pub tangent: Option<[f64; 3]>,
    pub classification: ClassifyElementDto,
}
#[derive(Debug, Clone)]
pub struct OperandBodyBounds {
    pub input: usize,
    pub body: BodyId,
    pub min: [f64; 3],
    pub max: [f64; 3],
}
#[derive(Debug, Clone)]
pub enum OperandContextGeometry {
    Element(Box<OperandGeometry>),
    Body(OperandBodyBounds),
}
#[derive(Debug, Clone)]
pub struct OperandContextPreview {
    pub prefix: PlanPreview,
    pub eligible_bodies: Vec<BodyId>,
    pub inputs: Vec<Result<OperandContextGeometry, String>>,
}

pub(crate) fn unavailable() -> onecad_core::regen::EngineError {
    onecad_core::regen::EngineError::OpFailed {
        code: onecad_core::regen::OpFailureCode::Unsupported,
        recoverable: true,
        message: format!(
            "edit operand context unavailable: worker lacks {EDIT_OPERAND_CAPABILITY}"
        ),
        diagnostics: Vec::new(),
    }
}

pub(crate) fn validate_operation(operation: &Operation) -> Result<(), String> {
    use onecad_core::document::record::KnownOperation as Op;
    let count = match operation {
        Operation::Known(Op::Fillet(p)) => {
            if p.edges.is_empty() {
                p.edge_ids.len()
            } else {
                p.edges.len()
            }
        }
        Operation::Known(Op::Chamfer(p)) => (if p.edges.is_empty() {
            p.edge_ids.len()
        } else {
            p.edges.len()
        })
        .saturating_add(p.reference_face_refs.len()),
        Operation::Known(Op::Shell(p)) => {
            if p.faces.is_empty() {
                p.open_faces.len()
            } else {
                p.faces.len()
            }
        }
        Operation::Known(Op::OffsetFace(p)) => p
            .faces
            .len()
            .saturating_add(usize::from(p.opposite_face.is_some())),
        Operation::Known(Op::Boolean(p)) => p.tools.len().saturating_add(1),
        Operation::Known(Op::TransformBody(p)) => p.targets.len(),
        _ => 2,
    };
    if count > MAX_EDIT_OPERAND_INPUTS {
        return Err(
            "edit operand context supports at most 64 inputs; numeric editing remains available"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn operation(request: &EditOperandRequest) -> Result<Value, String> {
    validate_operation(&request.operation)?;
    let operation = wire::preview_wire_op(&request.operation, &request.record.to_string());
    let inputs = operation["inputs"]
        .as_array()
        .ok_or("operand inputs missing")?;
    if inputs.len() > MAX_EDIT_OPERAND_INPUTS {
        return Err(
            "edit operand context supports at most 64 inputs; numeric editing remains available"
                .into(),
        );
    }
    Ok(operation)
}
fn vector(value: &Value) -> Result<[f64; 3], String> {
    let values = value
        .as_array()
        .filter(|v| v.len() == 3)
        .ok_or("invalid operand vector")?;
    let mut result = [0.0; 3];
    for (out, value) in result.iter_mut().zip(values) {
        *out = value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("nonfinite operand coordinate")?;
    }
    Ok(result)
}
fn direction(value: &Value) -> Result<[f64; 3], String> {
    let result = vector(value)?;
    let norm: f64 = result.iter().map(|v| v * v).sum();
    if (norm - 1.0).abs() > 1e-6 {
        return Err("operand direction is not unit length".into());
    }
    Ok(result)
}

pub(crate) fn parse(
    prefix: PlanPreview,
    operation: &Value,
    context: &Value,
) -> Result<OperandContextPreview, String> {
    let eligible = context["eligibleBodies"]
        .as_array()
        .ok_or("operand context body list missing")?;
    let mut known = HashSet::new();
    let mut eligible_bodies = Vec::new();
    for value in eligible {
        let body = wire::parse_body_id(value.as_str().ok_or("invalid operand body")?)?;
        if !prefix.bodies.contains(&body) || !known.insert(body) {
            return Err("operand body absent from prefix or duplicated".into());
        }
        eligible_bodies.push(body);
    }
    let expected = operation["inputs"]
        .as_array()
        .ok_or("requested operand list missing")?;
    let inputs = context["inputs"]
        .as_array()
        .ok_or("operand context inputs missing")?;
    if inputs.len() != expected.len() || inputs.len() > MAX_EDIT_OPERAND_INPUTS {
        return Err("operand context must answer every requested input exactly once".into());
    }
    let inputs = inputs
        .iter()
        .zip(expected)
        .enumerate()
        .map(|(index, (value, expected))| parse_input(index, value, expected, &prefix.bodies))
        .collect::<Result<_, _>>()?;
    Ok(OperandContextPreview {
        prefix,
        eligible_bodies,
        inputs,
    })
}

fn parse_input(
    index: usize,
    value: &Value,
    expected: &Value,
    bodies: &[BodyId],
) -> Result<Result<OperandContextGeometry, String>, String> {
    if value["inputIndex"].as_u64() != Some(index as u64) {
        return Err("operand input order changed".into());
    }
    for key in ["bodyId", "elementId", "kind"] {
        if value[key].as_str().unwrap_or("") != expected["primary"][key].as_str().unwrap_or("") {
            return Err(format!("operand {key} changed"));
        }
    }
    if let Some(reason) = value["refusal"].as_str() {
        if reason.is_empty() {
            return Err("empty operand refusal".into());
        }
        return Ok(Err(reason.into()));
    }
    let body = wire::parse_body_id(value["bodyId"].as_str().ok_or("operand body missing")?)?;
    if !bodies.contains(&body) {
        return Err("operand belongs to a non-prefix body".into());
    }
    if value["kind"].as_str() == Some("body") {
        let min = vector(&value["bounds"]["min"])?;
        let max = vector(&value["bounds"]["max"])?;
        if min.iter().zip(max).any(|(min, max)| *min > max) {
            return Err("unordered operand body bounds".into());
        }
        return Ok(Ok(OperandContextGeometry::Body(OperandBodyBounds {
            input: index,
            body,
            min,
            max,
        })));
    }
    let kind = match value["kind"].as_str() {
        Some("face") => ElementKind::Face,
        Some("edge") => ElementKind::Edge,
        _ => return Err("unsupported operand kind without refusal".into()),
    };
    let element = value["elementId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("operand identity missing")?;
    if value["classification"]["kind"].as_str() != value["kind"].as_str() {
        return Err("operand classification kind changed".into());
    }
    let mut classification = value["classification"].clone();
    classification["present"] = Value::Bool(true);
    if let Some(frame) = classification.get("frame") {
        vector(&frame["origin"])?;
        for key in ["axis", "normal"] {
            if let Some(v) = frame.get(key) {
                direction(v)?;
            }
        }
        if let Some(radius) = frame.get("radius") {
            radius
                .as_f64()
                .filter(|r| r.is_finite() && *r > 0.0)
                .ok_or("invalid analytic radius")?;
        }
    }
    let classification =
        wire::parse_classify_element(&classification).ok_or("operand classification missing")?;
    Ok(Ok(OperandContextGeometry::Element(Box::new(
        OperandGeometry {
            input: index,
            body,
            element: ElementId::new(element),
            kind,
            point: vector(&value["point"])?,
            direction: direction(&value["direction"])?,
            tangent: value.get("tangent").map(direction).transpose()?,
            classification,
        },
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_reordered_and_cross_body_operand_results_fail_closed() {
        let body = BodyId::new();
        let body_wire = wire::body_id_wire(body);
        let primary = json!({"bodyId":body_wire,"elementId":"el_primary","kind":"face"});
        let expected = json!({"primary":primary});
        let result = json!({"inputIndex":0,"bodyId":body_wire,"elementId":"el_primary",
            "kind":"face","point":[1,2,3],"direction":[0,0,1],
            "classification":{"kind":"face","surfaceType":"plane",
                "frame":{"origin":[1,2,3],"normal":[0,0,1]}}});
        assert!(parse_input(0, &result, &expected, &[body]).unwrap().is_ok());
        for (key, value) in [
            ("inputIndex", json!(1)),
            ("elementId", json!("el_secondary")),
            ("bodyId", json!(wire::body_id_wire(BodyId::new()))),
            ("direction", json!([0, 0, 0])),
            ("classification", json!({"kind":"edge"})),
        ] {
            let mut invalid = result.clone();
            invalid[key] = value;
            assert!(
                parse_input(0, &invalid, &expected, &[body]).is_err(),
                "{key}"
            );
        }
        let refusal = json!({"inputIndex":0,"bodyId":body_wire,"elementId":"el_primary",
            "kind":"face","refusal":"unresolved primary"});
        assert_eq!(
            parse_input(0, &refusal, &expected, &[body])
                .unwrap()
                .unwrap_err(),
            "unresolved primary"
        );
    }
    #[test]
    fn body_bounds_are_ordered_finite_and_do_not_require_invented_surface_geometry() {
        let body = BodyId::new();
        let primary = serde_json::json!({"bodyId":wire::body_id_wire(body),
            "elementId":wire::body_id_wire(body),"kind":"body"});
        let expected = serde_json::json!({"primary":primary});
        let mut result = primary;
        result["inputIndex"] = serde_json::json!(0);
        result["bounds"] = serde_json::json!({"min":[-1,-2,-3],"max":[4,5,6]});
        let OperandContextGeometry::Body(bounds) = parse_input(0, &result, &expected, &[body])
            .unwrap()
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(bounds.min, [-1.0, -2.0, -3.0]);
        result["bounds"]["min"] = serde_json::json!([9, 0, 0]);
        assert!(parse_input(0, &result, &expected, &[body]).is_err());
        result["bounds"]["min"] = serde_json::json!([null, 0, 0]);
        assert!(parse_input(0, &result, &expected, &[body]).is_err());
    }
    #[test]
    fn sixty_four_input_cap_does_not_change_operation_commit_contract() {
        for count in [64, 65] {
            let operation: Operation = serde_json::from_value(json!({"opType":"Shell",
                "params":{"thickness":1,"openFaces":vec!["el_face";count]}}))
            .unwrap();
            assert_eq!(validate_operation(&operation).is_ok(), count == 64);
            assert_eq!(
                wire::preview_wire_op(&operation, "op_cap")["inputs"]
                    .as_array()
                    .unwrap()
                    .len(),
                count
            );
        }
    }
}
