//! Extrusion controls replace one correlated candidate while retaining its exact profile.
use crate::{DocumentContext, Effect, Interaction, NumericParameter, ParameterKey, SessionPhase};
use onecad_core::{
    document::record::{BooleanMode, ExtrudeParams, KnownOperation, Operation},
    ids::BodyId,
};

impl Interaction {
    pub(crate) fn change_extrude_mode(&mut self, mode: BooleanMode) -> Vec<Effect> {
        if mode == BooleanMode::Intersect {
            return Vec::new();
        }
        let Some(params) = self.editable_extrude() else {
            return Vec::new();
        };
        if params.boolean_mode == mode {
            return Vec::new();
        }
        params.boolean_mode = mode;
        if mode == BooleanMode::NewBody {
            params.target_body = None;
        }
        self.refresh_extrude()
    }

    pub(crate) fn change_extrude_target(
        &mut self,
        body: Option<BodyId>,
        context: DocumentContext,
    ) -> Vec<Effect> {
        if self.context.as_ref() != Some(&context) {
            return Vec::new();
        }
        let Some(params) = self.editable_extrude() else {
            return Vec::new();
        };
        if params.boolean_mode == BooleanMode::NewBody || params.target_body == body {
            return Vec::new();
        }
        params.target_body = body;
        self.refresh_extrude()
    }

    pub(crate) fn flip_extrude(&mut self) -> Vec<Effect> {
        if self.editable_extrude().is_none() {
            return Vec::new();
        }
        let Some(text) = self
            .tool
            .as_ref()
            .and_then(|tool| tool.params.field(DISTANCE))
            .map(|field| field.text.clone())
        else {
            return Vec::new();
        };
        // Resolve the negated expression through the same application expression service.
        self.change_parameter(DISTANCE, format!("-({text})"))
    }

    /// Choice-like controls commit an open drag before replacing the base.
    fn editable_extrude(&mut self) -> Option<&mut ExtrudeParams> {
        let tool = self.tool.as_mut()?;
        if tool.phase != SessionPhase::Editing
            || tool.editing_existing
            || self.context.as_ref() != Some(&tool.tag.context)
        {
            return None;
        }
        tool.drag = None;
        match &mut tool.record.op {
            Operation::Known(KnownOperation::Extrude(params)) => Some(params),
            _ => None,
        }
    }

    fn refresh_extrude(&mut self) -> Vec<Effect> {
        let Some(tool) = self.tool.as_mut() else {
            return Vec::new();
        };
        tool.record.inputs = tool.record.op.derive_inputs();
        self.refresh()
    }
}

const DISTANCE: ParameterKey = ParameterKey::Scalar(NumericParameter::ExtrudeDistance);

pub(crate) fn validate_target(operation: &Operation) -> Result<(), String> {
    let (mode, target) = match operation {
        Operation::Known(KnownOperation::Extrude(p)) => (p.boolean_mode, p.target_body),
        Operation::Known(KnownOperation::Revolve(p)) => (p.boolean_mode, p.target_body),
        _ => return Ok(()),
    };
    if mode != BooleanMode::NewBody && target.is_none() {
        return Err("Select a visible, healthy target body for Add, Cut or Intersect".into());
    }
    Ok(())
}
