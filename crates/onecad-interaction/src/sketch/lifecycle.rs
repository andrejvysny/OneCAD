use super::*;
use crate::DocumentContext;
use onecad_core::{edit::EditCommand, sketch::Sketch};

impl SketchInteraction {
    pub fn dispatch(&mut self, action: SketchAction) -> Vec<SketchEffect> {
        match action {
            SketchAction::ContextChanged(context) => self.context_changed(context),
            SketchAction::Begin {
                sketch,
                create,
                context,
            } => self.begin_captured(*sketch, create, context),
            SketchAction::BeginCreated { sketch, context } => {
                let effects = self.begin_captured(*sketch, true, context);
                match effects.into_iter().next() {
                    Some(SketchEffect::CreateSketch { tag, .. }) => {
                        if let Some(session) = self.session.as_mut() {
                            session.phase = SketchPhase::Entering;
                        }
                        vec![SketchEffect::EnterSketch {
                            tag,
                            created_here: true,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            SketchAction::Created { tag } => self.created(tag),
            SketchAction::Entered {
                tag,
                snapshot,
                solve,
            } => self.entered(tag, snapshot, *solve),
            SketchAction::Refreshed {
                tag,
                snapshot,
                solve,
            } => self.refreshed(tag, snapshot, *solve),
            SketchAction::SetTool(tool) => {
                self.set_tool(tool);
                Vec::new()
            }
            SketchAction::SetConstructionMode { key, construction } => {
                if let Some(session) = self
                    .session
                    .as_mut()
                    .filter(|session| session.key == key && session.phase == SketchPhase::Active)
                {
                    session.construction = construction;
                }
                Vec::new()
            }
            SketchAction::SetTangentArc { key, enabled } => {
                self.set_tangent_arc(key, enabled);
                Vec::new()
            }
            SketchAction::Move(point) => {
                self.moved(point);
                Vec::new()
            }
            SketchAction::Click { point, ids } => self.clicked(point, ids),
            SketchAction::ClickInferred { point, ids, snap } => {
                self.clicked_inferred(point, ids, snap.map(|snap| *snap))
            }
            SketchAction::BeginPointNumeric { key } => {
                self.begin_point_numeric(key);
                Vec::new()
            }
            SketchAction::PreviewNumeric {
                key,
                capture,
                dimensions,
            } => {
                self.preview_numeric(key, capture, *dimensions);
                Vec::new()
            }
            SketchAction::CreateNumeric {
                key,
                capture,
                ids,
                dimensions,
            } => self.create_numeric(key, capture, ids, *dimensions),
            SketchAction::CreateNumericAuthored {
                key,
                capture,
                draft_generation,
                ids,
                dimensions,
                authored,
            } => self.create_numeric_authored(
                key,
                capture,
                ids,
                *dimensions,
                draft_generation,
                authored,
            ),
            SketchAction::Escape => self.escape(),
            SketchAction::Enter => self.enter_key(),
            SketchAction::Finish => self.exit(ExitIntent::Finish(FinishReason::Explicit)),
            SketchAction::Discard => self.exit(ExitIntent::Discard),
            SketchAction::AcceptView {
                key,
                document,
                capture,
            } => {
                self.accept_view(key, document, capture);
                Vec::new()
            }
            SketchAction::SelectionDrag {
                key,
                document,
                capture,
                active,
            } => {
                if self.selection_scope(&key, &document, capture) {
                    if let Some(session) = self.session.as_mut() {
                        session.selection_dragging = active;
                    }
                }
                Vec::new()
            }
            SketchAction::Select {
                key,
                document,
                capture,
                targets,
                mode,
            } => {
                self.select(key, document, capture, targets, mode);
                Vec::new()
            }
            SketchAction::AddConstraints { key, constraints } => {
                self.add_constraints(key, constraints)
            }
            SketchAction::SetDimension {
                key,
                constraint,
                value,
            } => self.set_dimension(key, constraint, value),
            SketchAction::Repair { key, ops } => self.repair(key, ops),
            SketchAction::History(direction) => self.history(direction),
            SketchAction::HistoryResolved {
                tag,
                snapshot,
                solve,
                moved,
                direction,
            } => self.history_resolved(tag, snapshot, *solve, moved, direction),
            SketchAction::ConstraintRefused { tag, message } => {
                self.constraint_refused(tag, message)
            }
            SketchAction::BatchAccepted { tag } => self.batch_accepted(tag),
            SketchAction::Solved {
                tag,
                snapshot,
                solve,
            } => self.solved(tag, snapshot, *solve),
            SketchAction::Failed { tag, message } => self.failed(tag, message),
            SketchAction::FinishRefused { tag, message, .. } => self.finish_refused(tag, message),
            SketchAction::ReenterForRepair { key, document } => {
                self.reenter_for_repair(key, document)
            }
            SketchAction::Finished { tag, result } => self.finished(tag, result),
            SketchAction::Discarded { tag, result } => self.discarded(tag, result),
        }
    }

    fn begin_captured(
        &mut self,
        sketch: Sketch,
        create: bool,
        context: DocumentContext,
    ) -> Vec<SketchEffect> {
        if self.context.as_ref() != Some(&context) {
            return Vec::new();
        }
        self.begin(sketch, create)
    }

    fn context_changed(&mut self, next: Option<DocumentContext>) -> Vec<SketchEffect> {
        let same = self
            .context
            .as_ref()
            .zip(next.as_ref())
            .is_some_and(|(a, b)| a.same_runtime(b));
        if same
            && self
                .context
                .as_ref()
                .zip(next.as_ref())
                .is_some_and(|(a, b)| b.revision < a.revision)
        {
            return Vec::new();
        }
        let refresh = same
            && self
                .session
                .as_ref()
                .zip(next.as_ref())
                .is_some_and(|(session, context)| {
                    matches!(
                        session.phase,
                        SketchPhase::Active | SketchPhase::NeedsRefresh
                    ) && (session.document.revision != context.revision
                        || session.document.snapshot_id != context.snapshot_id)
                });
        if !same {
            self.session = None;
        } else if let (Some(session), Some(context)) = (&mut self.session, &next) {
            session.document = context.clone();
            if refresh {
                session.phase = SketchPhase::NeedsRefresh;
                session.draft = None;
                session.preview = None;
                session.selection.clear();
                session.accepted_capture = None;
                session.selection_dragging = false;
                session.status =
                    SketchStatus::Refused("Document changed; refresh sketch before editing".into());
            }
        }
        self.context = next;
        if refresh {
            if let Some(tag) = self.next_request() {
                return vec![SketchEffect::RefreshSketch { tag }];
            }
        }
        Vec::new()
    }

    fn begin(&mut self, sketch: Sketch, create: bool) -> Vec<SketchEffect> {
        if !super::geometry::valid_plane(sketch.effective_plane()) {
            return Vec::new();
        }
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.phase != SketchPhase::Closed)
        {
            return Vec::new();
        }
        let Some(document) = self.context.clone().filter(|context| {
            !context.document_id.is_empty() && !context.runtime_session.is_empty()
        }) else {
            return Vec::new();
        };
        let Some(sequence) = self.sequence.checked_add(1) else {
            return Vec::new();
        };
        self.sequence = sequence;
        let key = SketchSessionKey {
            document_id: document.document_id.clone(),
            runtime_session: document.runtime_session.clone(),
            sketch_id: sketch.id,
            generation: sequence,
        };
        let tag = SketchTag {
            key: key.clone(),
            sequence,
            document: document.clone(),
        };
        let effect = if create {
            SketchEffect::CreateSketch {
                tag: tag.clone(),
                command: Box::new(EditCommand::AddSketch {
                    sketch: sketch.clone(),
                }),
            }
        } else {
            SketchEffect::EnterSketch {
                tag: tag.clone(),
                created_here: false,
            }
        };
        self.session = Some(SketchSession::starting(sketch, tag, create));
        vec![effect]
    }

    fn reenter_for_repair(
        &mut self,
        key: SketchSessionKey,
        document: DocumentContext,
    ) -> Vec<SketchEffect> {
        if self.context.as_ref() != Some(&document) {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut().filter(|session| {
            session.key == key
                && session.document == document
                && session.phase == SketchPhase::Faulted
                && session.repair_reentry
                && session.batch.is_none()
        }) else {
            return Vec::new();
        };
        if self.sequence.checked_add(1).is_none() || !super::geometry::valid_plane(session.plane) {
            return Vec::new();
        }
        let sketch = session.authoritative.clone();
        session.phase = SketchPhase::Closed;
        self.begin(sketch, false)
    }

    pub(crate) fn next_request(&mut self) -> Option<SketchTag> {
        let session = self.session.as_mut()?;
        self.sequence = self.sequence.checked_add(1)?;
        let tag = SketchTag {
            key: session.key.clone(),
            sequence: self.sequence,
            document: session.document.clone(),
        };
        session.request = tag.clone();
        Some(tag)
    }

    pub(crate) fn valid_tag(&self, tag: &SketchTag) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.request == *tag)
            && self
                .context
                .as_ref()
                .is_some_and(|context| context.same_runtime(&tag.document))
    }

    pub(crate) fn exit(&mut self, intent: ExitIntent) -> Vec<SketchEffect> {
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if matches!(
            session.phase,
            SketchPhase::Submitting | SketchPhase::Accepted
        ) {
            if !matches!(session.pending_exit, Some(ExitIntent::Discard)) {
                session.pending_exit = Some(intent);
            }
            session.draft = None;
            session.preview = None;
            return Vec::new();
        }
        if !matches!(
            session.phase,
            SketchPhase::Active | SketchPhase::NeedsRefresh | SketchPhase::Faulted
        ) {
            return Vec::new();
        }
        let Some(tag) = self.next_request() else {
            return Vec::new();
        };
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        session.draft = None;
        session.preview = None;
        session.selection.clear();
        match intent {
            ExitIntent::Finish(reason) => {
                session.phase = SketchPhase::Finishing;
                vec![SketchEffect::FinishSketch { tag, reason }]
            }
            ExitIntent::Discard => {
                session.phase = SketchPhase::Discarding;
                vec![SketchEffect::DiscardSketch { tag }]
            }
        }
    }

    fn history(&mut self, direction: onecad_core::edit::HistoryDirection) -> Vec<SketchEffect> {
        if !self
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
        {
            return Vec::new();
        }
        if self.clear_draft() {
            return Vec::new();
        }
        let Some(edit_session) = self
            .session
            .as_ref()
            .and_then(|session| session.edit_session.clone())
        else {
            return self.refuse("Sketch history: edit session is unavailable");
        };
        let Some(tag) = self.next_request() else {
            return Vec::new();
        };
        if let Some(session) = &mut self.session {
            session.phase = SketchPhase::Submitting;
            session.status = SketchStatus::Pending;
        }
        vec![SketchEffect::History {
            tag,
            edit_session,
            direction,
        }]
    }

    fn enter_key(&mut self) -> Vec<SketchEffect> {
        if self.clear_draft() {
            return Vec::new();
        }
        self.exit(ExitIntent::Finish(FinishReason::Explicit))
    }

    fn escape(&mut self) -> Vec<SketchEffect> {
        if self.clear_draft() {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if session.selection_dragging {
            session.selection_dragging = false;
            return Vec::new();
        }
        if session.tool != SketchTool::Select {
            session.tool = SketchTool::Select;
            return Vec::new();
        }
        if !session.selection.is_empty() {
            session.selection.clear();
            return Vec::new();
        }
        Vec::new()
    }

    pub(crate) fn clear_draft(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let had_draft = session.draft.take().is_some();
        session.preview = None;
        if had_draft {
            if let Some(batch) = &mut session.batch {
                batch.after = AfterPrimitive::Reset;
            }
        }
        had_draft
    }
}
