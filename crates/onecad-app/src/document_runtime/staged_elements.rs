//! Staged operand promotion (ONECAD-315 S-315c): mint and bind element ids for a
//! tool's picks without persisting, dirtying or emitting. A commit that references
//! a staged id adopts its evidence atomically; a cancelled stage leaves
//! `doc.elements` untouched because the leak guard strips staged ids from every
//! published regen mirror until they are adopted.
use super::*;
use crate::read::PreviewContext;

/// Scope one staging call was admitted under. Mirrors [`FaceEntryTicket`]: any
/// moved revision, head, epoch, runtime instance or engine makes it stale.
pub struct StageTicket {
    instance: Uuid,
    revision: DocumentRevision,
    snapshot: SnapshotId,
    epoch: WorkerEpoch,
    engine: Arc<dyn GeometryEngine>,
}

impl StageTicket {
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
}

impl DocumentRuntime {
    /// Admits a staging request whose UI scope is exactly the current head.
    pub fn stage_ticket(&self, scope: &PreviewContext) -> Result<StageTicket, EngineError> {
        let current = self.document_id() == scope.document_id
            && self.matches_runtime_session(&scope.runtime_session)
            && self.revision() == scope.revision
            && self.head_snapshot_id() == Some(scope.snapshot_id)
            && self.head_epoch == scope.worker_epoch
            && self.fencing.get().1 == scope.worker_epoch
            && self.engine.current_epoch() == scope.worker_epoch;
        if !current {
            return Err(stage_stale());
        }
        Ok(StageTicket {
            instance: self.instance,
            revision: scope.revision,
            snapshot: scope.snapshot_id,
            epoch: scope.worker_epoch,
            engine: self.engine.clone(),
        })
    }

    pub fn stage_ticket_is_current(&self, ticket: &StageTicket) -> bool {
        self.instance == ticket.instance
            && self.revision() == ticket.revision
            && self.head_snapshot_id() == Some(ticket.snapshot)
            && self.head_epoch == ticket.epoch
            && self.fencing.get().1 == ticket.epoch
            && self.engine.current_epoch() == ticket.epoch
            && Arc::ptr_eq(&self.engine, &ticket.engine)
    }

    /// Promotes TopoKey picks on one body for a tool's operands. Returns refs in
    /// pick order with intent and anchor filled, so preview and commit carry the
    /// same bytes. Does not touch `regen.elements`, `dirty` or the revision.
    pub async fn stage_selection(
        &mut self,
        ticket: &StageTicket,
        body: BodyId,
        picks: Vec<TopoKey>,
    ) -> Result<Vec<ElementRef>, EngineError> {
        if !self.stage_ticket_is_current(ticket) {
            return Err(stage_stale());
        }
        let requested = picks.clone();
        let minted = self
            .promote_inner(
                ticket.snapshot,
                body,
                picks.into_iter().map(|key| (key, None)).collect(),
            )
            .await?;
        // Guard every bound id at once, before any later refusal can drop it.
        for (id, evidence) in &minted {
            if !self.regen.elements.contains(id) {
                self.staged.insert(
                    id.clone(),
                    ElementEntry::new(evidence.body, evidence.kind)
                        .with_evidence(evidence.anchor.clone(), evidence.descriptor.clone()),
                );
            }
        }
        requested
            .iter()
            .map(|key| {
                minted
                    .iter()
                    .find(|(_, evidence)| evidence.topo_key == *key)
                    .map(|(id, evidence)| self.staged_ref(id, evidence))
                    .ok_or_else(|| stage_refused("staged promotion lost a pick"))
            })
            .collect()
    }

    /// Stages a pick the mesh already labels with a promoted ElementId: the id is
    /// looked up to its current TopoKey and promoted again, which must reuse it.
    pub async fn stage_existing(
        &mut self,
        ticket: &StageTicket,
        body: BodyId,
        id: &ElementId,
        kind: ElementKind,
    ) -> Result<ElementRef, EngineError> {
        if !self.stage_ticket_is_current(ticket) {
            return Err(stage_stale());
        }
        let Some(query) = self.read_query.clone() else {
            return Err(stage_refused("element lookup is unavailable"));
        };
        let info = query
            .query_element(ticket.snapshot, body, id.as_str())
            .await?
            .ok_or_else(|| stage_refused("the picked element no longer exists"))?;
        if info.kind != kind_str(kind) || info.body_id != crate::worker::wire::body_id_wire(body) {
            return Err(stage_refused("the picked element changed kind or body"));
        }
        let mut refs = self
            .stage_selection(ticket, body, vec![TopoKey::new(info.topo_key)])
            .await?;
        let reference = refs.pop().ok_or_else(|| stage_refused("no element"))?;
        if reference.primary.as_ref().map(|p| &p.element) != Some(id) {
            return Err(stage_refused("the picked element resolved to another id"));
        }
        Ok(reference)
    }

    fn staged_ref(
        &self,
        id: &ElementId,
        evidence: &onecad_core::regen::WorkerElementEvidence,
    ) -> ElementRef {
        let mut reference = ElementRef {
            primary: Some(PrimaryRef {
                body: evidence.body,
                element: id.clone(),
                kind: evidence.kind,
                extra: Default::default(),
            }),
            intent: None,
            anchor: evidence.anchor.clone(),
            extra: Default::default(),
        };
        self.stamp_intent(&mut reference);
        reference
    }

    /// Staged ids referenced by `commands`: operation refs, a host-face sketch
    /// attachment, a rebound element input and a face colour all persist an id,
    /// so each must carry its evidence into `doc.elements`. Ids in neither map are
    /// left alone: a regen legitimately drops consumed elements (a filleted edge)
    /// from the partition, and re-editing such a feature must still commit.
    pub(crate) fn staged_ids_in(&self, commands: &[EditCommand]) -> Vec<ElementId> {
        let mut referenced = Vec::new();
        for command in commands {
            command_element_ids(command, &mut referenced);
        }
        referenced.retain(|id| self.staged.contains_key(id));
        referenced
    }

    pub(crate) fn adopt_staged(&mut self, ids: Vec<ElementId>) {
        for id in ids {
            if let Some(entry) = self.staged.remove(&id) {
                self.regen.elements.insert(id, entry);
            }
        }
    }

    /// Strips staged-but-unadopted ids a regen delta folded into `scratch`. The
    /// predicate reads the live mirror, so ids any persisting writer recorded
    /// (promotion, face entry, adoption) survive.
    pub(crate) fn guard_staged_leaks(&self, scratch: &mut RegenSession) {
        for id in self.staged.keys() {
            if !self.regen.elements.contains(id) {
                scratch.elements.remove(id);
            }
        }
    }

    pub fn staged_element_count(&self) -> usize {
        self.staged.len()
    }
}

/// Every element id `command` would persist.
fn command_element_ids(command: &EditCommand, out: &mut Vec<ElementId>) {
    let mut push_ref = |reference: &ElementRef| {
        if let Some(primary) = reference.primary.as_ref() {
            out.push(primary.element.clone());
        }
    };
    match command {
        EditCommand::AddOperation {
            record:
                OperationRecord {
                    op: Operation::Known(known),
                    ..
                },
            ..
        }
        | EditCommand::UpdateOperationParams {
            op: Operation::Known(known),
            ..
        } => {
            let mut known = known.clone();
            for reference in known.element_refs_mut() {
                push_ref(reference);
            }
        }
        EditCommand::AddSketch { sketch } => {
            if let SketchAttachment::HostFace { face, .. } = &sketch.attachment {
                push_ref(face);
            }
        }
        EditCommand::UpdateSketchAttachment {
            attachment: SketchAttachment::HostFace { face, .. },
            ..
        } => push_ref(face),
        EditCommand::EditOperationInput {
            reference: InputRef::Element(reference),
            ..
        } => push_ref(reference),
        EditCommand::SetFaceColor { element_id, .. } => out.push(element_id.clone()),
        _ => {}
    }
}

fn stage_stale() -> EngineError {
    EngineError::OpFailed {
        code: onecad_core::regen::OpFailureCode::StalePreview,
        recoverable: true,
        message: "operand selection belongs to a stale document head; re-pick".into(),
        diagnostics: Vec::new(),
    }
}

fn stage_refused(reason: &str) -> EngineError {
    EngineError::OpFailed {
        code: onecad_core::regen::OpFailureCode::RefUnresolved,
        recoverable: true,
        message: format!("{reason}; re-pick"),
        diagnostics: Vec::new(),
    }
}
