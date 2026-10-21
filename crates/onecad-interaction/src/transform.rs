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
        // Compare against what the session shows, but write only into the base:
        // field overrides (and Authored slots) stay out of it, so a field typed
        // back to its original still commits the authored slot verbatim.
        let effective = tool.effective_op();
        let (
            Operation::Known(KnownOperation::TransformBody(shown)),
            Operation::Known(KnownOperation::TransformBody(base)),
        ) = (&effective, &tool.record.op)
        else {
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
        let axis = Vec3::new_unchecked(axis[0], axis[1], axis[2]);
        let mut placed = shown.clone();
        let mut updated = base.clone();
        let mut moved = shown.rotate.axis != axis;
        placed.rotate.axis = axis;
        updated.rotate.axis = axis;
        let slots = placed
            .translate
            .iter_mut()
            .zip(updated.translate.iter_mut())
            .zip(translate)
            .chain([(
                (&mut placed.rotate.angle_deg, &mut updated.rotate.angle_deg),
                angle_degrees,
            )]);
        for ((shown_slot, base_slot), value) in slots {
            if shown_slot.value != value {
                *shown_slot = Scalar::new(value);
                *base_slot = Scalar::new(value);
                moved = true;
            }
        }
        if !moved {
            return Vec::new();
        }
        if let Err(message) = placed.validate() {
            return self.reject_transform(message);
        }
        let placed = Operation::Known(KnownOperation::TransformBody(placed));
        let operation = Operation::Known(KnownOperation::TransformBody(updated));
        let Some(tool) = self.tool.as_mut() else {
            return Vec::new();
        };
        let existing = tool.editing_existing;
        for field in tool.params.fields.iter_mut().filter(|field| field.active) {
            let parameter = field.key.parameter();
            let Ok(moved) = parameter.current_scalar(&placed) else {
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
