//! [`DocumentSession`]: the single writer to the authoritative document.
//!
//! All mutations flow through one session (plan "DocumentSession single writer")
//! so ordering is deterministic and undo/redo stays consistent. Each
//! [`EditCommand`] is validated, applied, and paired with a memento inverse
//! ([`crate::edit::undo`]); a [`DependencyGraph`] is rebuilt on every structural
//! change and used for anti-time-travel (`produces_before`) validation.
//!
//! ## Dirty / regen table (per command)
//!
//! | command | dirty span | [`RegenHint`] |
//! |---|---|---|
//! | `AddOperation` (at cursor) | `[insertIndex, len)` | `ToEnd` |
//! | `AddOperation` (frontier append, `cursor == len`) | `[index, len)` | `ToEnd` (joins the applied prefix) |
//! | `AddOperation` (rolled-back append, `cursor < len`) | `[index, len)` | `None` (stays a draft) |
//! | `UpdateOperationParams` | `[step, len)` | `ToEnd` |
//! | `EditOperationInput` | `[step, len)` | `ToEnd` |
//! | `RemoveOperation` | `[removedIndex, len)` | `ToEnd` |
//! | `SetRollback` | span from `set_cursor` | `PreviewTo(cursor)` |
//! | `SetOperationSuppression` | `[step, len)` | `ToEnd` |
//! | `AddSketch` | first dependent op step (`None` if none) | `ToEnd` / `None` |
//! | `DeleteSketch` / `UpdateSketchAttachment` / `SketchEdit` / `SketchDragGesture` | `min(producing Sketch op, first dependent op)` | `ToEnd` / `None` |
//! | `RenameSketch` | — | `None` |
//! | `AddBody` / `DeleteBody` / `RenameBody` / `SetVisibility` | — | `None` |
//! | `AddDatumPlane` / `DeleteDatum` | — | `None` |
//! | `SetVariable` / `AddVariable` / `RemoveVariable` | `[0, len)` (conservative) | `ToEnd` / `None` |
//!
//! `UpdateOperationParams` keeps it simple (`ToEnd`) rather than a `PreviewTo`
//! interactive path (deferred; plan "keep simple"). Variable edits are
//! conservative: a bare-variable-name expression can drive any dimensioned op,
//! so any variable change dirties the whole applied timeline (a precise
//! variable→op dependency awaits the expression engine).
//!
//! A sketch edit dirties from `min(producing Sketch op, first dependent op)`: a
//! `Sketch` op is itself a timeline node whose regen re-runs region detection
//! (worker-side), so an edit must dirty from that producer, not only from the
//! first op consuming a region. A document sketch with no producing op falls back
//! to its first consumer.
//!
//! ## Reference existence is a regen-time concern (F7; C++ parity)
//!
//! The session validates *structural* invariants — duplicate ids, `opType`
//! immutability, fillet/chamfer lockstep, and anti-time-travel
//! (`produces_before`) — but does NOT check that a referenced sketch or body
//! actually *exists* when a command binds it (e.g. an `EditOperationInput`
//! profile pointing at an absent sketch, or an `AddOperation` whose params name a
//! body no op produces). Reference resolution/existence is deferred to regen by
//! design, matching C++: the command layer only rewrites the record, and the
//! worker's `RegenerationEngine` surfaces an unresolved reference as a
//! failure/`NeedsRepair` rather than the command rejecting it up front.
//!
//! **The one exception is a `Datum` sketch attachment** (`stamp_datum_plane`): a
//! sketch's coordinate frame is FROZEN at attach time and every entity coordinate
//! is expressed in it, so the datum has to exist and be resolved *now*. Deferring
//! it would leave the sketch on the XY placeholder — a silent wrong frame, not a
//! deferred resolution.

use std::collections::{BTreeMap, BTreeSet};

use crate::document::body::{BodyMeta, BodyRegistry};
use crate::document::datum::{resolve_datum, DatumContext, DatumKind, DatumPlane};
use crate::document::record::{KnownOperation, Operation, OperationRecord};
use crate::document::refs::ElementRef;
use crate::document::variables::{Scalar, Variable, VariableTable};
use crate::document::Document;
use crate::error::DomainError;
use crate::history::{DependencyGraph, DirtyRange, Timeline};
use crate::ids::{BodyId, RecordId, SketchId};
use crate::math::Vec2;
use crate::sketch::{Constraint, Sketch, SketchAttachment, SketchEntity, SketchError};

use super::command::{EditCommand, InputPath, InputRef, SketchEditOp, VisibilityTarget};
use super::outcome::{CommandOutcome, ProjectionDelta, RegenHint};
use super::undo::{insert_record_at, replace_record_at, AppliedEdit, Inverse, Txn, UndoStack};

/// Owns the document and applies [`EditCommand`]s as the sole writer.
#[derive(Debug)]
pub struct DocumentSession {
    document: Document,
    undo: UndoStack,
    graph: DependencyGraph,
    open_txn: Option<Box<TxnState>>,
}

/// A transaction being accumulated between `begin_transaction`/`end_transaction`.
#[derive(Debug)]
struct TxnState {
    label: String,
    edits: Vec<AppliedEdit>,
    combined: CommandOutcome,
}

impl DocumentSession {
    /// A session over `document`, with a freshly built dependency graph.
    #[must_use]
    pub fn new(document: Document) -> Self {
        let mut graph = DependencyGraph::new();
        graph.rebuild_from_records(document.timeline.records());
        Self {
            document,
            undo: UndoStack::new(),
            graph,
            open_txn: None,
        }
    }

    /// The authoritative document (read-only; mutate only via [`apply`]).
    ///
    /// [`apply`]: DocumentSession::apply
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The derived dependency graph.
    #[must_use]
    pub fn graph(&self) -> &DependencyGraph {
        &self.graph
    }

    /// Consumes the session, returning the document.
    #[must_use]
    pub fn into_document(self) -> Document {
        self.document
    }

    /// Whether an undo step is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    /// Whether a redo step is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Current undo depth.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo.undo_depth()
    }

    /// The monotonic count of undo steps evicted past the cap since open. The
    /// sketch-session squash records this at enter and refuses the squash if it
    /// has moved (an eviction shifted the stack bottom, invalidating the
    /// depth-based watermark).
    #[must_use]
    pub fn evictions(&self) -> u64 {
        self.undo.evictions()
    }

    /// Applies a command: validates, mutates the document, captures a memento
    /// inverse, and records it on the open transaction (or as a singleton undo
    /// step). Returns the per-command [`CommandOutcome`].
    ///
    /// # Errors
    /// A [`DomainError`] if the command fails validation; the document is left
    /// unchanged (apply is atomic). When a command fails **inside an open
    /// transaction**, the whole transaction is auto-cancelled first (its
    /// already-applied edits are rolled back in reverse and it is closed — C++
    /// `CommandProcessor::execute` `CommandProcessor.cpp:62-67`) before the
    /// error propagates.
    pub fn apply(&mut self, cmd: EditCommand) -> Result<CommandOutcome, DomainError> {
        let (outcome, inverse) = match self.apply_forward(&cmd) {
            Ok(v) => v,
            Err(e) => {
                // A failure mid-transaction cancels the whole batch (C++ parity).
                if self.open_txn.is_some() {
                    self.cancel_transaction();
                }
                return Err(e);
            }
        };
        let edit = AppliedEdit {
            forward: cmd,
            inverse,
        };
        match &mut self.open_txn {
            Some(txn) => {
                txn.edits.push(edit);
                merge_outcome(&mut txn.combined, &outcome);
            }
            None => {
                let label = edit.forward.label().to_string();
                self.undo.push_committed(Txn::single(label, edit));
            }
        }
        Ok(outcome)
    }

    /// Opens a transaction: subsequent [`apply`]s batch into one undo step until
    /// [`end_transaction`]. A no-op if one is already open.
    ///
    /// [`apply`]: DocumentSession::apply
    /// [`end_transaction`]: DocumentSession::end_transaction
    pub fn begin_transaction(&mut self, label: impl Into<String>) {
        if self.open_txn.is_none() {
            self.open_txn = Some(Box::new(TxnState {
                label: label.into(),
                edits: Vec::new(),
                combined: empty_outcome(),
            }));
        }
    }

    /// Commits the open transaction as one undo step and returns the combined
    /// outcome (merged deltas, unioned dirty span, strongest regen). Returns
    /// `None` if no transaction is open or it is empty.
    pub fn end_transaction(&mut self) -> Option<CommandOutcome> {
        let txn = self.open_txn.take()?;
        if txn.edits.is_empty() {
            return None;
        }
        let combined = txn.combined.clone();
        self.undo.push_committed(Txn {
            label: txn.label,
            edits: txn.edits,
        });
        Some(combined)
    }

    /// Rolls back and discards the open transaction (C++ `cancelTransaction`).
    pub fn cancel_transaction(&mut self) {
        if let Some(txn) = self.open_txn.take() {
            for edit in txn.edits.into_iter().rev() {
                edit.inverse.apply(&mut self.document);
            }
            self.rebuild_graph();
        }
    }

    /// Collapses the newest `count` committed undo steps of an in-progress sketch
    /// session into ONE net [`EditCommand::SketchEdit`] (Codex-review B1 squash).
    ///
    /// The document is **not** mutated — it already holds the final sketch; only
    /// the undo stack is rewritten so the whole session undoes/redoes as one step:
    ///
    /// * undo restores `prior` **verbatim** (via [`Inverse::RestoreSketch`]) —
    ///   exact for entities, constraints and positions regardless of the forward
    ///   ops (the net command's inverse never depends on a diff);
    /// * redo replays the net `SketchEdit`, whose ops full-replace the sketch body
    ///   (remove every prior entity — cascading its constraints — then re-add the
    ///   final entities and constraints in order), reconstructing the final sketch
    ///   exactly. A full replace via existing variants is chosen over a minimal
    ///   diff because it guarantees an order-exact redo without fragile per-field
    ///   diffing (the brief's blessed "remove-all + re-add" alternative).
    ///
    /// A net-zero session (final sketch == `prior`, or the sketch no longer
    /// exists) pops the granular steps and pushes nothing. `count == 0` is a no-op
    /// (a crash between enter and finish/cancel leaves the granular steps).
    pub fn squash_sketch_session(&mut self, sketch: SketchId, prior: Sketch, count: usize) {
        if count == 0 {
            return;
        }
        // B1 guard — **all-or-nothing, never a clamp**. Squash ONLY when the entire
        // watermark→head range (`count` newest steps) is a contiguous run of txns
        // whose *every* edit is a `SketchEdit`/`SketchDragGesture` for THIS sketch.
        // Any interleaved model op (or edit of another sketch) stops the trailing run
        // short (`< count`), so the granular steps are kept untouched: a clamped
        // squash would pair a shortened pop-count with the FULL prior-restore inverse
        // and corrupt undo (`Sketch A → model op → Sketch B` would restore pre-A
        // state while leaving the model op). Mixed txns are atomic — one non-matching
        // edit disqualifies the whole txn.
        let is_this_sketch_edit = |cmd: &EditCommand| {
            matches!(
                cmd,
                EditCommand::SketchEdit { sketch: s, .. }
                | EditCommand::SketchDragGesture { sketch: s, .. }
                    if *s == sketch
            )
        };
        let contiguous = self.undo.trailing_run(count, |txn| {
            txn.edits.iter().all(|e| is_this_sketch_edit(&e.forward))
        });
        if contiguous != count {
            return; // interleaved non-sketch work — keep the granular txns.
        }
        let Some(final_sketch) = self.document.sketches.get(&sketch) else {
            // Sketch deleted mid-session (unexpected): leave the granular steps.
            return;
        };
        let net_zero = final_sketch.entities() == prior.entities()
            && final_sketch.constraints() == prior.constraints();
        if net_zero {
            self.undo.squash(count, None);
            return;
        }
        let mut ops: Vec<SketchEditOp> = Vec::new();
        for e in prior.entities() {
            ops.push(SketchEditOp::RemoveEntity { entity: e.id() });
        }
        for e in final_sketch.entities() {
            ops.push(SketchEditOp::AddEntity { entity: e.clone() });
        }
        for c in final_sketch.constraints() {
            ops.push(SketchEditOp::AddConstraint {
                constraint: c.clone(),
            });
        }
        let edit = AppliedEdit {
            forward: EditCommand::SketchEdit { sketch, ops },
            inverse: Inverse::RestoreSketch {
                id: sketch,
                prior: Some(Box::new(prior)),
            },
        };
        let label = edit.forward.label().to_string();
        self.undo.squash(count, Some(Txn::single(label, edit)));
    }

    /// Syncs every record's **derived** `outputs` — the bodies that op
    /// produced/modified in the last accepted regen — and rebuilds the dependency
    /// graph. Returns whether anything changed.
    ///
    /// Derived data, not an edit: no undo entry, no command outcome, step states and
    /// the cursor untouched — the same treatment `inputs` gets on load (re-derived,
    /// never trusted from disk). `outputs` is the ONLY source of the graph's
    /// body-producer index, so until a regen calls this the graph has **no body
    /// edges at all** and both the suppression cascade
    /// ([`EditCommand::SetOperationSuppression`] with `cascade`) and the
    /// anti-time-travel check are inert for body dependencies.
    ///
    /// `outputs` is the whole truth **only for the records this regen actually ran**.
    /// `executed` names them (the committed plan's records); a record in that set but
    /// absent from `outputs` produced nothing and has its `outputs` cleared, so a stale
    /// producer edge cannot survive the regen that dropped it.
    ///
    /// A record **outside** `executed` is left **untouched** — never cleared. A
    /// checkpoint-accelerated regen executes only the post-checkpoint suffix, and the
    /// restored base registry carries **no lifecycle log** for the prefix (the worker
    /// reconstructs it from the immutable artifacts), so the caller's `outputs` map
    /// legitimately knows nothing about the pre-checkpoint records. Clearing them there
    /// would erase the graph's body-producer edges for the whole prefix and silently kill
    /// the suppression cascade — and the wipe is durable, because `outputs` is serialized
    /// with the record. Records absent from `executed` but PRESENT in `outputs` are still
    /// refreshed: the executor's replay-from-0 fallback (Invariant 7) re-runs more steps
    /// than the plan named, and that fresh provenance is strictly better than the old.
    ///
    /// **Suppressed records are skipped** — they were deliberately not executed, so they
    /// have no fresh provenance, and keeping their last-known `outputs` is what lets an
    /// *un*-suppress cascade find the ops standing on their bodies (clearing them would
    /// make suppression a one-way door).
    pub fn sync_record_outputs(
        &mut self,
        outputs: &BTreeMap<RecordId, Vec<BodyId>>,
        executed: &BTreeSet<RecordId>,
    ) -> bool {
        let mut changed = false;
        for i in 0..self.document.timeline.len() {
            let Some(rec) = self.document.timeline.record(i) else {
                continue;
            };
            if rec.suppressed {
                continue;
            }
            let id = rec.record_id;
            if !executed.contains(&id) && !outputs.contains_key(&id) {
                continue; // outside this regen's executed range — keep what it had.
            }
            let next = outputs.get(&id).cloned().unwrap_or_default();
            changed |= self.document.timeline.set_record_outputs(i, next);
        }
        if changed {
            self.rebuild_graph();
        }
        changed
    }

    /// Registers a document metadata row for every body in `regen` the document does
    /// not know yet (the regen row copied verbatim). Returns whether anything was
    /// adopted.
    ///
    /// Registers **identity, not user intent**: no undo entry, no command outcome, no
    /// dirty flip — the counterpart of
    /// [`sync_record_outputs`](Self::sync_record_outputs). A worker-minted body
    /// (an extrude output) is otherwise absent from `document.bodies`, so
    /// `DeleteBody` / `RenameBody` / `SetVisibility`, which all validate against it,
    /// reject every body the timeline actually produced.
    ///
    /// An existing row is **never** overwritten: its `name`/`visible` are user intent
    /// and outrank the regen row's generated defaults (a from-0 replay starts from a
    /// fresh registry and re-derives `"Body N"`).
    ///
    /// Callers MUST re-run this after undo/redo as well as after a regen commit:
    /// [`Inverse::RestoreBodies`] restores the WHOLE registry, so an undo erases every
    /// row adopted since the edit it reverts.
    pub fn adopt_regen_bodies(&mut self, regen: &BodyRegistry) -> bool {
        let mut adopted = false;
        for meta in regen.bodies() {
            if !self.document.bodies.contains(meta.id) {
                adopted |= self.document.bodies.register(meta.clone());
            }
        }
        adopted
    }

    /// Undoes the newest committed transaction (applies its inverses in reverse
    /// and moves it to the redo stack). Ignored while a transaction is open
    /// (C++ `CommandProcessor::undo`). Returns `true` if a step was undone.
    pub fn undo(&mut self) -> bool {
        if self.open_txn.is_some() {
            return false;
        }
        let Some(txn) = self.undo.pop_for_undo() else {
            return false;
        };
        for edit in txn.edits.iter().rev() {
            edit.inverse.clone().apply(&mut self.document);
        }
        self.rebuild_graph();
        self.undo.push_undone(txn);
        true
    }

    /// Redoes the newest undone transaction by **re-executing** its forward
    /// commands (recomputing fresh inverses). Ignored while a transaction is
    /// open. Returns `true` if a step was redone.
    ///
    /// # Errors
    /// A [`DomainError`] if a replayed command fails (the partial replay is
    /// rolled back and the step returned to the redo stack).
    pub fn redo(&mut self) -> Result<bool, DomainError> {
        if self.open_txn.is_some() {
            return Ok(false);
        }
        let Some(txn) = self.undo.pop_for_redo() else {
            return Ok(false);
        };
        let mut new_edits: Vec<AppliedEdit> = Vec::with_capacity(txn.edits.len());
        for edit in &txn.edits {
            match self.apply_forward(&edit.forward) {
                Ok((_, inverse)) => new_edits.push(AppliedEdit {
                    forward: edit.forward.clone(),
                    inverse,
                }),
                Err(e) => {
                    for done in new_edits.into_iter().rev() {
                        done.inverse.apply(&mut self.document);
                    }
                    self.rebuild_graph();
                    self.undo.push_undone(txn);
                    return Err(e);
                }
            }
        }
        self.undo.push_redone(Txn {
            label: txn.label,
            edits: new_edits,
        });
        Ok(true)
    }

    // ── Forward dispatch ─────────────────────────────────────────────────────

    /// Applies one command's forward effect, returning its outcome + inverse.
    /// Pure with respect to the undo stacks (used by `apply`, `redo`, cancel).
    fn apply_forward(
        &mut self,
        cmd: &EditCommand,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        match cmd {
            EditCommand::AddOperation { record, at_cursor } => {
                self.add_operation(record.clone(), *at_cursor)
            }
            EditCommand::UpdateOperationParams { record, op } => {
                self.update_operation_params(*record, op.clone())
            }
            EditCommand::EditOperationInput {
                record,
                path,
                reference,
            } => self.edit_operation_input(*record, path, reference),
            EditCommand::RemoveOperation { record } => self.remove_operation(*record),
            EditCommand::SetRollback { cursor } => self.set_rollback(*cursor),
            EditCommand::SetOperationSuppression {
                record,
                suppressed,
                cascade,
            } => self.set_suppression(*record, *suppressed, *cascade),
            EditCommand::AddSketch { sketch } => self.add_sketch(sketch.clone()),
            EditCommand::DeleteSketch { sketch } => self.delete_sketch(*sketch),
            EditCommand::RenameSketch { sketch, name } => self.rename_sketch(*sketch, name.clone()),
            EditCommand::UpdateSketchAttachment {
                sketch,
                plane,
                attachment,
            } => self.update_sketch_attachment(*sketch, *plane, attachment.clone()),
            EditCommand::SketchEdit { sketch, ops } => self.sketch_edit(*sketch, ops),
            EditCommand::SketchDragGesture { sketch, after, .. } => {
                self.sketch_drag(*sketch, after.clone())
            }
            EditCommand::AddBody { body } => self.add_body(body.clone()),
            EditCommand::DeleteBody { body } => self.delete_body(*body),
            EditCommand::RenameBody { body, name } => self.rename_body(*body, name.clone()),
            EditCommand::SetVisibility { target, visible } => {
                self.set_visibility(*target, *visible)
            }
            EditCommand::AddDatumPlane { datum } => self.add_datum(datum.clone()),
            EditCommand::DeleteDatum { datum } => self.delete_datum(*datum),
            EditCommand::SetVariable { variable, value } => {
                self.set_variable(*variable, value.clone())
            }
            EditCommand::AddVariable { variable } => self.add_variable(variable.clone()),
            EditCommand::RemoveVariable { variable } => self.remove_variable(*variable),
        }
    }

    // ── Timeline commands ────────────────────────────────────────────────────

    fn add_operation(
        &mut self,
        mut record: OperationRecord,
        at_cursor: bool,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let id = record.record_id;
        if self.document.timeline.record_by_id(id).is_some() {
            return Err(DomainError::Validation(format!("duplicate record id {id}")));
        }
        // F2: fillet/chamfer `edges`/`edge_ids` must be in lockstep (all entry paths).
        validate_fillet_lockstep(&record.op)?;
        // F8: re-derive the uniform input view for Known ops (self-healing — don't
        // trust caller-supplied `inputs`; mirrors `update_operation_params` and the
        // record deserialize path). An Opaque frozen node keeps its stored inputs.
        if matches!(record.op, Operation::Known(_)) {
            record.inputs = record.op.derive_inputs();
        }
        let len = self.document.timeline.len();
        let insert_index = if at_cursor {
            self.document.timeline.cursor().min(len)
        } else {
            len
        };
        // Anti-time-travel: validate on the would-be record list before mutating.
        let mut recs = self.document.timeline.records().to_vec();
        recs.insert(insert_index, record.clone());
        self.validate_temporal(&recs, id, &record.op.derive_inputs().bodies)?;

        // A **frontier append** (`!at_cursor` and the cursor already sits at
        // `len == insert_index`) joins the applied prefix — the C++ `addOperation`
        // parity `setAppliedOpCount(insertIndex + 1)` — so a fresh commit at the
        // timeline end regens (the `index < cursor` check below yields `ToEnd`). A
        // **rolled-back append** (`cursor < len`) restores the old cursor and stays
        // a draft (`RegenHint::None`), the documented C++ draft semantic.
        let cursor_before = self.document.timeline.cursor();
        let frontier_append = !at_cursor && cursor_before == insert_index;
        let index = if at_cursor {
            self.document.timeline.insert_at_cursor(record)
        } else {
            let restore = if frontier_append {
                cursor_before + 1
            } else {
                cursor_before
            };
            insert_record_at(&mut self.document.timeline, insert_index, record, restore);
            insert_index
        };
        self.rebuild_graph();

        let new_len = self.document.timeline.len();
        let dirty = DirtyRange::new(index, new_len);
        let regen = if index < self.document.timeline.cursor() {
            RegenHint::ToEnd
        } else {
            RegenHint::None
        };
        let mut delta = ProjectionDelta::timeline();
        delta.cursor_changed = at_cursor || frontier_append;
        Ok((
            CommandOutcome {
                projection_delta: delta,
                dirty: Some(dirty),
                regen,
            },
            Inverse::RemoveRecord { record: id },
        ))
    }

    fn update_operation_params(
        &mut self,
        id: RecordId,
        op: Operation,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        let prior = self.document.timeline.record(index).unwrap().clone();
        if !same_op_type(&prior.op, &op) {
            return Err(DomainError::Validation(
                "UpdateOperationParams may not change opType".into(),
            ));
        }
        // F2: fillet/chamfer `edges`/`edge_ids` must be in lockstep (all entry paths).
        validate_fillet_lockstep(&op)?;
        let mut nr = prior.clone();
        nr.op = op;
        nr.inputs = nr.op.derive_inputs();

        let mut recs = self.document.timeline.records().to_vec();
        recs[index] = nr.clone();
        self.validate_temporal(&recs, id, &nr.op.derive_inputs().bodies)?;

        replace_record_at(&mut self.document.timeline, index, nr);
        self.rebuild_graph();
        Ok(self.dirty_record_outcome(
            index,
            Inverse::RestoreRecord {
                index,
                record: Box::new(prior),
            },
        ))
    }

    fn edit_operation_input(
        &mut self,
        id: RecordId,
        path: &InputPath,
        reference: &InputRef,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        let prior = self.document.timeline.record(index).unwrap().clone();
        let mut nr = prior.clone();
        set_input(&mut nr.op, path, reference)?;
        nr.inputs = nr.op.derive_inputs();

        let mut recs = self.document.timeline.records().to_vec();
        recs[index] = nr.clone();
        self.validate_temporal(&recs, id, &nr.op.derive_inputs().bodies)?;

        replace_record_at(&mut self.document.timeline, index, nr);
        self.rebuild_graph();
        Ok(self.dirty_record_outcome(
            index,
            Inverse::RestoreRecord {
                index,
                record: Box::new(prior),
            },
        ))
    }

    fn remove_operation(&mut self, id: RecordId) -> Result<(CommandOutcome, Inverse), DomainError> {
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        let record = self.document.timeline.record(index).unwrap().clone();
        let cursor = self.document.timeline.cursor();
        let dirty = self.document.timeline.remove(id)?;
        self.rebuild_graph();
        let mut delta = ProjectionDelta::timeline();
        delta.cursor_changed = index < cursor;
        Ok((
            CommandOutcome {
                projection_delta: delta,
                dirty: Some(dirty),
                regen: RegenHint::ToEnd,
            },
            Inverse::InsertRecord {
                index,
                record: Box::new(record),
                cursor,
            },
        ))
    }

    fn set_rollback(&mut self, cursor: usize) -> Result<(CommandOutcome, Inverse), DomainError> {
        let len = self.document.timeline.len();
        if cursor > len {
            return Err(DomainError::Timeline(format!(
                "rollback cursor {cursor} exceeds op count {len}"
            )));
        }
        let prior = self.document.timeline.cursor();
        let dirty = self.document.timeline.set_cursor(cursor);
        let delta = ProjectionDelta {
            cursor_changed: true,
            ..ProjectionDelta::default()
        };
        Ok((
            CommandOutcome {
                projection_delta: delta,
                dirty: Some(dirty),
                regen: RegenHint::PreviewTo(cursor),
            },
            Inverse::RestoreCursor { cursor: prior },
        ))
    }

    fn set_suppression(
        &mut self,
        id: RecordId,
        suppressed: bool,
        cascade: bool,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        // Affected = the op + (if cascading) its downstream closure.
        let mut affected: Vec<RecordId> = vec![id];
        if cascade {
            affected.extend(self.graph.downstream(id));
        }
        // Capture priors (for the composite inverse) and set the flag in one rebuild.
        let mut recs = self.document.timeline.records().to_vec();
        let mut inverses: Vec<Inverse> = Vec::new();
        for &rid in &affected {
            if let Some(i) = recs.iter().position(|r| r.record_id == rid) {
                inverses.push(Inverse::RestoreRecord {
                    index: i,
                    record: Box::new(recs[i].clone()),
                });
                recs[i].suppressed = suppressed;
            }
        }
        let cursor = self.document.timeline.cursor();
        self.document.timeline = Timeline::from_records(recs);
        self.document
            .timeline
            .set_cursor(cursor.min(self.document.timeline.len()));
        self.rebuild_graph();

        let dirty = DirtyRange::new(index, self.document.timeline.len());
        Ok((
            CommandOutcome {
                projection_delta: ProjectionDelta::timeline(),
                dirty: Some(dirty),
                regen: RegenHint::ToEnd,
            },
            Inverse::Composite(inverses),
        ))
    }

    // ── Sketch commands ──────────────────────────────────────────────────────

    /// For a `Datum`-attached sketch, STAMP its frame from the datum's resolved
    /// plane and reject a missing / unresolved datum loudly.
    ///
    /// This is a DELIBERATE exception to the "reference existence is a regen-time
    /// concern" rule documented at the top of this module. The frame is frozen
    /// into the sketch at attach time and every entity coordinate is expressed in
    /// it, so the reference has to exist *now* — deferring it to regen would
    /// silently leave the sketch on the XY default (`Sketch::new`'s placeholder),
    /// i.e. exactly the silent-wrong-bind failure the migration exists to remove.
    /// It is also why the caller's `sketch.plane` is overwritten rather than
    /// trusted: the frontend round-trips it, and the core is the basis authority.
    fn stamp_datum_plane(&self, sketch: &mut Sketch) -> Result<(), DomainError> {
        let SketchAttachment::Datum { datum } = sketch.attachment else {
            return Ok(());
        };
        let d = self.document.datum_planes.get(&datum).ok_or_else(|| {
            DomainError::Validation(format!(
                "sketch {} attaches to datum {datum}, which does not exist",
                sketch.id
            ))
        })?;
        if !d.resolved_valid {
            return Err(DomainError::Validation(format!(
                "sketch {} attaches to datum {datum} ('{}'), whose frame is unresolved",
                sketch.id, d.name
            )));
        }
        sketch.plane = d.resolved_plane;
        Ok(())
    }

    fn add_sketch(&mut self, mut sketch: Sketch) -> Result<(CommandOutcome, Inverse), DomainError> {
        let id = sketch.id;
        if self.document.sketches.contains_key(&id) {
            return Err(DomainError::Validation(format!("duplicate sketch id {id}")));
        }
        self.stamp_datum_plane(&mut sketch)?;
        self.document.sketches.insert(id, sketch);
        Ok((
            self.sketch_dirty_outcome(id),
            Inverse::RestoreSketch { id, prior: None },
        ))
    }

    fn delete_sketch(&mut self, id: SketchId) -> Result<(CommandOutcome, Inverse), DomainError> {
        let prior = self
            .document
            .sketches
            .remove(&id)
            .ok_or_else(|| DomainError::Validation(format!("sketch {id} not found")))?;
        let prior_vis = self.document.sketch_visibility.remove(&id);
        Ok((
            self.sketch_dirty_outcome(id),
            Inverse::Composite(vec![
                Inverse::RestoreSketch {
                    id,
                    prior: Some(Box::new(prior)),
                },
                Inverse::RestoreSketchVisibility {
                    id,
                    prior: prior_vis,
                },
            ]),
        ))
    }

    fn rename_sketch(
        &mut self,
        id: SketchId,
        name: String,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let sketch = self
            .document
            .sketches
            .get_mut(&id)
            .ok_or_else(|| DomainError::Validation(format!("sketch {id} not found")))?;
        let prior = sketch.clone();
        sketch.name = name;
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::sketch(id)),
            Inverse::RestoreSketch {
                id,
                prior: Some(Box::new(prior)),
            },
        ))
    }

    fn update_sketch_attachment(
        &mut self,
        id: SketchId,
        plane: crate::sketch::SketchPlane,
        attachment: crate::sketch::SketchAttachment,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.sketches.contains_key(&id) {
            return Err(DomainError::Validation(format!("sketch {id} not found")));
        }
        let prior = self.document.sketches[&id].clone();
        // Re-attaching to a datum takes the same core-stamped frame add_sketch
        // does, so neither entry point can leave a datum-hosted sketch on a
        // frontend-supplied basis.
        let mut next = prior.clone();
        next.plane = plane;
        next.attachment = attachment;
        self.stamp_datum_plane(&mut next)?;
        self.document.sketches.insert(id, next);
        Ok((
            self.sketch_dirty_outcome(id),
            Inverse::RestoreSketch {
                id,
                prior: Some(Box::new(prior)),
            },
        ))
    }

    fn sketch_edit(
        &mut self,
        id: SketchId,
        ops: &[SketchEditOp],
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let prior = self
            .document
            .sketches
            .get(&id)
            .cloned()
            .ok_or_else(|| DomainError::Validation(format!("sketch {id} not found")))?;
        let next = apply_sketch_ops(&prior, ops)?;
        self.document.sketches.insert(id, next);
        Ok((
            self.sketch_dirty_outcome(id),
            Inverse::RestoreSketch {
                id,
                prior: Some(Box::new(prior)),
            },
        ))
    }

    fn sketch_drag(
        &mut self,
        id: SketchId,
        after: Sketch,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let prior = self
            .document
            .sketches
            .get(&id)
            .cloned()
            .ok_or_else(|| DomainError::Validation(format!("sketch {id} not found")))?;
        self.document.sketches.insert(id, after);
        Ok((
            self.sketch_dirty_outcome(id),
            Inverse::RestoreSketch {
                id,
                prior: Some(Box::new(prior)),
            },
        ))
    }

    // ── Body commands ────────────────────────────────────────────────────────

    fn add_body(&mut self, meta: BodyMeta) -> Result<(CommandOutcome, Inverse), DomainError> {
        let id = meta.id;
        let prior = self.document.bodies.clone();
        if !self.document.bodies.register(meta) {
            return Err(DomainError::Validation(format!("duplicate body id {id}")));
        }
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::body(id)),
            Inverse::RestoreBodies {
                registry: Box::new(prior),
            },
        ))
    }

    fn delete_body(&mut self, id: BodyId) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.bodies.contains(id) {
            return Err(DomainError::Validation(format!("body {id} not found")));
        }
        let prior = self.document.bodies.clone();
        self.document.bodies.remove(id);
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::body(id)),
            Inverse::RestoreBodies {
                registry: Box::new(prior),
            },
        ))
    }

    fn rename_body(
        &mut self,
        id: BodyId,
        name: String,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.bodies.contains(id) {
            return Err(DomainError::Validation(format!("body {id} not found")));
        }
        let prior = self.document.bodies.clone();
        self.document.bodies.set_name(id, name);
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::body(id)),
            Inverse::RestoreBodies {
                registry: Box::new(prior),
            },
        ))
    }

    fn set_visibility(
        &mut self,
        target: VisibilityTarget,
        visible: bool,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        match target {
            VisibilityTarget::Body(id) => {
                if !self.document.bodies.contains(id) {
                    return Err(DomainError::Validation(format!("body {id} not found")));
                }
                let prior = self.document.bodies.clone();
                self.document.bodies.set_visible(id, visible);
                Ok((
                    CommandOutcome::metadata_only(ProjectionDelta::body(id)),
                    Inverse::RestoreBodies {
                        registry: Box::new(prior),
                    },
                ))
            }
            VisibilityTarget::Sketch(id) => {
                if !self.document.sketches.contains_key(&id) {
                    return Err(DomainError::Validation(format!("sketch {id} not found")));
                }
                let prior = self.document.sketch_visibility.get(&id).copied();
                self.document.set_sketch_visible(id, visible);
                Ok((
                    CommandOutcome::metadata_only(ProjectionDelta::sketch(id)),
                    Inverse::RestoreSketchVisibility { id, prior },
                ))
            }
        }
    }

    // ── Datum commands ───────────────────────────────────────────────────────

    /// The frame `datum.base_plane_id` names: a world plane's basis, or another
    /// datum's already-resolved frame (chaining). `None` when the name is neither
    /// — an unknown id, or a base datum that never resolved.
    fn datum_base_frame(&self, datum: &DatumPlane) -> Option<crate::sketch::SketchPlane> {
        match datum.base_plane_id.as_str() {
            // NOTE the LEGACY-SWAPPED bases (sketch/plane.rs): "XZ" has world
            // normal +X and "YZ" has +Y, so an offset along the base normal moves
            // along those axes, not the intuitive ones. Ported verbatim from
            // `Sketch.h`; pinned by `offset_from_xz_slides_along_world_x`.
            "XY" => Some(crate::sketch::SketchPlane::xy()),
            "XZ" => Some(crate::sketch::SketchPlane::xz()),
            "YZ" => Some(crate::sketch::SketchPlane::yz()),
            other => {
                let id: crate::ids::DatumPlaneId = other.parse().ok()?;
                let base = self.document.datum_planes.get(&id)?;
                base.resolved_valid.then_some(base.resolved_plane)
            }
        }
    }

    /// Resolves a datum's frame with the context THE CORE can supply (V1: base
    /// world plane / base datum only). `OffsetFromFace` and `AngledFromEdge` need
    /// kernel-supplied geometry the core cannot derive, so they resolve to `None`
    /// and the datum is stored `resolved_valid == false` rather than guessing a
    /// frame — the same "never a silent wrong bind" rule the region and element
    /// paths follow.
    fn resolve_datum_frame(&self, datum: &DatumPlane) -> Option<crate::sketch::SketchPlane> {
        if datum.kind != DatumKind::OffsetFromPlane {
            return None;
        }
        let base = self.datum_base_frame(datum)?;
        resolve_datum(
            datum,
            &DatumContext {
                base,
                face: None,
                axis_dir: None,
                axis_origin: None,
            },
        )
    }

    fn add_datum(
        &mut self,
        mut datum: DatumPlane,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let id = datum.id;
        if self.document.datum_planes.contains_key(&id) {
            return Err(DomainError::Validation(format!("duplicate datum id {id}")));
        }
        // FROZEN policy: resolve HERE, at creation, and OVERWRITE whatever frame
        // the caller sent. Rust is the basis authority — a client-supplied
        // `resolved_plane` is never trusted, exactly as a sketch's frame is
        // stamped from the datum rather than round-tripped through the frontend.
        // Unresolvable definitions are stored LENIENTLY (`resolved_valid: false`)
        // instead of rejected, so a future import carrying a kind V1 cannot
        // resolve still loads; the guard that matters is in `add_sketch`, which
        // refuses to host a sketch on an unresolved datum.
        match self.resolve_datum_frame(&datum) {
            Some(plane) => datum.set_resolved(plane),
            None => {
                datum.resolved_plane = crate::sketch::SketchPlane::xy();
                datum.resolved_valid = false;
            }
        }
        self.document.datum_planes.insert(id, datum);
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::datum(id)),
            Inverse::RestoreDatum { id, prior: None },
        ))
    }

    fn delete_datum(
        &mut self,
        id: crate::ids::DatumPlaneId,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.datum_planes.contains_key(&id) {
            return Err(DomainError::Validation(format!("datum {id} not found")));
        }
        // Referenced-guard. A datum-hosted sketch's frame was STAMPED from this
        // datum at creation, so deleting it would leave the sketch pointing at a
        // dangling id with a frame nothing can re-derive. Reject loudly and NAME
        // the blockers so the user can act (mirrors the region-binding rule: a
        // stale reference fails with the available ids, never binds silently).
        let referencing: Vec<String> = self
            .document
            .sketches
            .values()
            .filter(|s| matches!(&s.attachment, SketchAttachment::Datum { datum } if *datum == id))
            .map(|s| format!("{} ({})", s.name, s.id))
            .collect();
        if !referencing.is_empty() {
            return Err(DomainError::Validation(format!(
                "datum {id} is referenced by {} sketch(es): {}",
                referencing.len(),
                referencing.join(", ")
            )));
        }
        let prior = self
            .document
            .datum_planes
            .remove(&id)
            .expect("presence checked above");
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::datum(id)),
            Inverse::RestoreDatum {
                id,
                prior: Some(Box::new(prior)),
            },
        ))
    }

    // ── Variable commands ────────────────────────────────────────────────────

    fn set_variable(
        &mut self,
        var_id: crate::ids::VariableId,
        value: Scalar,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let existing = self
            .document
            .variables
            .iter()
            .find(|v| v.id == var_id)
            .cloned()
            .ok_or_else(|| DomainError::Validation(format!("variable {var_id} not found")))?;
        let prior = self.document.variables.clone();
        self.document
            .variables
            .upsert(Variable { value, ..existing });
        Ok((
            self.variable_outcome(),
            Inverse::RestoreVariables {
                table: Box::new(prior),
            },
        ))
    }

    fn add_variable(&mut self, var: Variable) -> Result<(CommandOutcome, Inverse), DomainError> {
        if self.document.variables.get(&var.name).is_some() {
            return Err(DomainError::Validation(format!(
                "duplicate variable name {}",
                var.name
            )));
        }
        let prior = self.document.variables.clone();
        self.document.variables.upsert(var);
        Ok((
            self.variable_outcome(),
            Inverse::RestoreVariables {
                table: Box::new(prior),
            },
        ))
    }

    fn remove_variable(
        &mut self,
        var_id: crate::ids::VariableId,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.variables.iter().any(|v| v.id == var_id) {
            return Err(DomainError::Validation(format!(
                "variable {var_id} not found"
            )));
        }
        let prior = self.document.variables.clone();
        let mut vt = VariableTable::new();
        for v in self.document.variables.iter() {
            if v.id != var_id {
                vt.upsert(v.clone());
            }
        }
        self.document.variables = vt;
        Ok((
            self.variable_outcome(),
            Inverse::RestoreVariables {
                table: Box::new(prior),
            },
        ))
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn rebuild_graph(&mut self) {
        self.graph
            .rebuild_from_records(self.document.timeline.records());
    }

    /// Validates that every input body of `record_id` is produced strictly
    /// before it (anti-time-travel; C++ `DependencyGraph::producesBefore`).
    fn validate_temporal(
        &self,
        records: &[OperationRecord],
        record_id: RecordId,
        input_bodies: &[BodyId],
    ) -> Result<(), DomainError> {
        let mut g = DependencyGraph::new();
        g.rebuild_from_records(records);
        for &b in input_bodies {
            if !g.produces_before(b, record_id) {
                return Err(DomainError::Timeline(format!(
                    "op {record_id} references body {b} produced by a later operation (anti-time-travel)"
                )));
            }
        }
        Ok(())
    }

    /// Outcome for a record-content edit at `index`: dirties `[index, len)`,
    /// regen to end.
    fn dirty_record_outcome(&self, index: usize, inverse: Inverse) -> (CommandOutcome, Inverse) {
        let dirty = DirtyRange::new(index, self.document.timeline.len());
        (
            CommandOutcome::dirty_to_end(ProjectionDelta::timeline(), dirty),
            inverse,
        )
    }

    /// Outcome for a sketch edit: dirties to the end from the earliest affected
    /// step (regen to end); metadata-only if nothing depends on the sketch.
    ///
    /// The earliest step is `min(producer, first consumer)` (F4). When a `Sketch`
    /// op produces this `SketchId` in the timeline, that op's own regen re-runs
    /// region detection worker-side, so an edit must dirty from the producer — not
    /// merely from the first op that consumes a region. With no producer op (a
    /// document sketch that never became a timeline node) it falls back to the
    /// first consumer, and to metadata-only when nothing references it.
    fn sketch_dirty_outcome(&self, id: SketchId) -> CommandOutcome {
        let delta = ProjectionDelta::sketch(id);
        let producer_step = self
            .graph
            .sketch_producer(id)
            .and_then(|rid| self.document.timeline.index_of(rid));
        let consumer_step = self.first_step_referencing_sketch(id);
        let start = match (producer_step, consumer_step) {
            (Some(p), Some(c)) => Some(p.min(c)),
            (Some(s), None) | (None, Some(s)) => Some(s),
            (None, None) => None,
        };
        match start {
            Some(step) => CommandOutcome::dirty_to_end(
                delta,
                DirtyRange::new(step, self.document.timeline.len()),
            ),
            None => CommandOutcome::metadata_only(delta),
        }
    }

    /// Conservative outcome for a variable edit: dirties the whole applied
    /// timeline (a bare-variable expression can drive any op).
    fn variable_outcome(&self) -> CommandOutcome {
        let delta = ProjectionDelta {
            variables_changed: true,
            ..ProjectionDelta::default()
        };
        let len = self.document.timeline.len();
        if len == 0 {
            CommandOutcome::metadata_only(delta)
        } else {
            CommandOutcome::dirty_to_end(delta, DirtyRange::new(0, len))
        }
    }

    /// The first timeline index whose op references `sketch` as an input.
    fn first_step_referencing_sketch(&self, sketch: SketchId) -> Option<usize> {
        self.document
            .timeline
            .records()
            .iter()
            .position(|r| r.op.derive_inputs().sketches.contains(&sketch))
    }
}

// ── Free helpers ─────────────────────────────────────────────────────────────

fn empty_outcome() -> CommandOutcome {
    CommandOutcome {
        projection_delta: ProjectionDelta::default(),
        dirty: None,
        regen: RegenHint::None,
    }
}

/// Merges `o` into `acc` (transaction batching).
fn merge_outcome(acc: &mut CommandOutcome, o: &CommandOutcome) {
    acc.projection_delta.merge(&o.projection_delta);
    acc.dirty = union_dirty(acc.dirty, o.dirty);
    acc.regen = stronger_regen(acc.regen, o.regen);
}

fn union_dirty(a: Option<DirtyRange>, b: Option<DirtyRange>) -> Option<DirtyRange> {
    match (a, b) {
        (Some(x), Some(y)) => Some(DirtyRange::new(x.from.min(y.from), x.to.max(y.to))),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn stronger_regen(a: RegenHint, b: RegenHint) -> RegenHint {
    use RegenHint::{None, PreviewTo, ToEnd};
    match (a, b) {
        (ToEnd, _) | (_, ToEnd) => ToEnd,
        (PreviewTo(x), PreviewTo(y)) => PreviewTo(x.max(y)),
        (PreviewTo(x), None) | (None, PreviewTo(x)) => PreviewTo(x),
        (None, None) => None,
    }
}

/// Validates fillet/chamfer edge consistency (F2): when the typed `edges` list
/// is populated it must stay in lockstep with the bare `edge_ids` — equal length,
/// and each typed ref's `primary.element` (when present) must equal the parallel
/// `edge_ids` entry. An intent-only ref (`primary` = `None`) is accepted
/// positionally (matched by count alone). An empty `edges` list is the legacy
/// bare-ids form and is always accepted (the operated body binds at regen time).
/// Non-fillet/chamfer ops and opaque ops are trivially valid.
fn validate_fillet_lockstep(op: &Operation) -> Result<(), DomainError> {
    let (edge_ids, edges) = match op {
        Operation::Known(KnownOperation::Fillet(p)) => (&p.edge_ids, &p.edges),
        Operation::Known(KnownOperation::Chamfer(p)) => (&p.edge_ids, &p.edges),
        _ => return Ok(()),
    };
    if edges.is_empty() {
        return Ok(());
    }
    if edges.len() != edge_ids.len() {
        return Err(DomainError::Validation(format!(
            "fillet/chamfer edges ({}) and edgeIds ({}) length mismatch",
            edges.len(),
            edge_ids.len()
        )));
    }
    for (i, e) in edges.iter().enumerate() {
        if let Some(primary) = &e.primary {
            if primary.element != edge_ids[i] {
                return Err(DomainError::Validation(format!(
                    "fillet/chamfer edge {i}: typed ref element {} != edgeIds[{i}] {}",
                    primary.element, edge_ids[i]
                )));
            }
        }
    }
    Ok(())
}

/// True iff two operations have the same `opType` (params update may not change it).
fn same_op_type(a: &Operation, b: &Operation) -> bool {
    match (a, b) {
        (Operation::Known(x), Operation::Known(y)) => {
            std::mem::discriminant(x) == std::mem::discriminant(y)
        }
        (Operation::Opaque(x), Operation::Opaque(y)) => x.raw.get("opType") == y.raw.get("opType"),
        _ => false,
    }
}

/// Writes `reference` into the op input slot named by `path`, keeping fillet
/// `edge_ids`/`edges` in lockstep. See [`crate::edit::command`] for the divergence.
fn set_input(
    op: &mut Operation,
    path: &InputPath,
    reference: &InputRef,
) -> Result<(), DomainError> {
    let known = match op {
        Operation::Known(k) => k,
        Operation::Opaque(_) => {
            return Err(DomainError::InvalidReference(
                "cannot edit inputs of a frozen (opaque) op".into(),
            ))
        }
    };
    match (path, known) {
        (InputPath::ExtrudeProfile, KnownOperation::Extrude(p)) => {
            p.profile = Some(want_region(reference)?);
        }
        (InputPath::ExtrudeTargetFace { second }, KnownOperation::Extrude(p)) => {
            let er = want_element(reference)?;
            if *second {
                p.target_face2 = Some(er);
            } else {
                p.target_face = Some(er);
            }
        }
        (InputPath::RevolveAxis, KnownOperation::Revolve(p)) => {
            p.axis = Some(want_axis(reference)?);
        }
        (InputPath::ExtrudeProfile, KnownOperation::Revolve(p)) => {
            p.profile = Some(want_region(reference)?);
        }
        (InputPath::FilletEdges { index }, KnownOperation::Fillet(p)) => {
            set_fillet_edge(
                &mut p.edges,
                &mut p.edge_ids,
                *index,
                want_element(reference)?,
            )?;
        }
        (InputPath::FilletEdges { index }, KnownOperation::Chamfer(p)) => {
            set_fillet_edge(
                &mut p.edges,
                &mut p.edge_ids,
                *index,
                want_element(reference)?,
            )?;
        }
        (InputPath::BooleanTarget, KnownOperation::Boolean(p)) => {
            p.target_body = want_body(reference)?;
        }
        (InputPath::BooleanTool, KnownOperation::Boolean(p)) => {
            p.tool_body = want_body(reference)?;
        }
        _ => {
            return Err(DomainError::InvalidReference(
                "input path does not match the operation type / reference kind".into(),
            ))
        }
    }
    Ok(())
}

/// Sets fillet/chamfer edge `index`, keeping the typed `edges` ref and the bare
/// `edge_ids` element id consistent (both must be populated — see the
/// `FilletParams` doc). The typed ref must carry a `primary` element id.
fn set_fillet_edge(
    edges: &mut Vec<ElementRef>,
    edge_ids: &mut Vec<crate::ids::ElementId>,
    index: usize,
    reference: ElementRef,
) -> Result<(), DomainError> {
    let element = reference
        .primary
        .as_ref()
        .map(|p| p.element.clone())
        .ok_or_else(|| {
            DomainError::InvalidReference(
                "a fillet/chamfer edge ref must carry a primary element id".into(),
            )
        })?;
    // The bare `edge_ids` and typed `edges` lists must stay the same length and
    // in lockstep (see `FilletParams`). Overwrite an existing slot or append one;
    // a gap beyond the end is an out-of-range edit.
    let len = edges.len().max(edge_ids.len());
    if index > len {
        return Err(DomainError::InvalidReference(format!(
            "fillet edge index {index} out of range (len {len})"
        )));
    }
    if index == edges.len() {
        edges.push(reference);
    } else {
        edges[index] = reference;
    }
    if index == edge_ids.len() {
        edge_ids.push(element);
    } else {
        edge_ids[index] = element;
    }
    Ok(())
}

fn want_element(r: &InputRef) -> Result<ElementRef, DomainError> {
    match r {
        InputRef::Element(e) => Ok(e.clone()),
        _ => Err(DomainError::InvalidReference(
            "expected an element ref".into(),
        )),
    }
}
fn want_region(r: &InputRef) -> Result<crate::document::refs::SketchRegionRef, DomainError> {
    match r {
        InputRef::Region(reg) => Ok(reg.clone()),
        _ => Err(DomainError::InvalidReference(
            "expected a region ref".into(),
        )),
    }
}
fn want_axis(r: &InputRef) -> Result<crate::document::refs::AxisRef, DomainError> {
    match r {
        InputRef::Axis(a) => Ok(a.clone()),
        _ => Err(DomainError::InvalidReference("expected an axis ref".into())),
    }
}
fn want_body(r: &InputRef) -> Result<BodyId, DomainError> {
    match r {
        InputRef::Body(b) => Ok(*b),
        _ => Err(DomainError::InvalidReference("expected a body ref".into())),
    }
}

/// Applies a batch of sketch edits to a copy of `prior`, preserving entity /
/// constraint order (SetDimension/SetEntityPositions mutate in place). Validation
/// (dup id / dangling ref) runs on the rebuilt sketch via its `add_*` API.
///
/// **Reference-locked geometry (L1 guard).** Every op that MUTATES existing
/// geometry — remove (including anything the cascade would take), reposition,
/// construction flip — is refused with [`SketchError::ReferenceLocked`] when it
/// names locked host-face geometry. The refusal happens before any working-vec
/// mutation, so the batch is atomic and the caller's memento/inverse is never
/// captured for a rejected edit. Adding a constraint against locked geometry is
/// deliberately NOT guarded (that is how a profile snaps to the face boundary).
fn apply_sketch_ops(prior: &Sketch, ops: &[SketchEditOp]) -> Result<Sketch, DomainError> {
    let mut entities: Vec<SketchEntity> = prior.entities().to_vec();
    let mut constraints: Vec<Constraint> = prior.constraints().to_vec();

    let locked = |entities: &[SketchEntity], id: crate::ids::EntityId| -> Result<(), DomainError> {
        match entities.iter().find(|e| e.id() == id) {
            Some(e) if e.is_reference_locked() => Err(sketch_err(SketchError::ReferenceLocked(id))),
            _ => Ok(()),
        }
    };

    for op in ops {
        match op {
            SketchEditOp::AddEntity { entity } => entities.push(entity.clone()),
            SketchEditOp::RemoveEntity { entity } => {
                // The cascade takes everything that references the seed, so the
                // guard has to cover the whole doomed set — otherwise removing a
                // free point would silently delete the locked line hanging off it.
                for doomed in doomed_by_remove(&entities, *entity) {
                    locked(&entities, doomed)?;
                }
                entities = cascade_remove_entity(&entities, &mut constraints, *entity);
            }
            SketchEditOp::AddConstraint { constraint } => {
                validate_positive_dimension(constraint)?;
                constraints.push(constraint.clone());
            }
            SketchEditOp::RemoveConstraint { constraint } => {
                constraints.retain(|c| c.id() != *constraint);
            }
            SketchEditOp::SetDimension { constraint, value } => {
                let c = constraints
                    .iter_mut()
                    .find(|c| c.id() == *constraint)
                    .ok_or_else(|| {
                        DomainError::Validation(format!("constraint {constraint} not found"))
                    })?;
                let next = constraint_with_value(c, value.clone()).ok_or_else(|| {
                    DomainError::Validation(format!("constraint {constraint} is not dimensional"))
                })?;
                validate_positive_dimension(&next)?;
                *c = next;
            }
            SketchEditOp::SetEntityPositions { positions } => {
                for (eid, _) in positions {
                    locked(&entities, *eid)?;
                }
                for (eid, at) in positions {
                    let e = entities
                        .iter_mut()
                        .find(|e| e.id() == *eid)
                        .ok_or_else(|| {
                            DomainError::Validation(format!("entity {eid} not found"))
                        })?;
                    *e = entity_with_position(e, *at).ok_or_else(|| {
                        DomainError::Validation(format!("entity {eid} is not a point"))
                    })?;
                }
            }
            SketchEditOp::SetEntityConstruction {
                entity,
                construction,
            } => {
                locked(&entities, *entity)?;
                let e = entities
                    .iter_mut()
                    .find(|e| e.id() == *entity)
                    .ok_or_else(|| DomainError::Validation(format!("entity {entity} not found")))?;
                *e = entity_with_construction(e, *construction);
            }
        }
    }
    rebuild_sketch(prior, entities, constraints)
}

/// `seed` plus, to a fixpoint, every entity that transitively references it —
/// exactly the set [`cascade_remove_entity`] would drop. Split out so the
/// reference-lock guard can inspect the doomed set BEFORE anything is removed.
fn doomed_by_remove(
    entities: &[SketchEntity],
    seed: crate::ids::EntityId,
) -> Vec<crate::ids::EntityId> {
    let mut removed = vec![seed];
    loop {
        let newly: Vec<_> = entities
            .iter()
            .filter(|e| {
                !removed.contains(&e.id())
                    && e.referenced_entities().iter().any(|r| removed.contains(r))
            })
            .map(SketchEntity::id)
            .collect();
        if newly.is_empty() {
            return removed;
        }
        removed.extend(newly);
    }
}

/// Removes `seed` and, to a fixpoint, every entity that transitively references
/// it, then drops any constraint that referenced a removed entity.
fn cascade_remove_entity(
    entities: &[SketchEntity],
    constraints: &mut Vec<Constraint>,
    seed: crate::ids::EntityId,
) -> Vec<SketchEntity> {
    let removed = doomed_by_remove(entities, seed);
    constraints.retain(|c| !c.entities().iter().any(|r| removed.contains(r)));
    entities
        .iter()
        .filter(|e| !removed.contains(&e.id()))
        .cloned()
        .collect()
}

/// Rebuilds a validated [`Sketch`] from working entity/constraint vecs,
/// preserving `prior`'s identity/plane/attachment/regions.
fn rebuild_sketch(
    prior: &Sketch,
    entities: Vec<SketchEntity>,
    constraints: Vec<Constraint>,
) -> Result<Sketch, DomainError> {
    let mut s = Sketch::new(prior.id, prior.name.clone(), prior.attachment.clone());
    s.plane = prior.plane;
    s.regions = prior.regions.clone();
    s.extra = prior.extra.clone();
    for e in entities {
        s.add_entity(e).map_err(sketch_err)?;
    }
    for c in constraints {
        s.add_constraint(c).map_err(sketch_err)?;
    }
    Ok(s)
}

fn sketch_err(e: SketchError) -> DomainError {
    DomainError::Validation(e.to_string())
}

/// Rejects a non-positive or non-finite value for the **strictly-positive**
/// dimensional kinds (Distance, Radius, Diameter). `HorizontalDistance` /
/// `VerticalDistance` are SIGNED (`x2−x1` / `y2−y1`) and `Angle` is a UI-domain
/// value, so none of those is guarded (see [`crate::sketch::constraint`]).
fn validate_positive_dimension(c: &Constraint) -> Result<(), DomainError> {
    let (kind, v) = match c {
        Constraint::Distance { value, .. } => ("distance", value.value),
        Constraint::Radius { value, .. } => ("radius", value.value),
        Constraint::Diameter { value, .. } => ("diameter", value.value),
        _ => return Ok(()),
    };
    if !v.is_finite() || v <= 0.0 {
        return Err(DomainError::Validation(format!(
            "{kind} must be positive (got {v})"
        )));
    }
    Ok(())
}

/// Returns a clone of `c` with its dimensional value replaced, or `None` for a
/// geometric constraint.
fn constraint_with_value(c: &Constraint, value: Scalar) -> Option<Constraint> {
    Some(match c {
        Constraint::Distance {
            id,
            entity1,
            entity2,
            ..
        } => Constraint::Distance {
            id: *id,
            entity1: *entity1,
            entity2: *entity2,
            value,
        },
        Constraint::HorizontalDistance {
            id, point1, point2, ..
        } => Constraint::HorizontalDistance {
            id: *id,
            point1: *point1,
            point2: *point2,
            value,
        },
        Constraint::VerticalDistance {
            id, point1, point2, ..
        } => Constraint::VerticalDistance {
            id: *id,
            point1: *point1,
            point2: *point2,
            value,
        },
        Constraint::Angle {
            id, line1, line2, ..
        } => Constraint::Angle {
            id: *id,
            line1: *line1,
            line2: *line2,
            value,
        },
        Constraint::Radius { id, entity, .. } => Constraint::Radius {
            id: *id,
            entity: *entity,
            value,
        },
        Constraint::Diameter { id, entity, .. } => Constraint::Diameter {
            id: *id,
            entity: *entity,
            value,
        },
        _ => return None,
    })
}

/// Returns a clone of `e` at position `at`, or `None` if `e` is not a point.
fn entity_with_position(e: &SketchEntity, at: Vec2) -> Option<SketchEntity> {
    match e {
        SketchEntity::Point {
            id,
            construction,
            reference_locked,
            ..
        } => Some(SketchEntity::Point {
            id: *id,
            at,
            construction: *construction,
            reference_locked: *reference_locked,
        }),
        _ => None,
    }
}

/// Returns a clone of `e` with its construction flag replaced. Total over all
/// five entity kinds — construction is a property of every entity, unlike
/// position (points only), so there is no `None` arm.
fn entity_with_construction(e: &SketchEntity, construction: bool) -> SketchEntity {
    match e {
        SketchEntity::Point {
            id,
            at,
            reference_locked,
            ..
        } => SketchEntity::Point {
            id: *id,
            at: *at,
            construction,
            reference_locked: *reference_locked,
        },
        SketchEntity::Line {
            id,
            start,
            end,
            reference_locked,
            ..
        } => SketchEntity::Line {
            id: *id,
            start: *start,
            end: *end,
            construction,
            reference_locked: *reference_locked,
        },
        SketchEntity::Arc {
            id,
            center,
            radius,
            start_angle,
            end_angle,
            reference_locked,
            ..
        } => SketchEntity::Arc {
            id: *id,
            center: *center,
            radius: *radius,
            start_angle: *start_angle,
            end_angle: *end_angle,
            construction,
            reference_locked: *reference_locked,
        },
        SketchEntity::Circle {
            id,
            center,
            radius,
            reference_locked,
            ..
        } => SketchEntity::Circle {
            id: *id,
            center: *center,
            radius: *radius,
            construction,
            reference_locked: *reference_locked,
        },
        SketchEntity::Ellipse {
            id,
            center,
            major_r,
            minor_r,
            rotation,
            reference_locked,
            ..
        } => SketchEntity::Ellipse {
            id: *id,
            center: *center,
            major_r: *major_r,
            minor_r: *minor_r,
            rotation: *rotation,
            construction,
            reference_locked: *reference_locked,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ConstraintId, EntityId};
    use crate::sketch::WorldPlane;
    use uuid::Uuid;

    fn sid() -> SketchId {
        SketchId(Uuid::from_u128(1))
    }
    fn eid(n: u128) -> EntityId {
        EntityId(Uuid::from_u128(n))
    }
    fn cid(n: u128) -> ConstraintId {
        ConstraintId(Uuid::from_u128(n))
    }

    /// Two free points + a circle centered on the first — enough to hang a
    /// Radius/Distance dimension off.
    fn base_sketch() -> Sketch {
        let mut s = Sketch::on_world_plane(sid(), "T", WorldPlane::XY);
        s.add_entity(SketchEntity::point(
            eid(1),
            Vec2::new_unchecked(0.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(SketchEntity::point(
            eid(2),
            Vec2::new_unchecked(10.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(SketchEntity::circle(eid(3), eid(1), 5.0, false).unwrap())
            .unwrap();
        s
    }

    #[test]
    fn add_constraint_rejects_non_positive_radius() {
        let s = base_sketch();
        // value == 0.0 is rejected (strictly positive).
        let zero = vec![SketchEditOp::AddConstraint {
            constraint: Constraint::Radius {
                id: cid(1),
                entity: eid(3),
                value: Scalar::new(0.0),
            },
        }];
        assert!(matches!(
            apply_sketch_ops(&s, &zero),
            Err(DomainError::Validation(_))
        ));
        // a negative radius is likewise rejected.
        let neg = vec![SketchEditOp::AddConstraint {
            constraint: Constraint::Radius {
                id: cid(1),
                entity: eid(3),
                value: Scalar::new(-2.0),
            },
        }];
        assert!(matches!(
            apply_sketch_ops(&s, &neg),
            Err(DomainError::Validation(_))
        ));
        // a positive radius is accepted.
        let ok = vec![SketchEditOp::AddConstraint {
            constraint: Constraint::Radius {
                id: cid(1),
                entity: eid(3),
                value: Scalar::new(5.0),
            },
        }];
        assert!(apply_sketch_ops(&s, &ok).is_ok());
    }

    #[test]
    fn set_dimension_rejects_non_positive_distance() {
        let mut s = base_sketch();
        s.add_constraint(Constraint::Distance {
            id: cid(1),
            entity1: eid(1),
            entity2: eid(2),
            value: Scalar::new(10.0),
        })
        .unwrap();
        // driving the Distance to <= 0 is rejected.
        let ops = vec![SketchEditOp::SetDimension {
            constraint: cid(1),
            value: Scalar::new(-3.0),
        }];
        assert!(matches!(
            apply_sketch_ops(&s, &ops),
            Err(DomainError::Validation(_))
        ));
        // a positive re-dimension is accepted.
        let ok = vec![SketchEditOp::SetDimension {
            constraint: cid(1),
            value: Scalar::new(25.0),
        }];
        assert!(apply_sketch_ops(&s, &ok).is_ok());
    }

    #[test]
    fn set_entity_construction_flips_only_the_named_entity() {
        let s = base_sketch();
        assert!(s.entities().iter().all(|e| !e.is_construction()));

        let flip = vec![SketchEditOp::SetEntityConstruction {
            entity: eid(3), // the circle
            construction: true,
        }];
        let out = apply_sketch_ops(&s, &flip).expect("flip accepted");
        assert!(
            out.get_entity(eid(3)).unwrap().is_construction(),
            "the named entity became construction"
        );
        assert!(
            !out.get_entity(eid(1)).unwrap().is_construction()
                && !out.get_entity(eid(2)).unwrap().is_construction(),
            "no other entity is touched — not even the circle's own center point"
        );
        // Entity order and the rest of the shape survive the rebuild.
        assert_eq!(
            out.entities()
                .iter()
                .map(SketchEntity::id)
                .collect::<Vec<_>>(),
            s.entities()
                .iter()
                .map(SketchEntity::id)
                .collect::<Vec<_>>()
        );

        // …and flipping back is exact.
        let back = apply_sketch_ops(
            &out,
            &[SketchEditOp::SetEntityConstruction {
                entity: eid(3),
                construction: false,
            }],
        )
        .expect("flip back accepted");
        assert_eq!(back.entities(), s.entities());
    }

    /// `stamp_datum_plane` is a DATUM-only stamp. A host-face sketch carries a
    /// frame the worker derived from the face itself; overwriting it would
    /// silently reproject every entity coordinate, and the projected boundary
    /// (the reference-locked geometry a later wave lands) is expressed in it.
    /// The `let … else` on the attachment is what keeps that frame frozen — this
    /// pins it for both entry points.
    #[test]
    fn stamp_datum_plane_no_ops_for_a_host_face_sketch() {
        use crate::document::refs::ElementRef;
        use crate::document::Document;
        use crate::ids::DocumentId;
        use crate::sketch::SketchPlane;

        // A frame that is NOT any named world plane, so a stray stamp shows up.
        let frozen = SketchPlane {
            origin: crate::math::Vec3::new_unchecked(1.0, 2.0, 3.0),
            ..SketchPlane::xz()
        };
        let mut authored = Sketch::new(
            sid(),
            "on face",
            SketchAttachment::HostFace {
                face: ElementRef {
                    primary: None,
                    intent: None,
                    anchor: None,
                    extra: Default::default(),
                },
                projected_boundary_version: 0,
            },
        );
        authored.plane = frozen;

        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        // No datum planes exist at all: a stamp would have to ERROR, so passing
        // proves the host-face arm returned early.
        session
            .apply(EditCommand::AddSketch {
                sketch: authored.clone(),
            })
            .expect("a host-face sketch needs no datum");
        assert_eq!(session.document().sketches[&sid()].plane, frozen);

        // The same holds on the re-attach entry point.
        session
            .apply(EditCommand::UpdateSketchAttachment {
                sketch: sid(),
                plane: frozen,
                attachment: authored.attachment.clone(),
            })
            .expect("re-attaching a host-face sketch needs no datum");
        assert_eq!(session.document().sketches[&sid()].plane, frozen);
    }

    /// A sketch whose circle (and its center point) is locked host-face geometry.
    fn locked_sketch() -> Sketch {
        let mut s = Sketch::on_world_plane(sid(), "T", WorldPlane::XY);
        s.add_entity(SketchEntity::point(
            eid(1),
            Vec2::new_unchecked(0.0, 0.0),
            false,
            true,
        ))
        .unwrap();
        s.add_entity(SketchEntity::point(
            eid(2),
            Vec2::new_unchecked(10.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(
            SketchEntity::circle(eid(3), eid(1), 5.0, false)
                .unwrap()
                .with_reference_locked(true),
        )
        .unwrap();
        s
    }

    #[test]
    fn geometry_edits_against_locked_entities_are_refused() {
        let s = locked_sketch();

        let refused = |ops: Vec<SketchEditOp>, what: &str| {
            let err = apply_sketch_ops(&s, &ops).expect_err(what);
            let DomainError::Validation(msg) = err else {
                panic!("{what}: expected a validation refusal");
            };
            assert!(
                msg.contains("reference-locked"),
                "{what}: refusal must name the lock, got {msg}"
            );
        };

        refused(
            vec![SketchEditOp::RemoveEntity { entity: eid(3) }],
            "removing the locked circle",
        );
        // The CASCADE is guarded too: the free center point is removable in
        // isolation, but taking it would drag the locked circle with it.
        refused(
            vec![SketchEditOp::RemoveEntity { entity: eid(1) }],
            "removing a point the locked circle references",
        );
        refused(
            vec![SketchEditOp::SetEntityPositions {
                positions: vec![(eid(1), Vec2::new_unchecked(1.0, 1.0))],
            }],
            "moving the locked center point",
        );
        refused(
            vec![SketchEditOp::SetEntityConstruction {
                entity: eid(3),
                construction: true,
            }],
            "flipping the locked circle to construction",
        );

        // …and nothing locked was touched by any of the rejected batches.
        assert_eq!(s.entities(), locked_sketch().entities());
    }

    #[test]
    fn locked_geometry_still_accepts_constraints_and_free_edits() {
        let s = locked_sketch();
        // A dimension ON the locked circle is allowed — that is how a profile
        // binds to the projected face boundary.
        let dim = apply_sketch_ops(
            &s,
            &[SketchEditOp::AddConstraint {
                constraint: Constraint::Radius {
                    id: cid(1),
                    entity: eid(3),
                    value: Scalar::new(5.0),
                },
            }],
        )
        .expect("a constraint may reference locked geometry");
        assert_eq!(dim.constraints().len(), 1);

        // The FREE point next to it still moves.
        let moved = apply_sketch_ops(
            &s,
            &[SketchEditOp::SetEntityPositions {
                positions: vec![(eid(2), Vec2::new_unchecked(3.0, 4.0))],
            }],
        )
        .expect("a free point still moves");
        assert!(matches!(
            moved.get_entity(eid(2)).unwrap(),
            SketchEntity::Point { at, .. } if [at.x, at.y] == [3.0, 4.0]
        ));
    }

    #[test]
    fn set_entity_construction_rejects_an_unknown_entity() {
        let s = base_sketch();
        let ops = vec![SketchEditOp::SetEntityConstruction {
            entity: eid(999),
            construction: true,
        }];
        assert!(matches!(
            apply_sketch_ops(&s, &ops),
            Err(DomainError::Validation(_))
        ));
    }

    /// The `SketchEdit` memento (`Inverse::RestoreSketch`) makes the inverse free
    /// — no per-op undo arm is needed for the new op.
    #[test]
    fn set_entity_construction_undo_restores_via_the_memento() {
        use crate::document::Document;
        use crate::ids::DocumentId;

        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        session
            .apply(EditCommand::AddSketch {
                sketch: base_sketch(),
            })
            .unwrap();
        session
            .apply(EditCommand::SketchEdit {
                sketch: sid(),
                ops: vec![SketchEditOp::SetEntityConstruction {
                    entity: eid(3),
                    construction: true,
                }],
            })
            .unwrap();
        assert!(session
            .document()
            .sketch(sid())
            .unwrap()
            .get_entity(eid(3))
            .unwrap()
            .is_construction());

        assert!(session.undo(), "undo available");
        assert!(
            !session
                .document()
                .sketch(sid())
                .unwrap()
                .get_entity(eid(3))
                .unwrap()
                .is_construction(),
            "undo restores the prior sketch verbatim"
        );
        assert!(session.redo().unwrap(), "redo available");
        assert!(session
            .document()
            .sketch(sid())
            .unwrap()
            .get_entity(eid(3))
            .unwrap()
            .is_construction());
    }

    /// Wire pin — the exact JSON the frontend will author (mirror in
    /// `src/ipc/sketchWireMap.ts` when W1-B wires the UI).
    #[test]
    fn set_entity_construction_wire_shape() {
        let uid = "00000000-0000-0000-0000-000000000003";
        let wire = serde_json::json!({
            "op": "setEntityConstruction", "entity": uid, "construction": true
        });
        let op: SketchEditOp = serde_json::from_value(wire.clone()).expect("op parses");
        let SketchEditOp::SetEntityConstruction {
            entity,
            construction,
        } = op
        else {
            panic!("wrong variant");
        };
        assert_eq!(entity, eid(3));
        assert!(construction);
        assert_eq!(
            serde_json::to_value(SketchEditOp::SetEntityConstruction {
                entity: eid(3),
                construction: true
            })
            .unwrap(),
            wire,
            "serialization is byte-stable against the documented shape"
        );
    }

    #[test]
    fn squash_skipped_when_run_not_contiguous() {
        use crate::document::Document;
        use crate::ids::DocumentId;

        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        session
            .apply(EditCommand::AddSketch {
                sketch: base_sketch(),
            })
            .unwrap();
        // Snapshot the pre-session sketch + watermark (as the runtime would on enter).
        let prior = session.document().sketch(sid()).unwrap().clone();
        let watermark = session.undo_depth();

        let add_point = |sk: SketchId, id: u128, x: f64, y: f64| EditCommand::SketchEdit {
            sketch: sk,
            ops: vec![SketchEditOp::AddEntity {
                entity: SketchEntity::point(eid(id), Vec2::new_unchecked(x, y), false, false),
            }],
        };

        // (1) a sketch edit for THIS sketch …
        session.apply(add_point(sid(), 50, 1.0, 1.0)).unwrap();
        // (2) … an INTERLEAVED model op (a different sketch add — not a SketchEdit
        //     for sid(), so the pred rejects it) …
        session
            .apply(EditCommand::AddSketch {
                sketch: Sketch::on_world_plane(
                    SketchId(Uuid::from_u128(2)),
                    "Other",
                    WorldPlane::XY,
                ),
            })
            .unwrap();
        // (3) … then another sketch edit for THIS sketch.
        session.apply(add_point(sid(), 51, 2.0, 2.0)).unwrap();

        let count = session.undo_depth() - watermark; // 3
        let depth_before = session.undo_depth();
        session.squash_sketch_session(sid(), prior, count);
        assert_eq!(
            session.undo_depth(),
            depth_before,
            "the interleaved model op breaks the trailing run — NO squash, granular steps kept"
        );

        // The critical property: undo of the HEAD sketch edit reverts only that edit
        // (entity 51), NOT the whole session — the granular inverses stay sound.
        assert!(session.undo(), "undo the head sketch edit");
        let s = session.document().sketch(sid()).unwrap();
        assert!(
            s.entities().iter().any(|e| e.id() == eid(50)),
            "the first in-session edit survives (no spurious pre-session restore)"
        );
        assert!(
            !s.entities().iter().any(|e| e.id() == eid(51)),
            "only the head edit was undone"
        );
    }

    #[test]
    fn horizontal_distance_accepts_negative_value() {
        let s = base_sketch();
        // HorizontalDistance is SIGNED (x2 − x1): a negative value is legal on both
        // AddConstraint and SetDimension (non-regression for the positive guard).
        let add = vec![SketchEditOp::AddConstraint {
            constraint: Constraint::HorizontalDistance {
                id: cid(1),
                point1: eid(1),
                point2: eid(2),
                value: Scalar::new(-10.0),
            },
        }];
        let out = apply_sketch_ops(&s, &add).expect("negative HorizontalDistance accepted");
        let set = vec![SketchEditOp::SetDimension {
            constraint: cid(1),
            value: Scalar::new(-20.0),
        }];
        assert!(
            apply_sketch_ops(&out, &set).is_ok(),
            "negative signed distance via SetDimension stays accepted"
        );
    }
}
