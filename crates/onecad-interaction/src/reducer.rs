use crate::*;

impl Interaction {
    pub fn dispatch(&mut self, action: Action) -> Vec<Effect> {
        let effects = self.route(action);
        if let Some(tool) = self.tool.as_mut() {
            tool.project();
        }
        effects
    }

    fn route(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::BeginOperationEdit {
                record,
                gesture,
                context,
            } => {
                if self.context.as_ref() != Some(&context) {
                    return Vec::new();
                }
                self.begin_operation_edit(*record, gesture)
            }
            Action::ContextChanged(context) => self.change_context(context),
            Action::ClearSelection => {
                self.selection = None;
                Vec::new()
            }
            Action::SelectProfile(selection) => {
                if self.context.as_ref() == Some(&selection.context)
                    && selection.capture.scene == selection.context.scene
                {
                    self.selection = Some(*selection);
                }
                Vec::new()
            }
            Action::BeginExtrude {
                record_id,
                gesture,
                step_index,
            } => self.begin_extrude(record_id, gesture, step_index),
            Action::BeginOperation {
                record,
                fields,
                focus,
                gesture,
                context,
            } => {
                if self.context.as_ref() != Some(&context) {
                    return Vec::new();
                }
                self.begin_operation(*record, fields, focus, gesture)
            }
            Action::BeginEdit {
                record,
                gesture,
                context,
            } => {
                if self.context.as_ref() != Some(&context) {
                    return Vec::new();
                }
                self.begin_edit(*record, gesture)
            }
            Action::BeginParameterEdit {
                record,
                parameter,
                gesture,
                context,
            } => {
                if self.context.as_ref() != Some(&context) {
                    return Vec::new();
                }
                self.begin_parameter_edit(*record, parameter, gesture)
            }
            Action::DraftChanged(draft) => self.change_draft(draft),
            Action::ParameterChanged { key, text } => self.change_parameter(key, text),
            Action::FocusParameter(key) => self.focus_parameter(key),
            Action::ParameterDrag(phase) => self.drag(phase),
            Action::ModelingOptionsChanged { tag, options } => {
                self.change_modeling_options(tag, options)
            }
            Action::ExtrudeModeChanged(mode) => self.change_extrude_mode(mode),
            Action::ExtrudeTargetChanged { body, context } => {
                self.change_extrude_target(body, context)
            }
            Action::FlipExtrude => self.flip_extrude(),
            Action::TransformDraftChanged {
                translate,
                axis,
                angle_degrees,
            } => self.change_transform(translate, axis, angle_degrees),
            Action::Evaluated { tag, result } => self.evaluated(tag, result),
            Action::PreviewFinished { tag, result } => self.preview_finished(tag, result),
            Action::Confirm => self.confirm(),
            Action::Cancel => self.cancel(),
            Action::Republish => self.republish(),
            Action::CommitAccepted {
                tag,
                revision,
                rollback_token,
            } => self.accepted(tag, revision, rollback_token),
            Action::CommitRejected { tag, message } => self.rejected(tag, message),
            Action::CompletionUnavailable { tag, message } => {
                self.completion_unavailable(tag, message)
            }
            Action::RegenFinished { tag, completion } => self.completed(tag, completion),
        }
    }

    fn change_context(&mut self, context: Option<DocumentContext>) -> Vec<Effect> {
        if self.context == context {
            return Vec::new();
        }
        let same_runtime = self
            .context
            .as_ref()
            .zip(context.as_ref())
            .is_some_and(|(old, next)| old.same_runtime(next));
        if same_runtime
            && self
                .context
                .as_ref()
                .zip(context.as_ref())
                .is_some_and(|(old, next)| next.revision < old.revision)
        {
            return Vec::new();
        }
        let pending = self.tool.as_ref().is_some_and(|tool| {
            matches!(
                tool.phase,
                SessionPhase::Committing | SessionPhase::Accepted | SessionPhase::Completed
            )
        });
        self.context = context;
        self.selection = None;
        if same_runtime && pending {
            return Vec::new();
        }
        self.status = CommitStatus::Idle;
        self.deferred_completion = None;
        self.tool
            .take()
            .map(|tool| {
                vec![Effect::ClearPreview {
                    gesture: tool.tag.gesture,
                }]
            })
            .unwrap_or_default()
    }

    pub(crate) fn matches_editing(&self, tag: &RequestTag) -> bool {
        self.tool
            .as_ref()
            .is_some_and(|tool| tool.tag == *tag && tool.phase == SessionPhase::Editing)
            && self.context.as_ref() == Some(&tag.context)
    }

    pub(crate) fn matches_commit(&self, tag: &RequestTag, phase: SessionPhase) -> bool {
        self.tool
            .as_ref()
            .is_some_and(|tool| tool.tag == *tag && tool.phase == phase)
            && self
                .context
                .as_ref()
                .is_some_and(|context| context.same_runtime(&tag.context))
    }
}
