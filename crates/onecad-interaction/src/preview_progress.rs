//! An exact request owns its tag until its terminal, independently of newer scalar drafts.
use crate::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct PreviewProgress {
    pub submitted: Option<RequestTag>,
    pub displayed: Option<RequestTag>,
    pub validated: Option<RequestTag>,
}

impl Interaction {
    pub(crate) fn matches_submitted_preview(&self, tag: &RequestTag) -> bool {
        self.tool.as_ref().is_some_and(|tool| {
            tool.phase == SessionPhase::Editing
                && tool.progress.submitted.as_ref() == Some(tag)
                && tool.tag.context == tag.context
                && tool.tag.gesture == tag.gesture
                && tool.tag.record_id == tag.record_id
        }) && self.context.as_ref() == Some(&tag.context)
    }

    pub(crate) fn finish_preview(
        &mut self,
        tag: RequestTag,
        status: Option<PreviewStatus>,
        geometry: Option<PreviewResultDto>,
    ) -> Vec<Effect> {
        if !self.matches_submitted_preview(&tag) {
            return Vec::new();
        }
        let Some(tool) = self.tool.as_mut() else {
            return Vec::new();
        };
        tool.progress.submitted = None;
        let current = tool.tag == tag;
        if current {
            if let Some(status) = status {
                tool.preview = status;
                tool.progress.validated = Some(tag.clone());
            }
        }
        let mut effects = Vec::new();
        if let Some(result) = geometry.filter(|_| {
            tool.progress
                .displayed
                .as_ref()
                .is_none_or(|shown| shown.sequence < tag.sequence)
        }) {
            tool.progress.displayed = Some(tag.clone());
            effects.push(Effect::PublishPreview {
                tag,
                result: Box::new(result),
            });
        }
        // Only a newer desired draft is pumped: cancellation/errors of the current
        // request must not create an automatic retry loop.
        if !current {
            effects.extend(tool.pump(None));
        }
        effects
    }
}
