use super::*;
use onecad_app::dto::{CancelSketchDto, FinishSketchDto, SketchSessionDto, SketchUpsertDto};
use onecad_core::{
    math::Vec2,
    sketch::{Sketch, SketchEntity},
};

impl SketchInteraction {
    pub(crate) fn created(&mut self, tag: SketchTag) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) || !self.phase_is(SketchPhase::Creating) {
            return Vec::new();
        }
        let Some(tag) = self.next_request() else {
            return Vec::new();
        };
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        session.phase = SketchPhase::Entering;
        vec![SketchEffect::EnterSketch {
            tag,
            created_here: session.created_here,
        }]
    }

    pub(crate) fn entered(
        &mut self,
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: SketchSessionDto,
    ) -> Vec<SketchEffect> {
        self.adopt_session(tag, snapshot, solve, SketchPhase::Entering)
    }

    pub(crate) fn refreshed(
        &mut self,
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: SketchSessionDto,
    ) -> Vec<SketchEffect> {
        self.adopt_session(tag, snapshot, solve, SketchPhase::NeedsRefresh)
    }

    fn adopt_session(
        &mut self,
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: SketchSessionDto,
        phase: SketchPhase,
    ) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag)
            || !self.phase_is(phase)
            || snapshot.sketch.id != tag.key.sketch_id
            || solve.sketch_id != tag.key.sketch_id.to_string()
        {
            return Vec::new();
        }
        if self.context.as_ref() != Some(&snapshot.document)
            || !super::geometry::valid_plane(snapshot.sketch.effective_plane())
        {
            return self.failed(tag, "Entered sketch snapshot is no longer current".into());
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        session.plane = snapshot.sketch.effective_plane();
        session.history = snapshot.history;
        session.authoritative = *snapshot.sketch;
        session.edit_session = solve.edit_session.clone();
        session.status = SketchStatus::Entered(Box::new(solve));
        session.phase = SketchPhase::Active;
        Vec::new()
    }

    pub(crate) fn batch_accepted(&mut self, tag: SketchTag) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) || !self.phase_is(SketchPhase::Submitting) {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        let Some(batch) = session.batch.as_mut().filter(|batch| batch.tag == tag) else {
            return Vec::new();
        };
        session.phase = SketchPhase::Accepted;
        session.status = SketchStatus::Accepted;
        let deferred = batch.deferred.take();
        if let Some((snapshot, solve)) = deferred {
            return self.solved(tag, snapshot, solve);
        }
        Vec::new()
    }

    pub(crate) fn solved(
        &mut self,
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: SketchUpsertDto,
    ) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag)
            || !matches!(
                self.session.as_ref().map(|s| s.phase),
                Some(SketchPhase::Submitting | SketchPhase::Accepted)
            )
            || snapshot.sketch.id != tag.key.sketch_id
            || solve.sketch_id != tag.key.sketch_id.to_string()
        {
            return Vec::new();
        }
        if self.context.as_ref() != Some(&snapshot.document) {
            return self.failed(tag, "Solved sketch snapshot is no longer current".into());
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if let Err(message) = validate_snapshot(session, &snapshot) {
            return self.failed(tag, message.into());
        }
        if session.phase == SketchPhase::Submitting {
            if let Some(batch) = &mut session.batch {
                batch.deferred = Some((snapshot, solve));
            }
            return Vec::new();
        }
        let Some(batch) = session.batch.take() else {
            return Vec::new();
        };
        session.history = snapshot.history;
        session.authoritative = *snapshot.sketch;
        session
            .selection
            .retain(|target| target.is_valid(&session.authoritative));
        session.preview = None;
        if let Err(message) = apply_after(session, batch.after) {
            return self.failed(tag, message);
        }
        session.phase = SketchPhase::Active;
        session.status = SketchStatus::Solved(Box::new(solve));
        session
            .pending_exit
            .take()
            .map(|exit| self.exit(exit))
            .unwrap_or_default()
    }

    pub(crate) fn history_resolved(
        &mut self,
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: SketchSessionDto,
        moved: bool,
        direction: onecad_core::edit::HistoryDirection,
    ) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag)
            || !self.phase_is(SketchPhase::Submitting)
            || self.context.as_ref() != Some(&snapshot.document)
            || snapshot.sketch.id != tag.key.sketch_id
            || solve.sketch_id != tag.key.sketch_id.to_string()
        {
            return Vec::new();
        }
        let Some(session) = self
            .session
            .as_mut()
            .filter(|session| session.batch.is_none())
        else {
            return Vec::new();
        };
        if session.edit_session != solve.edit_session
            || snapshot.sketch.effective_plane() != session.plane
        {
            return self.failed(
                tag,
                "Sketch history: edit session or sketch plane changed".into(),
            );
        }
        session.history = snapshot.history;
        session.authoritative = *snapshot.sketch;
        session
            .selection
            .retain(|target| target.is_valid(&session.authoritative));
        session.phase = SketchPhase::Active;
        session.status = if moved {
            SketchStatus::Entered(Box::new(solve))
        } else {
            SketchStatus::Refused(format!(
                "No sketch edits to {}",
                if direction == onecad_core::edit::HistoryDirection::Undo {
                    "undo"
                } else {
                    "redo"
                }
            ))
        };
        session
            .pending_exit
            .take()
            .map(|exit| self.exit(exit))
            .unwrap_or_default()
    }

    pub(crate) fn constraint_refused(
        &mut self,
        tag: SketchTag,
        message: String,
    ) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) || !self.phase_is(SketchPhase::Submitting) {
            return Vec::new();
        }
        let pending = self.session.as_mut().and_then(|session| {
            session.phase = SketchPhase::Active;
            session.status = SketchStatus::Refused(message);
            session.batch = None;
            session.pending_exit.take()
        });
        pending.map(|exit| self.exit(exit)).unwrap_or_default()
    }

    pub(crate) fn failed(&mut self, tag: SketchTag, message: String) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if matches!(
            session.phase,
            SketchPhase::Closed | SketchPhase::Active | SketchPhase::Faulted
        ) {
            return Vec::new();
        }
        let accepted = (session.phase == SketchPhase::Accepted).then_some(true);
        session.status = SketchStatus::Failed { accepted, message };
        session.phase = SketchPhase::Faulted;
        session.draft = None;
        session.preview = None;
        session.batch = None;
        Vec::new()
    }

    pub(crate) fn finish_refused(&mut self, tag: SketchTag, message: String) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) || !self.phase_is(SketchPhase::Finishing) {
            return Vec::new();
        }
        let effects = self.failed(tag, message);
        if let Some(session) = self.session.as_mut() {
            session.repair_reentry = true;
            session.edit_session = None;
            session.history = None;
        }
        effects
    }

    pub(crate) fn finished(
        &mut self,
        tag: SketchTag,
        result: Box<FinishSketchDto>,
    ) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) || !self.phase_is(SketchPhase::Finishing) {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        session.phase = SketchPhase::Closed;
        session.selection.clear();
        session.status = SketchStatus::Finished {
            result,
            kept_reason: session.kept_reason.take(),
        };
        Vec::new()
    }

    pub(crate) fn discarded(
        &mut self,
        tag: SketchTag,
        result: CancelSketchDto,
    ) -> Vec<SketchEffect> {
        if !self.valid_tag(&tag) || !self.phase_is(SketchPhase::Discarding) {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if let Some(reason) = result.kept_reason {
            session.kept_reason = Some(reason);
            session.phase = SketchPhase::Active;
            return self.exit(ExitIntent::Finish(FinishReason::DiscardRefused));
        }
        session.phase = SketchPhase::Closed;
        session.status = SketchStatus::Discarded(result);
        Vec::new()
    }

    fn phase_is(&self, phase: SketchPhase) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.phase == phase)
    }
}

fn validate_snapshot(
    session: &SketchSession,
    snapshot: &SketchSnapshot,
) -> Result<(), &'static str> {
    if snapshot.sketch.effective_plane() != session.plane {
        return Err("Sketch plane changed during solve");
    }
    if session.batch.as_ref().is_none_or(|batch| {
        batch
            .entities
            .iter()
            .any(|id| !snapshot.sketch.contains_entity(*id))
            || batch
                .constraints
                .iter()
                .any(|id| !snapshot.sketch.contains_constraint(*id))
    }) {
        return Err("Solved sketch snapshot is missing accepted batch geometry");
    }
    Ok(())
}

fn apply_after(session: &mut SketchSession, after: AfterPrimitive) -> Result<(), String> {
    match after {
        AfterPrimitive::Reset => session.draft = None,
        AfterPrimitive::ContinueLine {
            first,
            last,
            segments,
        } => {
            let a = point(&session.authoritative, first).ok_or("Solved chain start is missing")?;
            let b =
                point(&session.authoritative, last).ok_or("Solved chain endpoint is missing")?;
            session.draft = Some(SketchDraft {
                first: a,
                anchor: b,
                first_id: Some(first),
                anchor_id: Some(last),
                cursor: b,
                segments,
                anchor_snap: None,
                secondary: None,
            });
        }
        AfterPrimitive::Preserve => {
            if let Some(draft) = &mut session.draft {
                if let Some(id) = draft.first_id {
                    draft.first =
                        point(&session.authoritative, id).ok_or("Solved chain start is missing")?;
                }
                if let Some(id) = draft.anchor_id {
                    draft.anchor = point(&session.authoritative, id)
                        .ok_or("Solved chain endpoint is missing")?;
                }
                draft.cursor = draft.anchor;
            }
        }
    }
    Ok(())
}

fn point(sketch: &Sketch, id: onecad_core::ids::EntityId) -> Option<Vec2> {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => Some(*at),
        _ => None,
    }
}
