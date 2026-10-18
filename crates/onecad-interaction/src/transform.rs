//! Gizmo drafts replace placement against frozen targets and pivot, never accumulate deltas.
use crate::{DraftStatus, Effect, Interaction, PreviewStatus, SessionPhase};
use onecad_core::{
    document::{
        record::{KnownOperation, Operation},
        variables::Scalar,
    },
    math::Vec3,
};

impl Interaction {
    pub(crate) fn change_transform(
        &mut self,
        translate: [f64; 3],
        axis: [f64; 3],
        angle_degrees: f64,
    ) -> Vec<Effect> {
        let Some(tool) = self.tool.as_ref().filter(|tool| {
            tool.phase == SessionPhase::Editing && self.context.as_ref() == Some(&tool.tag.context)
        }) else {
            return Vec::new();
        };
        let Operation::Known(KnownOperation::TransformBody(params)) = &tool.record.op else {
            return Vec::new();
        };
        if !translate
            .iter()
            .chain(axis.iter())
            .all(|value| value.is_finite())
            || !angle_degrees.is_finite()
        {
            return self.reject_transform("Transform draft components must be finite".into());
        }
        let mut updated = params.clone();
        for (scalar, value) in updated.translate.iter_mut().zip(translate) {
            replace_scalar(scalar, value);
        }
        updated.rotate.axis = Vec3::new_unchecked(axis[0], axis[1], axis[2]);
        replace_scalar(&mut updated.rotate.angle_deg, angle_degrees);
        if let Err(message) = updated.validate() {
            return self.reject_transform(message);
        }
        if updated == *params {
            return Vec::new();
        }
        let operation = Operation::Known(KnownOperation::TransformBody(updated));
        let Ok(scalar) = tool.parameter.current_scalar(&operation) else {
            return Vec::new();
        };
        let draft = scalar.expr.unwrap_or_else(|| scalar.value.to_string());
        if let Some(tool) = self.tool.as_mut() {
            tool.record.op = operation;
        }
        self.change_draft(draft)
    }

    fn reject_transform(&mut self, message: String) -> Vec<Effect> {
        let mut effects = self.change_draft(String::new());
        effects.retain(|effect| matches!(effect, Effect::ClearPreview { .. }));
        if let Some(tool) = self.tool.as_mut() {
            tool.draft_status = DraftStatus::Invalid(message);
            tool.preview = PreviewStatus::Empty;
        }
        effects
    }
}

fn replace_scalar(scalar: &mut Scalar, value: f64) {
    if scalar.value != value {
        *scalar = Scalar::new(value);
    }
}
