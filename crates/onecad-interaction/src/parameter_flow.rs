//! Parameter-set transitions. Every input that changes the candidate takes a new
//! sequence and clears the preview; the single correlated evaluation slot is
//! then refilled with the next pending field, or the set previews when ready.
use crate::{
    parameter_set::DragOrigin, DragPhase, Effect, FieldStatus, Interaction, ModelSession,
    ParameterKey, PreviewStatus, Readiness, SessionPhase,
};

impl Interaction {
    /// Typed input: an open drag is committed first, so typed and dragged edits alternate.
    pub(crate) fn change_parameter(&mut self, key: ParameterKey, text: String) -> Vec<Effect> {
        self.commit_drag();
        self.set_parameter(key, text)
    }

    fn set_parameter(&mut self, key: ParameterKey, text: String) -> Vec<Effect> {
        let Some(tool) = self.editing_tool() else {
            return Vec::new();
        };
        let existing = tool.editing_existing;
        let Some(field) = tool.params.field_mut(key).filter(|field| field.active) else {
            return Vec::new();
        };
        field.status = if existing && text == field.original {
            FieldStatus::Authored
        } else {
            FieldStatus::Pending
        };
        field.text = text;
        tool.params.last_edited = Some(key);
        self.changed(Some(key))
    }

    pub(crate) fn focus_parameter(&mut self, key: ParameterKey) -> Vec<Effect> {
        if let Some(tool) = self.editing_tool() {
            if tool.params.field(key).is_some_and(|field| field.active) {
                tool.params.focus = Some(key);
            }
        }
        Vec::new()
    }

    pub(crate) fn drag(&mut self, phase: DragPhase) -> Vec<Effect> {
        match phase {
            DragPhase::Begin => {
                if let Some(tool) = self.editing_tool() {
                    tool.drag = Some(Box::new(DragOrigin {
                        record: tool.record.clone(),
                        fields: tool.params.fields.clone(),
                        focus: tool.params.focus,
                    }));
                }
                Vec::new()
            }
            DragPhase::Update { key, text } => self.set_parameter(key, text),
            DragPhase::Commit => {
                self.commit_drag();
                Vec::new()
            }
            DragPhase::Cancel => self.cancel_drag(),
        }
    }

    pub(crate) fn commit_drag(&mut self) {
        if let Some(tool) = self.tool.as_mut() {
            tool.drag = None;
        }
    }

    fn cancel_drag(&mut self) -> Vec<Effect> {
        let Some(tool) = self.editing_tool() else {
            return Vec::new();
        };
        let Some(origin) = tool.drag.take() else {
            return Vec::new();
        };
        if tool.record == origin.record
            && tool.params.fields == origin.fields
            && tool.params.focus == origin.focus
        {
            return Vec::new();
        }
        tool.record = origin.record;
        tool.params.fields = origin.fields;
        tool.params.focus = origin.focus;
        self.changed(None)
    }

    /// Re-evaluates the focused field after a base change (choice, target, placement).
    pub(crate) fn refresh(&mut self) -> Vec<Effect> {
        let Some(tool) = self.editing_tool() else {
            return Vec::new();
        };
        let focus = tool.params.focus;
        if let Some(field) = focus.and_then(|key| tool.params.field_mut(key)) {
            field.status = FieldStatus::Pending;
        }
        self.changed(focus)
    }

    /// New sequence, cleared preview, then the next evaluation or the preview.
    pub(crate) fn changed(&mut self, prefer: Option<ParameterKey>) -> Vec<Effect> {
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
        tool.params.op_error = None;
        let mut effects = vec![clear];
        effects.extend(tool.pump(prefer));
        effects
    }

    pub(crate) fn editing_tool(&mut self) -> Option<&mut ModelSession> {
        self.tool
            .as_mut()
            .filter(|tool| tool.phase == SessionPhase::Editing && tool.repair.is_none())
    }
}

impl ModelSession {
    /// Evaluates one pending field under the current sequence, else previews a ready set.
    pub(crate) fn pump(&mut self, prefer: Option<ParameterKey>) -> Vec<Effect> {
        let pending = prefer
            .filter(|key| self.params.is_pending(*key))
            .or_else(|| self.params.first_pending());
        self.params.evaluating = pending;
        if let Some(key) = pending {
            let expression = self
                .params
                .field(key)
                .map(|field| field.text.clone())
                .unwrap_or_default();
            return vec![Effect::EvaluateExpression {
                tag: self.tag.clone(),
                expression,
                site: key.site(),
            }];
        }
        if self.params.readiness() == Readiness::Ready {
            self.request_preview()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn request_preview(&mut self) -> Vec<Effect> {
        if let Some(repair) = &self.repair {
            self.preview = PreviewStatus::Pending;
            return vec![Effect::PreviewInputRepair {
                tag: self.tag.clone(),
                repair: Box::new(repair.clone()),
            }];
        }
        let operation = match self.params.candidate(&self.record.op) {
            Ok(operation) => operation,
            Err(error) => {
                self.params.op_error = Some(error);
                return Vec::new();
            }
        };
        if let Err(message) = crate::extrude::validate_target(&operation) {
            self.preview = PreviewStatus::Failed(message);
            return Vec::new();
        }
        if self.editing_existing {
            if operation == *self.origin {
                self.preview = PreviewStatus::Ready;
                return Vec::new();
            }
            self.preview = PreviewStatus::Pending;
            return vec![Effect::PreviewExistingEdit {
                tag: self.tag.clone(),
                operation: Box::new(operation),
            }];
        }
        self.preview = PreviewStatus::Pending;
        vec![Effect::Preview {
            tag: self.tag.clone(),
            sketch_id: crate::session::profile_sketch(&operation),
            operation: Box::new(operation),
            expected_snapshot: self.tag.context.snapshot_id,
        }]
    }

    /// Writes the legacy single-draft projection of the focused field.
    pub(crate) fn project(&mut self) {
        let Some(key) = self.params.focus() else {
            self.draft.clear();
            self.draft_status = crate::DraftStatus::Pending;
            return;
        };
        let Some(field) = self.params.field(key) else {
            return;
        };
        self.draft = field.text.clone();
        self.draft_status = if let Some(message) = self.params.error(key) {
            crate::DraftStatus::Invalid(message.to_owned())
        } else {
            match &field.status {
                FieldStatus::Authored => crate::DraftStatus::Valid(field.authored_shown()),
                FieldStatus::Pending => crate::DraftStatus::Pending,
                FieldStatus::Valid(scalar) => crate::DraftStatus::Valid(scalar.clone()),
                FieldStatus::Invalid(message) => crate::DraftStatus::Invalid(message.clone()),
            }
        };
    }
}
