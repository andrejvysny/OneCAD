//! Projection edits that hold the sketch upsert lane across unlocked kernel queries.
use super::*;

/// The sketch upsert lane plus the fence a projection edit was planned against.
///
/// Claimed under the runtime lock before the unlocked `ProjectToSketchPlane`
/// round trips and held until the fenced commit, so no upsert, gesture, history
/// move or Finish on the same sketch can interleave. Every other mutation is
/// caught by the fence instead: the commit refuses unless the runtime, worker
/// epoch, document revision, head snapshot, edit visit and the WHOLE sketch
/// (provenance rows included) are exactly what the plan read.
pub struct ProjectionVisit {
    before: Sketch,
    instance: Uuid,
    fencing: (DocumentRevision, WorkerEpoch),
    head: Option<SnapshotId>,
    session: Uuid,
    _claim: Claim,
}

impl ProjectionVisit {
    /// The sketch exactly as the plan must read it.
    #[must_use]
    pub fn sketch(&self) -> &Sketch {
        &self.before
    }

    /// The head the plan's kernel queries must answer against.
    #[must_use]
    pub fn head(&self) -> SnapshotId {
        self.head.unwrap_or(SnapshotId(0))
    }
}

impl DocumentRuntime {
    /// Claims `sketch`'s upsert lane for a projection edit. `pick` is the
    /// snapshot the caller's picks were taken against; a superseded one is
    /// refused here, and the head fence carries that verdict to the commit.
    ///
    /// In-visit only: the commit is a visit step that Finish squashes. Outside
    /// `sketch`'s own visit it would land between another visit's steps and
    /// make that visit's Finish refuse as interleaved; the headless
    /// projection services own the no-visit path.
    ///
    /// # Errors
    /// A loud refusal on a read-only document, no open edit visit on `sketch`,
    /// an open drag, an in-flight upsert, an unadopted worker epoch, a stale
    /// pick or an unknown sketch.
    pub async fn claim_projection_visit(
        &self,
        sketch: SketchId,
        pick: Option<SnapshotId>,
        verb: &str,
    ) -> Result<ProjectionVisit, EngineError> {
        if self.read_only {
            return Err(op_failed(format!("{verb}: read-only document")));
        }
        let Some(session) = self.sketch_edit_token(sketch) else {
            return Err(op_failed(format!(
                "{verb}: sketch {sketch} has no open edit visit — enter the sketch first"
            )));
        };
        if self.has_active_gesture(sketch) {
            return Err(op_failed(format!(
                "{verb}: sketch {sketch} has an active drag gesture — retry after pointer-up"
            )));
        }
        let claim = self.claim_upsert_lane(sketch).await?;
        if let Some(pick) = pick {
            self.gate_stale_pick(pick)?;
        }
        let fencing = self.fencing.get();
        if fencing.1 != self.engine.current_epoch() {
            return Err(op_failed(format!(
                "{verb}: document has not adopted the worker epoch"
            )));
        }
        Ok(ProjectionVisit {
            before: self.sketch_or_err(sketch, verb)?,
            instance: self.instance,
            fencing,
            head: self.head_snapshot_id(),
            session,
            _claim: claim,
        })
    }

    /// Commits a planned projection edit as one undoable sketch edit, then
    /// resyncs the worker's solver lane with the result (`SketchUpsert`).
    ///
    /// The upsert runs on the candidate BEFORE the commit, exactly as
    /// [`sketch_upsert_with_outcome`](Self::sketch_upsert_with_outcome) does, so
    /// the worker store and the document cannot disagree after success.
    ///
    /// # Errors
    /// A loud refusal when anything the plan read moved, the edit layer's own
    /// refusal, or a worker failure. Nothing is committed on any error.
    pub async fn commit_projection_visit(
        &mut self,
        visit: ProjectionVisit,
        ops: Vec<SketchEditOp>,
        verb: &str,
    ) -> Result<(SketchUpsertDto, Option<CommandOutcome>), EngineError> {
        let id = visit.before.id;
        if self.read_only
            || self.instance != visit.instance
            || self.fencing.get() != visit.fencing
            || self.engine.current_epoch() != visit.fencing.1
            || self.head_snapshot_id() != visit.head
            || self.sketch_edit_token(id) != Some(visit.session)
            || self.sketch_or_err(id, verb)? != visit.before
        {
            return Err(op_failed(format!(
                "{verb}: the sketch, its edit visit, the document or the worker changed while \
                 projecting — retry"
            )));
        }
        let candidate = self
            .session
            .prepare_sketch_edit(id, &ops)
            .map_err(|e| op_failed(format!("{verb}: {e}")))?;
        let (solved, after) = self.solve_sketch_candidate(&candidate).await?;
        let outcome = self.commit_sketch_solve(candidate, after, ops)?;
        self.record_upsert(id, &solved);
        self.clear_projection_staleness(id);
        Ok((solved, outcome))
    }
}
