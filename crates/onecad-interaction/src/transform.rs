//! Gizmo drafts replace placement against frozen targets and pivot, never accumulate deltas.
use crate::{Effect, FieldStatus, Interaction, PreviewStatus, SessionPhase};
use onecad_core::{
    document::{
        record::{KnownOperation, Operation},
        variables::Scalar,
    },
    math::Vec3,
};

impl Interaction {
    /// Drag-sourced placement: it never commits an open parameter drag.
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
        // Compare against what the session shows, so evaluated overrides (and
        // their expressions) survive motion on other components.
        let effective = tool.effective_op();
        let Operation::Known(KnownOperation::TransformBody(params)) = &effective else {
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
        let Some(tool) = self.tool.as_mut() else {
            return Vec::new();
        };
        let existing = tool.editing_existing;
        for field in tool.params.fields.iter_mut().filter(|field| field.active) {
            let parameter = field.key.parameter();
            let Ok(moved) = parameter.current_scalar(&operation) else {
                continue;
            };
            if parameter.current_scalar(&effective).ok().as_ref() != Some(&moved) {
                field.text = moved.value.to_string();
                field.status = if existing && field.text == field.original {
                    FieldStatus::Authored
                } else {
                    FieldStatus::Pending
                };
            }
        }
        tool.record.op = operation;
        self.refresh()
    }

    fn reject_transform(&mut self, message: String) -> Vec<Effect> {
        let Some(tool) = self
            .tool
            .as_mut()
            .filter(|tool| tool.phase == SessionPhase::Editing)
        else {
            return Vec::new();
        };
        let clear = Effect::ClearPreview {
            gesture: tool.tag.gesture,
        };
        let Some(sequence) = self.sequence.checked_add(1) else {
            tool.preview = PreviewStatus::Failed("Interaction sequence exhausted".into());
            return vec![clear];
        };
        self.sequence = sequence;
        tool.tag.sequence = sequence;
        tool.preview = PreviewStatus::Empty;
        tool.params.evaluating = None;
        tool.params.op_error = Some((tool.params.focus, message));
        vec![clear]
    }
}

fn replace_scalar(scalar: &mut Scalar, value: f64) {
    if scalar.value != value {
        *scalar = Scalar::new(value);
    }
}
