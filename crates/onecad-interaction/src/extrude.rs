//! Extrusion controls replace one correlated candidate while retaining its exact profile.
use crate::{DocumentContext, Effect, Interaction, NumericParameter, SessionPhase};
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
        let Some(tool) = self.tool.as_ref() else {
            return Vec::new();
        };
        // Resolve the negated expression through the same application expression service.
        let draft = format!("-({})", tool.draft);
        self.change_draft(draft)
    }

    fn editable_extrude(&mut self) -> Option<&mut ExtrudeParams> {
        let tool = self.tool.as_mut()?;
        if tool.phase != SessionPhase::Editing
            || tool.editing_existing
            || tool.parameter != NumericParameter::ExtrudeDistance
            || self.context.as_ref() != Some(&tool.tag.context)
        {
            return None;
        }
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
        let draft = tool.draft.clone();
        self.change_draft(draft)
    }
}

pub(crate) fn validate_target(operation: &Operation) -> Result<(), String> {
    if let Operation::Known(KnownOperation::Extrude(params)) = operation {
        if params.boolean_mode != BooleanMode::NewBody && params.target_body.is_none() {
            return Err("Select a visible, healthy target body for Add or Cut".into());
        }
    }
    Ok(())
}
