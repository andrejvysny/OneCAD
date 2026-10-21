//! Capability fence for the SCHEMA §7.3 Boolean array form (ONECAD-362).
//!
//! The array shape already fences itself on an old worker (absent `toolBodyId`
//! refuses `REF_UNRESOLVED`); this gate turns that misleading refusal into an
//! attributable protocol error on the `ONECAD_WORKER_PATH` dev seam, the only
//! place a new app can meet an old worker.

use onecad_core::document::record::{KnownOperation, Operation};
use onecad_core::regen::{EngineError, PlanRequest};

pub(super) const CAPABILITY: &str = "op.boolean.tools";

fn needs_tools_array(op: &Operation) -> bool {
    matches!(op, Operation::Known(KnownOperation::Boolean(p)) if p.uses_tools_array())
}

fn require(required: bool, capabilities: &[String]) -> Result<(), EngineError> {
    if required && !capabilities.iter().any(|value| value == CAPABILITY) {
        return Err(EngineError::Protocol {
            message: format!(
                "Boolean with multiple or retained tools requires worker capability {CAPABILITY}"
            ),
        });
    }
    Ok(())
}

pub(super) fn plan(request: &PlanRequest, capabilities: &[String]) -> Result<(), EngineError> {
    require(
        request
            .ops
            .iter()
            .any(|op| needs_tools_array(&op.operation)),
        capabilities,
    )
}

pub(super) fn operation(op: &Operation, capabilities: &[String]) -> Result<(), EngineError> {
    require(needs_tools_array(op), capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::document::record::{BooleanOp, BooleanParams};
    use onecad_core::ids::BodyId;

    fn boolean(tools: usize, keep: bool) -> Operation {
        let tools = (0..tools).map(|_| BodyId::new()).collect();
        Operation::Known(KnownOperation::Boolean(BooleanParams::new(
            BooleanOp::Cut,
            BodyId::new(),
            tools,
            keep,
        )))
    }

    #[test]
    fn array_form_requires_capability_legacy_does_not() {
        let with = [CAPABILITY.to_string()];
        assert!(operation(&boolean(1, false), &[]).is_ok());
        for op in [boolean(2, false), boolean(1, true), boolean(3, true)] {
            let error = operation(&op, &[]).unwrap_err().to_string();
            assert!(error.contains(CAPABILITY), "{error}");
            assert!(operation(&op, &with).is_ok());
            assert!(operation(&op, &["op.boolean".into()]).is_err());
        }
    }
}
