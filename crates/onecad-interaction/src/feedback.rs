use crate::*;
use onecad_core::document::variables::Scalar;

impl Interaction {
    pub(crate) fn evaluated(
        &mut self,
        tag: RequestTag,
        result: Result<EvaluatedExpressionDto, String>,
    ) -> Vec<Effect> {
        if !self.matches_editing(&tag) {
            return Vec::new();
        }
        let Some(tool) = self.tool.as_mut() else {
            return Vec::new();
        };
        if tool.draft_status != DraftStatus::Pending {
            return Vec::new();
        }
        let scalar = match validated_scalar(tool.parameter, &tool.draft, result) {
            Ok(scalar) => scalar,
            Err(message) => {
                tool.draft_status = DraftStatus::Invalid(message);
                return Vec::new();
            }
        };
        let mut operation = tool.record.op.clone();
        if let Err(message) = tool.parameter.apply(&mut operation, scalar.clone()) {
            tool.draft_status = DraftStatus::Invalid(message);
            return Vec::new();
        }
        tool.draft_status = DraftStatus::Valid(scalar);
        if tool.editing_existing {
            tool.preview = PreviewStatus::UnavailableForEdit;
            return Vec::new();
        }
        let sketch_id = crate::session::profile_sketch(&operation);
        tool.preview = PreviewStatus::Pending;
        vec![Effect::Preview {
            tag: tag.clone(),
            sketch_id,
            operation: Box::new(operation),
            expected_snapshot: tag.context.snapshot_id,
        }]
    }

    pub(crate) fn preview_finished(
        &mut self,
        tag: RequestTag,
        result: Result<PreviewResultDto, String>,
    ) -> Vec<Effect> {
        if !self.matches_editing(&tag) {
            return Vec::new();
        }
        let Some(tool) = self.tool.as_mut() else {
            return Vec::new();
        };
        if tool.preview != PreviewStatus::Pending
            || !matches!(tool.draft_status, DraftStatus::Valid(_))
        {
            return Vec::new();
        }
        match result {
            Ok(result) if result.snapshot_id == tag.context.snapshot_id.0 => {
                tool.preview = if result.needs_repair.is_empty() {
                    PreviewStatus::Ready
                } else {
                    PreviewStatus::NeedsRepair
                };
                vec![Effect::PublishPreview {
                    tag,
                    result: Box::new(result),
                }]
            }
            Ok(_) => {
                tool.preview = PreviewStatus::Failed("Preview snapshot changed".into());
                Vec::new()
            }
            Err(message) => {
                tool.preview = PreviewStatus::Failed(message);
                Vec::new()
            }
        }
    }

    pub(crate) fn accepted(
        &mut self,
        tag: RequestTag,
        revision: u64,
        rollback_token: Option<String>,
    ) -> Vec<Effect> {
        if !self.matches_commit(&tag, SessionPhase::Committing) || revision < tag.context.revision {
            return Vec::new();
        }
        if let Some(tool) = self.tool.as_mut() {
            tool.phase = SessionPhase::Accepted;
        }
        self.status = CommitStatus::Accepted {
            tag,
            revision,
            rollback_token,
        };
        if let Some((tag, completion)) = self.deferred_completion.take() {
            return self.completed(tag, completion);
        }
        Vec::new()
    }

    pub(crate) fn rejected(&mut self, tag: RequestTag, message: String) -> Vec<Effect> {
        if !self.matches_commit(&tag, SessionPhase::Committing) {
            return Vec::new();
        }
        if let Some(tool) = self.tool.as_mut() {
            tool.phase = SessionPhase::Editing;
        }
        self.status = CommitStatus::Rejected(message);
        self.deferred_completion = None;
        Vec::new()
    }

    pub(crate) fn completion_unavailable(
        &mut self,
        tag: RequestTag,
        message: String,
    ) -> Vec<Effect> {
        if !self.matches_commit(&tag, SessionPhase::Accepted) {
            return Vec::new();
        }
        let CommitStatus::Accepted {
            revision,
            rollback_token,
            ..
        } = &self.status
        else {
            return Vec::new();
        };
        self.status = CommitStatus::CompletionUnavailable {
            tag,
            accepted_revision: *revision,
            rollback_token: rollback_token.clone(),
            message,
        };
        if let Some(tool) = self.tool.as_mut() {
            tool.phase = SessionPhase::Completed;
        }
        self.deferred_completion = None;
        Vec::new()
    }

    pub(crate) fn completed(
        &mut self,
        tag: RequestTag,
        completion: Box<RegenFinished>,
    ) -> Vec<Effect> {
        if self.matches_commit(&tag, SessionPhase::Committing) {
            self.defer_completion(tag, completion);
            return Vec::new();
        }
        if !self.matches_commit(&tag, SessionPhase::Accepted) {
            return Vec::new();
        }
        let CommitStatus::Accepted {
            revision,
            rollback_token,
            ..
        } = &self.status
        else {
            return Vec::new();
        };
        if completion.runtime_session != tag.context.runtime_session
            || completion.source_revision < *revision
            || completion.outcome == "superseded"
        {
            return Vec::new();
        }
        let outcome = feature_outcome(&tag, &completion);
        self.status = CommitStatus::Completed {
            tag,
            accepted_revision: *revision,
            rollback_token: rollback_token.clone(),
            outcome,
            completion,
        };
        if let Some(tool) = self.tool.as_mut() {
            tool.phase = SessionPhase::Completed;
        }
        Vec::new()
    }

    fn defer_completion(&mut self, tag: RequestTag, completion: Box<RegenFinished>) {
        if completion.runtime_session == tag.context.runtime_session
            && completion.outcome != "superseded"
            && self
                .deferred_completion
                .as_ref()
                .is_none_or(|(_, previous)| completion.source_revision >= previous.source_revision)
        {
            self.deferred_completion = Some((tag, completion));
        }
    }
}

fn validated_scalar(
    parameter: NumericParameter,
    draft: &str,
    result: Result<EvaluatedExpressionDto, String>,
) -> Result<Scalar, String> {
    let result = result?;
    if let Some(error) = result.error {
        return Err(error);
    }
    if (result.dimension != parameter.site() && result.dimension != "scalar")
        || !result.value.is_finite()
        || draft.trim().is_empty()
    {
        return Err(format!("Enter a finite {} expression", parameter.site()));
    }
    Ok(Scalar::with_expr(result.value, draft))
}

fn feature_outcome(tag: &RequestTag, completion: &RegenFinished) -> FeatureOutcome {
    let id = tag.record_id.to_string();
    if let Some(failed) = completion
        .failed_steps
        .iter()
        .find(|step| step.record_id == id)
    {
        return FeatureOutcome::Failed(failed.message.clone());
    }
    if completion.repair_steps.contains(&id) {
        return FeatureOutcome::NeedsRepair;
    }
    match completion.outcome.as_str() {
        "published" => FeatureOutcome::Published,
        "noop" => FeatureOutcome::Noop,
        "cancelled" => FeatureOutcome::Cancelled,
        other => FeatureOutcome::Failed(
            completion
                .message
                .clone()
                .unwrap_or_else(|| format!("Regen ended with {other}")),
        ),
    }
}
