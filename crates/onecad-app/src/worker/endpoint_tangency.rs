use onecad_core::document::record::{KnownOperation, Operation};
use onecad_core::regen::{EngineError, PlanRequest};
use onecad_core::sketch::{Constraint, Sketch};

pub(super) const CAPABILITY: &str = "constraint.tangentAtPoint";

fn require(required: bool, capabilities: &[String]) -> Result<(), EngineError> {
    if required && !capabilities.iter().any(|value| value == CAPABILITY) {
        return Err(EngineError::Protocol {
            message: format!("TangentAtPoint requires worker capability {CAPABILITY}"),
        });
    }
    Ok(())
}

pub(super) fn sketch(sketch: &Sketch, capabilities: &[String]) -> Result<(), EngineError> {
    sketch
        .validate_endpoint_tangencies()
        .map_err(|error| EngineError::Protocol {
            message: error.to_string(),
        })?;
    require(
        sketch
            .constraints()
            .iter()
            .any(|c| matches!(c, Constraint::TangentAtPoint { .. })),
        capabilities,
    )
}

pub(super) fn plan(request: &PlanRequest, capabilities: &[String]) -> Result<(), EngineError> {
    let required = request.ops.iter().any(|op| {
        let Operation::Known(KnownOperation::Sketch(params)) = &op.operation else {
            return false;
        };
        params
            .constraints
            .iter()
            .any(|c| c.get("type").and_then(|v| v.as_str()) == Some("TangentAtPoint"))
    });
    require(required, capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_capability_is_required_only_for_new_kind() {
        assert!(require(false, &[]).is_ok());
        let error = require(true, &[]).unwrap_err().to_string();
        assert!(error.contains("TangentAtPoint"));
        assert!(error.contains(CAPABILITY));
        assert!(require(true, &[CAPABILITY.into()]).is_ok());
        assert!(require(true, &["Tangent".into()]).is_err());
    }
    #[test]
    fn wire_keeps_endpoint_owners_roles_and_identity_without_legacy_drift() {
        use onecad_core::{
            ids::{ConstraintId, EntityId, SketchId},
            math::Vec2,
            sketch::{CurvePosition, SketchEntity, WorldPlane},
        };
        use serde_json::json;
        let mut sketch = Sketch::on_world_plane(SketchId::new(), "wire", WorldPlane::XY);
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let first = EntityId::new();
        let second = EntityId::new();
        let id = ConstraintId::new();
        for (entity, x) in [(a, 0.0), (b, 1.0), (c, -1.0)] {
            sketch
                .add_entity(SketchEntity::point(
                    entity,
                    Vec2::new_unchecked(x, 0.0),
                    false,
                    false,
                ))
                .unwrap();
        }
        sketch
            .add_entity(SketchEntity::line(first, a, b, false))
            .unwrap();
        sketch
            .add_entity(SketchEntity::line(second, c, a, false))
            .unwrap();
        let legacy = Constraint::Tangent {
            id: ConstraintId::new(),
            entity1: first,
            entity2: second,
        };
        sketch.add_constraint(legacy.clone()).unwrap();
        let constraint = Constraint::TangentAtPoint {
            id,
            entity1: first,
            entity2: second,
            entity1_position: CurvePosition::Start,
            entity2_position: CurvePosition::End,
        };
        sketch.add_constraint(constraint.clone()).unwrap();
        assert!(super::sketch(&sketch, &[]).is_err());
        assert!(super::sketch(&sketch, &[CAPABILITY.into()]).is_ok());
        let (_, _, wire) = super::super::wire::sketch_wire(&sketch);
        assert_eq!(
            wire[0],
            json!({"id":legacy.id().to_string(),"type":"Tangent",
            "entities":[first.to_string(),second.to_string()]})
        );
        assert_eq!(
            wire[1],
            json!({"id":id.to_string(),"type":"TangentAtPoint",
            "entities":[first.to_string(),second.to_string()],"positions":["Start","End"]})
        );
        assert_eq!(
            serde_json::from_value::<Constraint>(serde_json::to_value(&constraint).unwrap())
                .unwrap(),
            constraint
        );
    }
}
