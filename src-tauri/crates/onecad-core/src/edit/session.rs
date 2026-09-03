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
//! | `SetRollback` | span from `set_cursor` | `PreviewTo(cursor − 1)` (`None` when the cursor does not move) |
//! | `SetOperationSuppression` | `[step, len)` | `ToEnd` |
//! | `AddSketch` | first dependent op step (`None` if none) | `ToEnd` / `None` |
//! | `DeleteSketch` / `UpdateSketchAttachment` / `SketchEdit` / `SketchDragGesture` | `min(producing Sketch op, first dependent op)` | `ToEnd` / `None` |
//! | `RenameSketch` | — | `None` |
//! | `AddBody` / `DeleteBody` / `RenameBody` / `SetVisibility` | — | `None` |
//! | `AddDatumPlane` / `DeleteDatum` | — | `None` |
//! | `SetVariable` / `AddVariable` / `RemoveVariable` / `RenameVariable` | `[0, len)` (conservative) | `ToEnd` / `None` |
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
//! immutability (with the ONE sanctioned Fillet⇄Chamfer exception — see
//! [`op_type_edit_allowed`]), fillet/chamfer lockstep, and anti-time-travel
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
use crate::document::modules::{ModuleId, ModuleState};
use crate::document::record::{KnownOperation, Operation, OperationRecord};
use crate::document::refs::{ElementKind, ElementRef};
use crate::document::repair::{LadderLevel, RepairItem, RepairReason};
use crate::document::variables::{Scalar, Variable, VariableTable};
use crate::document::Document;
use crate::error::DomainError;
use crate::history::{DependencyGraph, DirtyRange, Timeline};
use crate::ids::{BodyId, ElementId, RecordId, SketchId};
use crate::math::Vec2;
use crate::sketch::{Constraint, Sketch, SketchAttachment, SketchEntity, SketchError};

use super::command::{EditCommand, InputPath, InputRef, SketchEditOp, VisibilityTarget};
use super::outcome::{CommandOutcome, ProjectionDelta, RegenHint};
use super::undo::{
    fold_floor, insert_record_at, replace_record_at, AppliedEdit, Inverse, Txn, UndoOutcome,
    UndoStack,
};

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
        // A command that mementoed NOTHING changed nothing, so it must not occupy an
        // undo slot — undoing it would pop a step the user never took and (with a
        // bounded stack) evict a real one.
        let undoable = !matches!(edit.inverse, Inverse::Noop);
        match &mut self.open_txn {
            Some(txn) => {
                if undoable {
                    txn.edits.push(edit);
                }
                merge_outcome(&mut txn.combined, &outcome);
            }
            None if undoable => {
                let label = edit.forward.label().to_string();
                self.undo.push_committed(Txn::single(label, edit));
            }
            None => {}
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
    ///   (unlock the prior projected geometry so the remove is legal, remove every
    ///   prior entity — cascading its constraints — then re-add the final
    ///   entities and constraints in order and re-issue every final provenance
    ///   row), reconstructing the final sketch exactly. A full replace via
    ///   existing variants is chosen over a minimal diff because it guarantees an
    ///   order-exact redo without fragile per-field diffing (the brief's blessed
    ///   "remove-all + re-add" alternative). See [`squash_redo_ops`].
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
        // The provenance map is part of the net result: a session that only
        // projected, updated or detached geometry leaves entities and constraints
        // in place and changes nothing BUT `projections`, and calling that
        // net-zero would drop the session from the undo stack entirely (WP-P F5).
        let net_zero = final_sketch.entities() == prior.entities()
            && final_sketch.constraints() == prior.constraints()
            && final_sketch.projections == prior.projections;
        if net_zero {
            self.undo.squash(count, None);
            return;
        }
        let ops = squash_redo_ops(&prior, final_sketch);
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

    /// Syncs reseated `PlaceComponent` placements from the regen scratch's
    /// own timeline into the document's authoritative one — a DERIVED,
    /// no-undo writeback (Component Library P3 WP-3.1, spec §5.5), the same
    /// treatment `sync_record_outputs` gives `outputs`. Matched by
    /// `RecordId`, never a raw index carried across the two timelines: the
    /// regen scratch's timeline and the document's can differ in
    /// length/order across a checkpoint-accelerated regen, so only an
    /// identity-keyed lookup is safe. Only records in `executed` are
    /// considered (same "this regen's own truth only" rule
    /// `sync_record_outputs` documents); `set_place_component_placement`
    /// itself no-ops on an unchanged or non-`PlaceComponent` target.
    pub fn sync_mate_placements(
        &mut self,
        regen_timeline: &Timeline,
        executed: &BTreeSet<RecordId>,
    ) -> bool {
        let mut changed = false;
        for &id in executed {
            let Some(regen_rec) = regen_timeline.record_by_id(id) else {
                continue;
            };
            let Operation::Known(KnownOperation::PlaceComponent(regen_params)) = &regen_rec.op
            else {
                continue;
            };
            if regen_params.mate.is_none() {
                continue;
            }
            let Some(index) = self.document.timeline.index_of(id) else {
                continue;
            };
            changed |= self
                .document
                .timeline
                .set_place_component_placement(index, regen_params.placement.clone());
        }
        changed
    }

    /// Syncs the RESOLVED value of every expression-driven [`Scalar`] from the
    /// regen mirror's effective records onto the document's authoritative ones
    /// (WP-VE.1) — a DERIVED, no-undo writeback, right beside
    /// [`sync_mate_placements`](Self::sync_mate_placements) and matched by
    /// `RecordId` for the same reason.
    ///
    /// A `Scalar`'s `value` under an `expr` is documented as the last EVALUATED
    /// value, so a regen that just evaluated it is the moment to store it: a
    /// document saved afterwards carries current numbers instead of whatever the
    /// record was last hand-edited with. `expr` itself is never touched, so the
    /// binding survives, and the number written is exactly the one the plan was
    /// hashed and built with — re-substituting it is a no-op, so no hash moves
    /// and no checkpoint is invalidated by the writeback itself.
    ///
    /// [`Scalar`]: crate::document::variables::Scalar
    pub fn sync_variable_values(
        &mut self,
        regen_timeline: &Timeline,
        executed: &BTreeSet<RecordId>,
    ) -> bool {
        let mut changed = false;
        for &id in executed {
            let Some(regen_rec) = regen_timeline.record_by_id(id) else {
                continue;
            };
            let Some(index) = self.document.timeline.index_of(id) else {
                continue;
            };
            changed |= self
                .document
                .timeline
                .sync_resolved_scalar_values(index, &regen_rec.op);
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
            if self.document.bodies.contains(meta.id) {
                // Insert-only for USER-AUTHORED metadata (name/visible), but the
                // VF-B6 identity stamp is DERIVED: leaving a first-adoption stamp on
                // a row the mirror has since re-published would make the in-memory
                // registry disagree with what a save writes (a save takes bodies
                // from the mirror). Refresh it in place.
                if let Some(key) = meta.geom_stamp {
                    self.document.bodies.set_geom_stamp(meta.id, key);
                }
                continue;
            }
            adopted |= self.document.bodies.register(meta.clone());
        }
        adopted
    }

    /// Copies body-level display metadata onto a newly minted derived child.
    /// Face colors stay source-local because their ElementIds do not cross bodies.
    pub fn inherit_body_display_metadata(&mut self, source: BodyId, child: BodyId) -> bool {
        let Some(source_meta) = self.document.bodies.get(source) else {
            return false;
        };
        let visible = source_meta.visible;
        let color = source_meta.color;
        self.document.bodies.set_visible(child, visible)
            && self.document.bodies.set_color(child, color)
    }

    /// **Regen-lane** repair-gate seeding (VF-B6) — plants `items` as seeded gates
    /// with NO command inverse.
    ///
    /// Deliberately outside the VF-M8 single-snapshot rule that governs every EDIT
    /// command's repair mutation: `commit_snapshot` is not an [`EditCommand`], so
    /// there is no transaction for a `RestoreRepair` to ride. These gates are
    /// **self-healing state derived from geometry stamps** instead — each carries the
    /// [`OrdinalAnchor`](crate::document::repair::OrdinalAnchor) it was raised
    /// against, so undoing the offending edit re-runs regen, the ordering matches the
    /// anchor again, and [`clear_ordinal_gates`](Self::clear_ordinal_gates) drops
    /// them. An undo therefore does not need to (and must not) restore them by
    /// inverse: it restores the *geometry*, and the gate follows.
    pub fn seed_regen_repair_gates(&mut self, items: Vec<RepairItem>) {
        self.document.repair.seed(items);
    }

    /// **Regen-lane** clear of the VF-B6 ordinal gates anchored on `op` (the
    /// tripwire's self-heal). Returns `true` if anything was dropped. Inverse-free
    /// for the same reason as [`seed_regen_repair_gates`](Self::seed_regen_repair_gates).
    pub fn clear_ordinal_gates(&mut self, op: RecordId) -> bool {
        self.document.repair.clear_ordinal_gates(op)
    }

    /// Undoes the newest committed transaction (applies its inverses in reverse
    /// and moves it to the redo stack). Ignored while a transaction is open
    /// (C++ `CommandProcessor::undo`). `None` if there was nothing to undo.
    ///
    /// The returned [`UndoOutcome`] carries the **folded dirty floor** (H8): each
    /// memento's [`Inverse::dirty_floor`] is read BEFORE that memento is applied —
    /// `RemoveRecord` resolves its index against the timeline it is about to remove
    /// from — and the lowest wins. The caller replays from there instead of from 0.
    pub fn undo(&mut self) -> Option<UndoOutcome> {
        if self.open_txn.is_some() {
            return None;
        }
        let txn = self.undo.pop_for_undo()?;
        let mut dirty_floor: Option<usize> = None;
        for edit in txn.edits.iter().rev() {
            dirty_floor = fold_floor(dirty_floor, edit.inverse.dirty_floor(&self.document));
            edit.inverse.clone().apply(&mut self.document);
        }
        self.rebuild_graph();
        self.undo.push_undone(txn);
        Some(UndoOutcome { dirty_floor })
    }

    /// Redoes the newest undone transaction by **re-executing** its forward
    /// commands (recomputing fresh inverses). Ignored while a transaction is
    /// open. `None` if there was nothing to redo.
    ///
    /// The re-executed commands' [`CommandOutcome`]s were previously discarded; their
    /// dirty spans are now folded into the returned [`UndoOutcome`] (H8), so a redo
    /// replays from the same floor the original commit did.
    ///
    /// # Errors
    /// A [`DomainError`] if a replayed command fails (the partial replay is
    /// rolled back and the step returned to the redo stack).
    pub fn redo(&mut self) -> Result<Option<UndoOutcome>, DomainError> {
        if self.open_txn.is_some() {
            return Ok(None);
        }
        let Some(txn) = self.undo.pop_for_redo() else {
            return Ok(None);
        };
        let mut new_edits: Vec<AppliedEdit> = Vec::with_capacity(txn.edits.len());
        let mut dirty_floor: Option<usize> = None;
        for edit in &txn.edits {
            match self.apply_forward(&edit.forward) {
                Ok((outcome, inverse)) => {
                    dirty_floor = fold_floor(dirty_floor, outcome.dirty.map(|d| d.from));
                    new_edits.push(AppliedEdit {
                        forward: edit.forward.clone(),
                        inverse,
                    });
                }
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
        Ok(Some(UndoOutcome { dirty_floor }))
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
            EditCommand::SetModuleState { module, state } => {
                self.set_module_state(module.clone(), state.clone())
            }
            EditCommand::SetBodyColor { body, color } => self.set_body_color(*body, *color),
            EditCommand::SetFaceColor {
                body,
                element_id,
                color,
            } => self.set_face_color(*body, element_id.clone(), *color),
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
            EditCommand::RenameVariable { variable, name } => self.rename_variable(*variable, name),
        }
    }

    // ── Timeline commands ────────────────────────────────────────────────────

    fn add_operation(
        &mut self,
        mut record: OperationRecord,
        at_cursor: bool,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        // VF-M8 single-snapshot rule (see `fold_repair_inverse`).
        let prior_repair = self.document.repair.clone();
        let id = record.record_id;
        if self.document.timeline.record_by_id(id).is_some() {
            return Err(DomainError::Validation(format!("duplicate record id {id}")));
        }
        // F2: fillet/chamfer `edges`/`edge_ids` must be in lockstep (all entry paths).
        validate_fillet_lockstep(&record.op)?;
        // Shell typed refs preserve evidence and must mirror `openFaces`.
        validate_shell_lockstep(&record.op)?;
        // SCHEMA §7.3: `distance2`/`angleDeg` are Chamfer-only, in range, and
        // mutually exclusive (all entry paths).
        validate_edge_op_distances(&record.op)?;
        // ImportStep params must name a real content-addressed blob (all entry paths).
        validate_import_step(&record.op)?;
        // TransformBody params must be a well-formed rigid motion (all entry paths).
        validate_transform_body(&record.op)?;
        // Hole params must satisfy the SCHEMA §7.3 conditional blocks (all paths).
        validate_hole(&record.op)?;
        // Gear params must satisfy the SCHEMA §7.3 recipe-conditional blocks and
        // the exactly-one-of face/frame placement rule (all entry paths).
        validate_gear(&record.op)?;
        // OffsetFace params must satisfy the SCHEMA §7.3 lockstep + distance-type
        // invariants (all entry paths).
        validate_offset_face(&record.op)?;
        // An Extrude/Revolve `regionAnchor`, when present, must be two finite
        // numbers (all entry paths).
        validate_region_anchor(&record.op)?;
        // PlaceComponent params must be structurally well-formed (all entry paths).
        validate_place_component(&record.op)?;
        // DetachComponent params must be structurally well-formed (all entry paths).
        validate_detach_component(&record.op)?;
        // Every `Scalar` expression must evaluate against the CURRENT variable
        // table (all entry paths) — a record that could never resolve is refused
        // here rather than written and then discovered at the next regen. An
        // insert has no prior, so every expression on it is new.
        validate_expressions(&record.op, None, &self.document.variables)?;
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
        // VF-B5c: a record inserted at/below a seeded gate pushes that gate one step
        // down the timeline. Without this the positional `step_index` would keep
        // naming the OLD position — truncating regen at an innocent step while the
        // gated one runs free. A frontier append (`index == len − 1`) shifts nothing.
        self.document.repair.shift_seeded_for_insert(index);

        let new_len = self.document.timeline.len();
        let dirty = DirtyRange::new(index, new_len);
        let regen = if index < self.document.timeline.cursor() {
            RegenHint::ToEnd
        } else {
            RegenHint::None
        };
        let mut delta = ProjectionDelta::timeline();
        delta.cursor_changed = at_cursor || frontier_append;
        let inverse = self.fold_repair_inverse(prior_repair, Inverse::RemoveRecord { record: id });
        Ok((
            CommandOutcome {
                projection_delta: delta,
                dirty: Some(dirty),
                regen,
            },
            inverse,
        ))
    }

    fn update_operation_params(
        &mut self,
        id: RecordId,
        op: Operation,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        // VF-M8 single-snapshot rule: capture the repair state ONCE, at command
        // entry, before anything can mutate it.
        let prior_repair = self.document.repair.clone();
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        let prior = self.document.timeline.record(index).unwrap().clone();
        if let Err(reason) = op_type_edit_allowed(&prior.op, &op) {
            return Err(DomainError::Validation(reason.into()));
        }
        let type_changed = !same_op_type(&prior.op, &op);
        // F2: fillet/chamfer `edges`/`edge_ids` must be in lockstep (all entry paths).
        validate_fillet_lockstep(&op)?;
        // Shell typed refs preserve evidence and must mirror `openFaces`.
        validate_shell_lockstep(&op)?;
        // SCHEMA §7.3: `distance2`/`angleDeg` are Chamfer-only, in range, and
        // mutually exclusive (all entry paths).
        validate_edge_op_distances(&op)?;
        // ImportStep params must name a real content-addressed blob (all entry paths).
        validate_import_step(&op)?;
        // TransformBody params must be a well-formed rigid motion (all entry paths).
        validate_transform_body(&op)?;
        // Hole params must satisfy the SCHEMA §7.3 conditional blocks (all paths).
        validate_hole(&op)?;
        // Gear params must satisfy the SCHEMA §7.3 recipe-conditional blocks and
        // the exactly-one-of face/frame placement rule (all entry paths).
        validate_gear(&op)?;
        // OffsetFace params must satisfy the SCHEMA §7.3 lockstep + distance-type
        // invariants (all entry paths).
        validate_offset_face(&op)?;
        // An Extrude/Revolve `regionAnchor`, when present, must be two finite
        // numbers (all entry paths).
        validate_region_anchor(&op)?;
        // PlaceComponent params must be structurally well-formed (all entry paths).
        validate_place_component(&op)?;
        // DetachComponent params must be structurally well-formed (all entry paths).
        validate_detach_component(&op)?;
        // Every `Scalar` expression this edit INTRODUCES or CHANGES must
        // evaluate against the CURRENT variable table (all entry paths). Scoped
        // to the delta on purpose: a record whose binding was broken by a LATER
        // variable edit must stay editable on its other fields — see
        // `validate_op_expressions`.
        validate_expressions(&op, Some(&prior.op), &self.document.variables)?;
        let mut nr = prior.clone();
        nr.op = op;
        // A sanctioned Fillet⇄Chamfer swap is the only path that can strand the
        // legacy `mode` string on the wrong opType; the session is the single
        // writer, so the contradiction is corrected HERE rather than trusted to
        // whichever caller happened to build the payload.
        if type_changed {
            normalize_edge_op_mode(&mut nr.op, &prior.op);
        }
        nr.inputs = nr.op.derive_inputs();

        let mut recs = self.document.timeline.records().to_vec();
        recs[index] = nr.clone();
        self.validate_temporal(&recs, id, &nr.op.derive_inputs().bodies)?;

        replace_record_at(&mut self.document.timeline, index, nr);
        self.rebuild_graph();
        // SCHEMA §7.3 edit-safety gate: a params edit to a TransformBody moves the
        // geometry every downstream ref was frozen against, so those refs must
        // re-resolve through the REPAIR flow, never by silent descriptor scoring.
        // The seed rides THIS command's inverse (composite) so undo restores the
        // previous binding state exactly.
        self.seed_transform_gate(index);
        let inverse = self.fold_repair_inverse(
            prior_repair,
            Inverse::RestoreRecord {
                index,
                record: Box::new(prior),
            },
        );
        Ok(self.dirty_record_outcome(index, inverse))
    }

    /// Seeds the SCHEMA §7.3 `TransformBody` edit-safety gate for the record at
    /// `index`. **Snapshot-free** — the calling command owns the single
    /// command-entry repair snapshot and folds it in with
    /// [`fold_repair_inverse`](Self::fold_repair_inverse) (VF-M8).
    ///
    /// A no-op when the record is not a `TransformBody` or when nothing downstream
    /// stands on its target lineage.
    fn seed_transform_gate(&mut self, index: usize) {
        let items = transform_gate_items(&self.document, index);
        if items.is_empty() {
            return;
        }
        self.document.repair.seed(items);
    }

    /// Folds **exactly one** [`Inverse::RestoreRepair`] carrying the
    /// command-entry repair state into `inverse` — the single-snapshot repair rule
    /// (VF-M8).
    ///
    /// **Ordering is the whole point.** [`Inverse::Composite`] applies its members
    /// in REVERSE ([`undo`](super::undo)), so the restore must sit FIRST to be
    /// applied LAST — otherwise a later sub-inverse could re-plant repair state the
    /// undo just removed. The composite is kept FLAT (a `Composite` argument is
    /// spliced, not nested) so no command can ever accumulate two `RestoreRepair`s
    /// whose reverse order then decides which snapshot wins: before this rule, a
    /// cascade (un)suppression across two `TransformBody` records wrapped one
    /// snapshot per transform and the LAST-taken (already-seeded) one won, leaving
    /// the first transform's gate alive after undo.
    ///
    /// Returns `inverse` untouched when the command did not move the repair state.
    fn fold_repair_inverse(
        &self,
        prior: crate::document::repair::RepairState,
        inverse: Inverse,
    ) -> Inverse {
        if prior == self.document.repair {
            return inverse;
        }
        let restore = Inverse::RestoreRepair {
            state: Box::new(prior),
        };
        match inverse {
            Inverse::Composite(list) => {
                let mut flat = Vec::with_capacity(list.len() + 1);
                flat.push(restore);
                flat.extend(list);
                Inverse::Composite(flat)
            }
            other => Inverse::Composite(vec![restore, other]),
        }
    }

    fn edit_operation_input(
        &mut self,
        id: RecordId,
        path: &InputPath,
        reference: &InputRef,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        // VF-M8 single-snapshot rule (see `fold_repair_inverse`).
        let prior_repair = self.document.repair.clone();
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        let prior = self.document.timeline.record(index).unwrap().clone();
        let mut nr = prior.clone();
        set_input(&mut nr.op, path, reference)?;
        validate_shell_lockstep(&nr.op)?;
        nr.inputs = nr.op.derive_inputs();

        let mut recs = self.document.timeline.records().to_vec();
        recs[index] = nr.clone();
        self.validate_temporal(&recs, id, &nr.op.derive_inputs().bodies)?;

        replace_record_at(&mut self.document.timeline, index, nr);
        self.rebuild_graph();
        // `EditOperationInput` IS the repair flow (the user re-picked the ref), so
        // it CLOSES any seeded gate on this step. Ladder-produced items for the
        // step are left alone — the next regen republishes them.
        self.document.repair.clear_seeded_for_step(index);
        // A transform whose INPUT changed moves a different body set — same gate.
        self.seed_transform_gate(index);
        let inverse = self.fold_repair_inverse(
            prior_repair,
            Inverse::RestoreRecord {
                index,
                record: Box::new(prior),
            },
        );
        Ok(self.dirty_record_outcome(index, inverse))
    }

    fn remove_operation(&mut self, id: RecordId) -> Result<(CommandOutcome, Inverse), DomainError> {
        // VF-M8 single-snapshot rule (see `fold_repair_inverse`).
        let prior_repair = self.document.repair.clone();
        let index = self
            .document
            .timeline
            .index_of(id)
            .ok_or(DomainError::RecordNotFound(id))?;
        let record = self.document.timeline.record(index).unwrap().clone();
        // VF-B5b: the moved set must be read BEFORE the record leaves the timeline.
        let moved = transform_moved_bodies(&record);
        let cursor = self.document.timeline.cursor();
        let dirty = self.document.timeline.remove(id)?;
        self.rebuild_graph();
        // VF-B5c: `step_index` is positional — every gate above the hole shifts up
        // by one, and the removed step's own gates die with the record they guarded
        // (nothing could ever close them, and the ceiling would pin regen forever).
        self.document.repair.shift_seeded_for_remove(index);
        // VF-B5b: DELETING a `TransformBody` moves downstream geometry exactly like
        // suppressing it (the placement disappears), so it seeds the SAME gate —
        // otherwise the one edit that removes a placement outright is the one edit
        // that re-binds every downstream ref silently. The scan starts AT `index`
        // because the timeline has already closed up over the deleted record, so
        // the emitted `step_index`es address the POST-removal timeline.
        let label = format!(
            "reference re-bind blocked by a deleted Move (was step {index}); repair to re-pick"
        );
        let items = gate_items_for_moved(&self.document, &moved, index, &label);
        self.document.repair.seed(items);
        let mut delta = ProjectionDelta::timeline();
        delta.cursor_changed = index < cursor;
        let inverse = self.fold_repair_inverse(
            prior_repair,
            Inverse::InsertRecord {
                index,
                record: Box::new(record),
                cursor,
            },
        );
        Ok((
            CommandOutcome {
                projection_delta: delta,
                dirty: Some(dirty),
                regen: RegenHint::ToEnd,
            },
            inverse,
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
        // Rolling to the row the bar already sits on changes nothing, so it must cost
        // nothing: no dirty span (checkpoints keep their validity), no regen, and no
        // undo entry (`Inverse::Noop` is dropped at the `apply` funnel).
        if cursor == prior {
            return Ok((empty_outcome(), Inverse::Noop));
        }
        let dirty = self.document.timeline.set_cursor(cursor);
        let delta = ProjectionDelta {
            cursor_changed: true,
            ..ProjectionDelta::default()
        };
        Ok((
            CommandOutcome {
                projection_delta: delta,
                dirty: Some(dirty),
                // DOMAIN SPLIT: `cursor` is an applied-op COUNT, while `PreviewTo` (and
                // the `RegenRequest::ToStep` it maps to) takes a step INDEX. The last
                // applied step is therefore `cursor − 1`; emitting the count made every
                // non-zero rollback request a step past the applied end, which the
                // planner answers with an empty plan — a visually inert roll. Cursor 0
                // saturates to index 0 and the planner's `applied == 0` branch turns it
                // into the clear publish.
                regen: RegenHint::PreviewTo(cursor.saturating_sub(1)),
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
        // VF-M8 single-snapshot rule (see `fold_repair_inverse`).
        let prior_repair = self.document.repair.clone();
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

        // SCHEMA §7.3 edit-safety gate: suppressing OR un-suppressing a
        // `TransformBody` moves downstream geometry exactly like a params edit
        // (the placement appears / disappears), so it seeds the same gate. Applied
        // for every affected record so a cascade that (un)suppresses several
        // transforms gates all of them — under ONE command-entry snapshot
        // (VF-M8: the per-transform snapshots this loop used to take nested, and
        // undo's reverse application then restored the LAST one, which already
        // carried the first transform's seeds).
        for &rid in &affected {
            if let Some(i) = self.document.timeline.index_of(rid) {
                self.seed_transform_gate(i);
            }
        }
        let inverse = self.fold_repair_inverse(prior_repair, Inverse::Composite(inverses));

        let dirty = DirtyRange::new(index, self.document.timeline.len());
        Ok((
            CommandOutcome {
                projection_delta: ProjectionDelta::timeline(),
                dirty: Some(dirty),
                regen: RegenHint::ToEnd,
            },
            inverse,
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
        // VF-M8 single-snapshot rule (see `fold_repair_inverse`).
        let prior_repair = self.document.repair.clone();
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
        let host_face = next.host_face().cloned();
        self.document.sketches.insert(id, next);

        // VF-B5a: re-picking the host must RESTAMP the record's `host_face` (and
        // re-derive its inputs), else the dependency edge — and with it the
        // TransformBody gate — keeps naming the OLD face forever. Re-picking a host
        // IS the repair action for that step, so it also closes the step's seeded
        // gate (mirrors `edit_operation_input`).
        let mut inverse = Inverse::RestoreSketch {
            id,
            prior: Some(Box::new(prior)),
        };
        let (next_plane, next_attachment) = {
            let s = &self.document.sketches[&id];
            (s.plane, s.attachment.clone())
        };
        if let Some((index, prior_record)) =
            self.restamp_sketch_record(id, next_plane, &next_attachment, host_face)
        {
            self.rebuild_graph();
            self.document.repair.clear_seeded_for_step(index);
            inverse = Inverse::Composite(vec![
                inverse,
                Inverse::RestoreRecord {
                    index,
                    record: Box::new(prior_record),
                },
            ]);
        }
        let inverse = self.fold_repair_inverse(prior_repair, inverse);
        Ok((self.sketch_dirty_outcome(id), inverse))
    }

    /// Rewrites the `plane` and `host_face` params (and the derived `inputs`) of
    /// the `Sketch` record producing `id`. Returns `(index, prior record)` when a
    /// record was actually changed, `None` otherwise.
    ///
    /// **Why the plane too (H9 reattach).** The regen planner lowers the RECORD's
    /// `SketchOpParams`, not `document.sketches[id]` — the record is otherwise
    /// refreshed only when a sketch session finishes (`upsert_sketch_record`). So
    /// an attachment change that moved the sketch's frame but left the record's
    /// `plane` alone would update the tree and the on-screen sketch while every
    /// regen kept replaying the sketch on its OLD plane, and no downstream feature
    /// would follow. `plane_ref_of` mirrors the app layer's own helper of the same
    /// name, so a record restamped here is identical to one minted at finish time.
    ///
    /// **Anti-time-travel guard.** The HOST-FACE stamp is SKIPPED when the host
    /// body is not produced strictly before this record (`produces_before`) — a
    /// legacy front-inserted sketch record (`backfill_missing_sketch_records`) can
    /// sit ahead of the body it is glued to, and stamping there would author an
    /// edge the timeline cannot honour. Such a record is not downstream of any
    /// transform on that body anyway, so nothing is left ungated. The PLANE stamp
    /// is not gated: a frame is not a dependency edge.
    fn restamp_sketch_record(
        &mut self,
        id: SketchId,
        plane: crate::sketch::SketchPlane,
        attachment: &SketchAttachment,
        host_face: Option<ElementRef>,
    ) -> Option<(usize, OperationRecord)> {
        let (index, prior_record) = self
            .document
            .timeline
            .records()
            .iter()
            .enumerate()
            .find_map(|(i, r)| match &r.op {
                Operation::Known(KnownOperation::Sketch(p)) if p.sketch == id => {
                    Some((i, r.clone()))
                }
                _ => None,
            })?;
        let host_body = host_face
            .as_ref()
            .and_then(|f| f.primary.as_ref())
            .map(|p| p.body);
        let host_face = match host_body {
            Some(b) if !self.graph.produces_before(b, prior_record.record_id) => None,
            _ => host_face,
        };
        let plane_ref = plane_ref_of(plane, attachment);
        let mut next = prior_record.clone();
        let Operation::Known(KnownOperation::Sketch(p)) = &mut next.op else {
            return None;
        };
        if p.host_face == host_face && p.plane == plane_ref {
            return None;
        }
        p.host_face = host_face;
        p.plane = plane_ref;
        next.inputs = next.op.derive_inputs();
        replace_record_at(&mut self.document.timeline, index, next);
        Some((index, prior_record))
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

    /// Writes or clears one module's slice. The payload is stored VERBATIM: the
    /// platform has no schema for it and must not normalize what it cannot read
    /// (ADR-0005).
    fn set_module_state(
        &mut self,
        module: ModuleId,
        state: Option<ModuleState>,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let prior = self.document.modules.get(&module).cloned();
        match state {
            Some(next) => {
                self.document.modules.insert(module.clone(), next);
            }
            None => {
                self.document.modules.remove(&module);
            }
        }
        Ok((
            // No projection changes: module state is not part of the modeling
            // projection, and no timeline step can consume it.
            CommandOutcome::metadata_only(ProjectionDelta::new()),
            Inverse::RestoreModuleState {
                module,
                prior: prior.map(Box::new),
            },
        ))
    }

    fn set_body_color(
        &mut self,
        id: BodyId,
        color: Option<[u8; 4]>,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.bodies.contains(id) {
            return Err(DomainError::Validation(format!("body {id} not found")));
        }
        let prior = self.document.bodies.clone();
        self.document.bodies.set_color(id, color);
        Ok((
            CommandOutcome::metadata_only(ProjectionDelta::body(id)),
            Inverse::RestoreBodies {
                registry: Box::new(prior),
            },
        ))
    }

    fn set_face_color(
        &mut self,
        id: BodyId,
        face: ElementId,
        color: Option<[u8; 4]>,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        if !self.document.bodies.contains(id) {
            return Err(DomainError::Validation(format!("body {id} not found")));
        }
        let prior = self.document.bodies.clone();
        self.document.bodies.set_face_color(id, face, color);
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
        let updated = Variable { value, ..existing };
        // Validate on the table this edit WOULD produce, so a cycle the edit
        // closes (`a = b`, `b = a`) is refused instead of written and then
        // discovered by every consumer at once.
        let mut candidate = prior.clone();
        candidate.upsert(updated.clone());
        validate_variable_binding(&candidate, &updated.name)?;
        self.document.variables.upsert(updated);
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
        let mut candidate = prior.clone();
        candidate.upsert(var.clone());
        validate_variable_binding(&candidate, &var.name)?;
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

    /// Renames a variable AND every expression that references it, in one
    /// undoable step.
    ///
    /// Refused as a whole — nothing written — when the new name is illegal, is
    /// already taken, or when ANY expression that would have to be rewritten
    /// cannot be parsed. A partial rename is a document whose numbers silently
    /// stop tracking their variable, which is the failure mode this whole pass
    /// exists to remove.
    fn rename_variable(
        &mut self,
        var_id: crate::ids::VariableId,
        new_name: &str,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        let old_name = self
            .document
            .variables
            .iter()
            .find(|v| v.id == var_id)
            .map(|v| v.name.clone())
            .ok_or_else(|| DomainError::Validation(format!("variable {var_id} not found")))?;
        if new_name == old_name {
            return Ok((
                CommandOutcome::metadata_only(ProjectionDelta::default()),
                Inverse::Noop,
            ));
        }
        if !crate::regen::is_bare_name(new_name) {
            return Err(DomainError::Validation(format!(
                "invalid variable name {new_name:?}: a name must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
        if self.document.variables.get(new_name).is_some() {
            return Err(DomainError::Validation(format!(
                "duplicate variable name {new_name}"
            )));
        }

        // 1. The rewritten variable table (renamed entry + every reference).
        let prior_vars = self.document.variables.clone();
        let mut next_vars = VariableTable::new();
        for var in prior_vars.iter() {
            let mut next = var.clone();
            if next.id == var_id {
                next.name = new_name.to_string();
            }
            if let Some(text) = next.value.expr.as_deref() {
                if let Some(rewritten) = rename_in(
                    text,
                    &old_name,
                    new_name,
                    &format!("variable `{}`", var.name),
                )? {
                    next.value.expr = Some(rewritten);
                }
            }
            next_vars.upsert(next);
        }

        // 2. The rewritten records. Collect the inverse per CHANGED record only,
        //    so an undo restores exactly what the rename touched.
        let mut records = self.document.timeline.records().to_vec();
        let mut restores: Vec<Inverse> = Vec::new();
        for (index, record) in records.iter_mut().enumerate() {
            let prior = record.clone();
            let Operation::Known(known) = &mut record.op else {
                continue;
            };
            let mut changed = false;
            for (field, _, scalar) in known.scalars_mut() {
                let Some(text) = scalar.expr.as_deref() else {
                    continue;
                };
                if let Some(rewritten) = rename_in(text, &old_name, new_name, field)? {
                    scalar.expr = Some(rewritten);
                    changed = true;
                }
            }
            if changed {
                restores.push(Inverse::RestoreRecord {
                    index,
                    record: Box::new(prior),
                });
            }
        }

        // 3. Commit both halves together (every refusal above happened first).
        self.document.variables = next_vars;
        let cursor = self.document.timeline.cursor();
        self.document.timeline = Timeline::from_records(records);
        self.document.timeline.set_cursor(cursor);
        self.rebuild_graph();

        let mut outcome = self.variable_outcome();
        outcome.projection_delta.timeline_changed = true;
        restores.push(Inverse::RestoreVariables {
            table: Box::new(prior_vars),
        });
        Ok((outcome, Inverse::Composite(restores)))
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
    /// The earliest step is [`Document::sketch_dirty_step`] — `min(producer, first
    /// consumer)` (F4). Shared with the undo lane
    /// ([`Inverse::dirty_floor`](super::undo::Inverse::dirty_floor)) so an edit and
    /// its undo agree on which step a sketch change dirties.
    fn sketch_dirty_outcome(&self, id: SketchId) -> CommandOutcome {
        let delta = ProjectionDelta::sketch(id);
        match self.document.sketch_dirty_step(id) {
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
}

// ── Free helpers ─────────────────────────────────────────────────────────────

pub fn empty_outcome() -> CommandOutcome {
    CommandOutcome {
        projection_delta: ProjectionDelta::default(),
        dirty: None,
        regen: RegenHint::None,
    }
}

/// Merges `o` into `acc` (transaction batching).
pub fn merge_outcome(acc: &mut CommandOutcome, o: &CommandOutcome) {
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
/// and every typed ref must carry the matching EDGE primary on one operated body.
/// Empty `edges` remains the only legacy form; a mixed typed/intent-only list has
/// no unambiguous owner and must not reach the worker's descriptor fallback.
/// Non-fillet/chamfer ops and opaque ops are trivially valid.
fn validate_fillet_lockstep(op: &Operation) -> Result<(), DomainError> {
    let (edge_ids, edges, closure_version) = match op {
        Operation::Known(KnownOperation::Fillet(p)) => {
            (&p.edge_ids, &p.edges, p.tangent_closure_version)
        }
        Operation::Known(KnownOperation::Chamfer(p)) => {
            (&p.edge_ids, &p.edges, p.tangent_closure_version)
        }
        _ => return Ok(()),
    };
    if edges.is_empty() {
        if closure_version == Some(1) {
            return Err(DomainError::Validation(
                "version-1 tangent closure requires full typed edge refs".into(),
            ));
        }
        return Ok(());
    }
    if edges.len() != edge_ids.len() {
        return Err(DomainError::Validation(format!(
            "fillet/chamfer edges ({}) and edgeIds ({}) length mismatch",
            edges.len(),
            edge_ids.len()
        )));
    }
    let mut target_body = None;
    for (i, e) in edges.iter().enumerate() {
        let primary = e.primary.as_ref().ok_or_else(|| {
            DomainError::Validation(format!(
                "fillet/chamfer edge {i}: typed ref requires an EDGE primary"
            ))
        })?;
        if primary.kind != ElementKind::Edge {
            return Err(DomainError::Validation(format!(
                "fillet/chamfer edge {i}: typed ref requires an EDGE primary"
            )));
        }
        if primary.element != edge_ids[i] {
            return Err(DomainError::Validation(format!(
                "fillet/chamfer edge {i}: typed ref element {} != edgeIds[{i}] {}",
                primary.element, edge_ids[i]
            )));
        }
        if let Some(target) = target_body {
            if primary.body != target {
                return Err(DomainError::Validation(format!(
                    "fillet/chamfer edge {i}: typed ref body {} != operated body {target}",
                    primary.body
                )));
            }
        } else {
            target_body = Some(primary.body);
        }
    }
    Ok(())
}

/// Validates Shell's typed `faces` against the parallel bare `open_faces` list.
/// Empty typed refs remain readable for legacy records; once present, every slot
/// must carry the matching FACE primary so new writes cannot discard evidence.
fn validate_shell_lockstep(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::Shell(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates the SCHEMA §7.3 (2026-08-03) two-distance and distance-angle chamfer
/// rules on both edge ops:
///
/// * a **Chamfer**'s `distance2`, when present, must be a positive finite length,
///   its `angleDeg` must be strictly between 0 and 180 degrees, and the two may not
///   both be present ([`ChamferParams::validate`]);
/// * a **Fillet** must not carry `distance2` or `angleDeg` at all. `FilletParams`
///   has no typed field for either, so a payload that names one would otherwise land
///   in the `extra` flatten and round-trip VERBATIM — a second leg silently
///   persisted on an op the worker will never read it for, and (worse) resurrected
///   by a later Fillet→Chamfer flip as if the user had authored it.
///
/// Non-edge and opaque ops are trivially valid. Enforced here rather than at
/// deserialize time for the same single-writer reason as [`validate_import_step`]:
/// a document authored by another build still opens and round-trips.
fn validate_edge_op_distances(op: &Operation) -> Result<(), DomainError> {
    let closure_version = match op {
        Operation::Known(KnownOperation::Fillet(p)) => p.tangent_closure_version,
        Operation::Known(KnownOperation::Chamfer(p)) => p.tangent_closure_version,
        _ => None,
    };
    if closure_version.is_some_and(|version| version != 1) {
        return Err(DomainError::Validation(
            "tangentClosureVersion must be 1 when present".into(),
        ));
    }
    match op {
        Operation::Known(KnownOperation::Chamfer(p)) => {
            p.validate().map_err(DomainError::Validation)
        }
        Operation::Known(KnownOperation::Fillet(p)) if p.extra.contains_key("distance2") => {
            Err(DomainError::Validation(
                "distance2 is Chamfer-only (SCHEMA §7.3); a Fillet may not carry it".into(),
            ))
        }
        Operation::Known(KnownOperation::Fillet(p)) if p.extra.contains_key("angleDeg") => {
            Err(DomainError::Validation(
                "angleDeg is Chamfer-only (SCHEMA §7.3); a Fillet may not carry it".into(),
            ))
        }
        _ => Ok(()),
    }
}

/// Validates an [`KnownOperation::ImportStep`] record's params (sha256 shape,
/// unit scale, heal policy, codec/`brepFormat` agreement — see
/// [`ImportStepParams::validate`](crate::document::record::ImportStepParams::validate)).
/// Non-import and opaque ops are trivially valid.
///
/// Enforced HERE rather than at deserialize time, mirroring
/// [`validate_fillet_lockstep`]: the session is the single writer, so a bad
/// `sourceSha256` is rejected before it can reach `document.json` (where it would
/// be interpolated into a container entry name), while an existing document
/// authored by another build still opens and round-trips.
fn validate_import_step(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::ImportStep(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates a [`KnownOperation::TransformBody`] record's params (non-empty +
/// deduped targets, finite components, non-zero axis when the angle is non-zero —
/// see [`TransformBodyParams::validate`]). Non-transform and opaque ops are
/// trivially valid. Enforced here for the same single-writer reason as
/// [`validate_import_step`].
///
/// [`TransformBodyParams::validate`]: crate::document::record::TransformBodyParams::validate
fn validate_transform_body(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::TransformBody(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates a [`KnownOperation::Hole`] record's params (positive finite
/// diameter/depth, and the SCHEMA §7.3 conditional `cb*`/`cs*` blocks REQUIRED —
/// and only permitted — for their own `holeType`; see [`HoleParams::validate`]).
/// Non-hole and opaque ops are trivially valid. Enforced here for the same
/// single-writer reason as [`validate_import_step`].
///
/// This is the ONLY layer that can enforce "a simple hole carries no `cb*`": the
/// worker never sees a param it does not read, so a stale conditional would ride
/// the record forever and resurrect on the next profile switch.
///
/// [`HoleParams::validate`]: crate::document::record::HoleParams::validate
fn validate_hole(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::Hole(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates a [`KnownOperation::Gear`] record's params: the SCHEMA §7.3
/// recipe-conditional blocks (checked BOTH ways, so a stale block left by a
/// recipe switch cannot ride the record), the exactly-one-of face/frame
/// placement rule, and every dimension's domain. Non-gear and opaque ops are
/// trivially valid.
///
/// Enforced here for the same single-writer reason as [`validate_hole`]: the
/// worker never sees a param it does not read, so a stale inactive recipe block
/// would otherwise survive indefinitely and resurrect on the next recipe switch.
///
/// [`GearParams::validate`]: crate::document::record::GearParams::validate
fn validate_gear(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::Gear(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates a [`KnownOperation::OffsetFace`] record's params: the
/// `faceIds`/`faces` lockstep (the Fillet discipline of
/// [`validate_fillet_lockstep`], but MANDATORY here — an OffsetFace with no typed
/// refs has nothing for the ladder to rebind), a finite distance, and the SCHEMA
/// §7.3 distance-type conditionals (see
/// [`OffsetFaceParams::validate`](crate::document::record::OffsetFaceParams::validate)).
/// Non-offset and opaque ops are trivially valid.
///
/// Enforced here for the same single-writer reason as [`validate_hole`], and with
/// the same both-ways strictness: a `Total` opposite face stranded by a
/// distance-type switch would otherwise ride the record invisibly.
fn validate_offset_face(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::OffsetFace(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates the optional SCHEMA §7.3 `regionAnchor` an Extrude/Revolve profile
/// may carry: present ⇒ two FINITE numbers. Other and opaque ops are trivially
/// valid.
///
/// Enforced here for the same single-writer reason as [`validate_hole`]: a
/// malformed anchor decides what the op may bind to, so it is refused BY NAME at
/// the writer instead of reaching the worker, where it would surface as the
/// generic stale-`regionId` refusal and read as a different defect.
fn validate_region_anchor(op: &Operation) -> Result<(), DomainError> {
    match op {
        Operation::Known(KnownOperation::Extrude(p)) => {
            p.validate().map_err(DomainError::Validation)
        }
        Operation::Known(KnownOperation::Revolve(p)) => {
            p.validate().map_err(DomainError::Validation)
        }
        _ => Ok(()),
    }
}

/// Validates a [`KnownOperation::PlaceComponent`] record's params: a
/// namespaced `componentId`, a non-empty `componentVersion`, a well-formed
/// `sha256:`-prefixed `componentRevision`, a non-empty generator id, a
/// non-empty `mate.selfAttachment` when a mate is present, and a finite
/// placement (see [`PlaceComponentParams::validate`]). Non-component and
/// opaque ops are trivially valid. Enforced here for the same single-writer
/// reason as [`validate_import_step`].
///
/// **Structural only.** Whether a `params` key is actually declared `role:
/// free` on the component's signature cannot be checked here — `onecad-core`
/// must not depend on the library crate that resolves a component signature
/// (the same wall that keeps the kernel closed, ADR-0002's spirit applied to
/// the library). That enforcement lives at the app-crate authoring entry
/// point instead (Component Library WP-1.2/1.3).
///
/// [`PlaceComponentParams::validate`]: crate::document::record::PlaceComponentParams::validate
fn validate_place_component(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::PlaceComponent(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// Validates a [`KnownOperation::DetachComponent`] record's params: the same
/// `source`/`placement` self-consistency [`PlaceComponentParams`] requires
/// (see [`DetachComponentParams::validate`]). Non-detach and opaque ops are
/// trivially valid. Enforced here for the same single-writer reason as
/// [`validate_place_component`].
///
/// [`DetachComponentParams::validate`]: crate::document::record::DetachComponentParams::validate
/// [`PlaceComponentParams`]: crate::document::record::PlaceComponentParams
fn validate_detach_component(op: &Operation) -> Result<(), DomainError> {
    let Operation::Known(KnownOperation::DetachComponent(p)) = op else {
        return Ok(());
    };
    p.validate().map_err(DomainError::Validation)
}

/// [`crate::regen::rename_reference`] with the failure turned into a
/// `DomainError` that names WHERE the unparseable expression lives.
fn rename_in(text: &str, old: &str, new: &str, site: &str) -> Result<Option<String>, DomainError> {
    crate::regen::rename_reference(text, old, new).map_err(|reason| {
        DomainError::Validation(format!(
            "cannot rename `{old}`: {site} carries an expression that does not parse ({reason})"
        ))
    })
}

/// Refuses a variable edit whose own expression cannot resolve against the table
/// the edit WOULD produce — a bad expression, a missing reference, or a cycle
/// the edit closes.
///
/// Scoped to `name`: an edit is judged by whether IT resolves, not by whether it
/// broke someone else. A variable that becomes unresolvable because a DIFFERENT
/// one moved is later breakage, and later breakage is a diagnostic (the
/// projection's `VariableDto.error`), not a refusal of the edit in front of the
/// user.
fn validate_variable_binding(candidate: &VariableTable, name: &str) -> Result<(), DomainError> {
    let (_, errors) = crate::regen::resolve_variable_table(candidate);
    match errors.iter().find(|e| e.name == name) {
        Some(e) => Err(DomainError::Validation(format!(
            "variable `{name}`: {}",
            e.message
        ))),
        None => Ok(()),
    }
}

/// Validates the `Scalar` expressions this edit INTRODUCES or CHANGES against
/// `vars` — the EDIT-TIME half of the expression contract.
///
/// A newly written expression that cannot parse, names a missing or broken
/// variable, or produces the wrong dimension for its call site is refused HERE,
/// so nothing is recorded. The other half is later breakage: a variable deleted
/// or re-dimensioned AFTER the record was written cannot be refused
/// retroactively, and surfaces as a per-step `EXPR_UNRESOLVED` diagnostic plus a
/// `StepState::Error` at regen instead ([`crate::regen::variables`]).
///
/// `prior` is `None` for an insert. Scoping to the delta is what keeps a record
/// whose binding broke later EDITABLE on its other fields; see
/// [`crate::regen::validate_op_expressions`] for why that matters.
fn validate_expressions(
    op: &Operation,
    prior: Option<&Operation>,
    vars: &VariableTable,
) -> Result<(), DomainError> {
    crate::regen::validate_op_expressions(op, prior, vars).map_err(DomainError::Validation)
}

/// The bodies whose GEOMETRY a `TransformBody` record moves — the "target
/// lineage" the SCHEMA §7.3 edit-safety gate is scoped to.
///
/// * `copy: false` — the targets themselves (modify lineage preserves the BodyId,
///   so the targets' ids ARE the moved bodies for the whole downstream timeline;
///   this is why the conservative target-id set suffices for V1).
/// * `copy: true` — the sources are untouched; what moves are the minted copies,
///   which appear in the record's regen-derived `outputs`.
fn transform_moved_bodies(rec: &OperationRecord) -> Vec<BodyId> {
    let Operation::Known(KnownOperation::TransformBody(p)) = &rec.op else {
        return Vec::new();
    };
    if p.copy {
        rec.outputs.clone()
    } else {
        p.targets.clone()
    }
}

/// The §9 items the SCHEMA §7.3 edit-safety gate seeds when a `TransformBody` at
/// `index` is edited / (un)suppressed: one per downstream ref standing on a body
/// in the transform's target lineage.
///
/// A record contributes one item per referenced ELEMENT (the refs whose binding
/// descriptor scoring would otherwise re-decide) and, when it references the moved
/// body without naming an element (a boolean tool, a nested transform), one
/// whole-record item. **Suppressed downstream records are skipped**: they never
/// execute, so gating them would stall every op after them for no gain.
///
/// Takes the whole [`Document`] rather than just the records because of the
/// LEGACY-SKETCH BRIDGE (VF-B5a): a `Sketch` record minted before
/// `SketchOpParams::host_face` existed declares no inputs at all, so its host-face
/// dependency is invisible to `inputs.bodies`. Those records are never rewritten
/// (that would move the golden prefix hash of every legacy document), so the gate
/// consults `Document.sketches[..].attachment` for them instead — the sketch is
/// gated identically until its next re-finish stamps the param.
fn transform_gate_items(document: &Document, index: usize) -> Vec<RepairItem> {
    let Some(rec) = document.timeline.records().get(index) else {
        return Vec::new();
    };
    let moved = transform_moved_bodies(rec);
    let label =
        format!("reference re-bind blocked by an edited Move (step {index}); repair to re-pick");
    gate_items_for_moved(document, &moved, index + 1, &label)
}

/// The gate items for every record at or after `from_step` that stands on one of
/// the `moved` bodies — the scanning core shared by the EDIT/SUPPRESS gate
/// ([`transform_gate_items`], which starts one past the transform), the DELETE
/// gate (VF-B5b, which starts AT the removed index because the timeline has
/// already closed up over it), and the REGEN-side VF-B6 ordinal tripwire (which
/// passes the permuted split children as `moved`).
///
/// Public so the regen lane can seed the *same* walk rather than duplicating it:
/// "which downstream records stand on these bodies" must have exactly one answer,
/// or the edit gate and the tripwire would disagree about what is protected.
pub fn gate_items_for_moved(
    document: &Document,
    moved: &[BodyId],
    from_step: usize,
    label: &str,
) -> Vec<RepairItem> {
    if moved.is_empty() {
        return Vec::new();
    }
    let records = document.timeline.records();
    let mut items = Vec::new();
    for (step, down) in records.iter().enumerate().skip(from_step) {
        if down.suppressed {
            continue;
        }
        if !down.inputs.bodies.iter().any(|b| moved.contains(b)) {
            // Legacy hosted sketch: no derived inputs, but the document-level
            // attachment says it stands on a moved body.
            if let Some(el) = legacy_host_face_element(document, down, moved) {
                items.push(gate_item(
                    step,
                    &down.record_id.to_string(),
                    0,
                    Some(el),
                    label,
                ));
            }
            continue;
        }
        if down.inputs.elements.is_empty() {
            items.push(gate_item(step, &down.record_id.to_string(), 0, None, label));
            continue;
        }
        for (k, el) in down.inputs.elements.iter().enumerate() {
            items.push(gate_item(
                step,
                &down.record_id.to_string(),
                k,
                Some(el.clone()),
                label,
            ));
        }
    }
    items
}

/// The host-face element of a **legacy** `Sketch` record (params carry no
/// `host_face`) whose document attachment binds it to one of `moved` — the
/// VF-B5a bridge described on [`transform_gate_items`]. `None` for anything else,
/// including a record that already carries the param (that one is covered by its
/// derived inputs) and a host ref with no `primary` binding (nothing to gate on).
fn legacy_host_face_element(
    document: &Document,
    record: &OperationRecord,
    moved: &[BodyId],
) -> Option<crate::ids::ElementId> {
    let Operation::Known(KnownOperation::Sketch(p)) = &record.op else {
        return None;
    };
    if p.host_face.is_some() {
        return None;
    }
    let SketchAttachment::HostFace { face, .. } = &document.sketches.get(&p.sketch)?.attachment
    else {
        return None;
    };
    let primary = face.primary.as_ref()?;
    moved
        .contains(&primary.body)
        .then(|| primary.element.clone())
}

/// One seeded gate item. `ref_id` follows the worker's `"<opId>.input<k>"`
/// convention so a repair UI keys it the same way as a ladder-produced item.
fn gate_item(
    step: usize,
    op_id: &str,
    k: usize,
    element: Option<crate::ids::ElementId>,
    label: &str,
) -> RepairItem {
    RepairItem {
        step_index: step,
        ref_id: format!("{op_id}.input{k}"),
        element_id: element,
        ladder_failed: LadderLevel::Descriptor,
        reason: RepairReason::LowConfidence,
        candidates: Vec::new(),
        scoring_version: None,
        anchor: None,
        ui_label: label.to_string(),
        seeded: true,
        ordinal_anchor: None,
    }
}

/// The standard rejection reason when `UpdateOperationParams` is asked to change
/// `opType` outside the one sanctioned pair.
const OP_TYPE_EDIT_REASON: &str = "UpdateOperationParams may not change opType";

/// The `distance2` refinement of [`OP_TYPE_EDIT_REASON`] (SCHEMA §7.3,
/// 2026-08-03). Named as a constant because the mock client mirrors this exact
/// string (`src/ipc/mockClient.ts`) so the mock lane cannot stay green on an edit
/// the real backend rejects.
const CHAMFER_D2_FLIP_REASON: &str = "UpdateOperationParams may not change opType: a Chamfer carrying distance2 is not flippable to Fillet (clear distance2 first)";

/// The `angleDeg` refinement of [`OP_TYPE_EDIT_REASON`] (SCHEMA §7.3). Same
/// field-identity precondition as [`CHAMFER_D2_FLIP_REASON`]: a distance-angle
/// chamfer carries a field `FilletParams` has no home for, so the flip would drop
/// the user's angle silently.
const CHAMFER_ANGLE_FLIP_REASON: &str = "UpdateOperationParams may not change opType: a Chamfer carrying angleDeg is not flippable to Fillet (clear angleDeg first)";

/// Whether `UpdateOperationParams` may rewrite `prior` into `next` — `Ok(())`, or
/// the reason it may not.
///
/// **The default is NO.** `opType` is structural, not cosmetic: dependents bind
/// through it, the planner hash includes it (`planner.rs`), and the worker
/// dispatches an op on `opType` ALONE (`PlanExecutor.cpp:234-235`). Changing it
/// under a "params update" would silently re-author the operation.
///
/// Exactly ONE pair is sanctioned — **Fillet ⇄ Chamfer**, both directions —
/// because every property that makes a swap dangerous is absent there:
/// * [`FilletParams`](crate::document::record::FilletParams) and
///   [`ChamferParams`](crate::document::record::ChamferParams) are
///   FIELD-IDENTICAL, so the payload is interchangeable
///   with no field invented or dropped;
/// * both derive the same inputs, so `derive_inputs` is preserved and the
///   dependency graph does not move;
/// * they share one validator ([`validate_fillet_lockstep`]), so the edge
///   lockstep invariant holds identically across the swap;
/// * the worker keys only on `opType`, so the swapped record executes the other
///   kernel op with no further translation;
/// * one frontend tool authors both (the unified Fillet/Chamfer edge tool), so
///   this is a real user gesture rather than a synthetic edit; and
/// * the inverse is `Inverse::RestoreRecord` of the WHOLE prior record, so undo
///   restores the original `opType` (and params) exactly, and redo re-runs this
///   same symmetric guard.
///
/// **`distance2` breaks the first bullet** (SCHEMA §7.3, 2026-08-03 — WP-C T2a).
/// A two-distance chamfer carries a field a Fillet has no home for, so flipping it
/// would DROP the user's second leg silently. The precondition is therefore
/// enforced on the PRIOR record: a stored Chamfer with `distance2` set is not
/// flippable until that field is cleared by an ordinary (visible, undoable) params
/// edit. The other direction stays open — a Fillet becoming a two-distance Chamfer
/// invents nothing the target type cannot hold. **`angleDeg` breaks it the same
/// way** and carries the same precondition, with its own named reason.
///
/// **Widening rule:** a new pair may be added only when it is likewise
/// params-interchangeable AND `derive_inputs`-preserving. Anything else is a
/// remove + re-add, not a params update.
///
/// Matching is on ENUM VARIANTS, never on `opType` strings: a string allow-list
/// would also admit a `Known` ⇄ `Opaque` crossing, and `validate_temporal` is
/// vacuous for an opaque record — the swap would skip the anti-time-travel
/// check entirely.
fn op_type_edit_allowed(prior: &Operation, next: &Operation) -> Result<(), &'static str> {
    if same_op_type(prior, next) {
        return Ok(());
    }
    match (prior, next) {
        (
            Operation::Known(KnownOperation::Fillet(_)),
            Operation::Known(KnownOperation::Chamfer(_)),
        ) => Ok(()),
        (
            Operation::Known(KnownOperation::Chamfer(p)),
            Operation::Known(KnownOperation::Fillet(_)),
        ) => {
            if p.distance2.is_some() {
                Err(CHAMFER_D2_FLIP_REASON)
            } else if p.angle_deg.is_some() {
                Err(CHAMFER_ANGLE_FLIP_REASON)
            } else {
                Ok(())
            }
        }
        // DetachComponent (spec §3.4, "the honest break link"): a placed
        // component may be detached in place, one-directional — the reverse
        // (re-attaching a library identity to a bare detached body) is not a
        // sanctioned edit, mirroring how a Chamfer with `distance2` set
        // refuses to become a Fillet rather than silently dropping data.
        (
            Operation::Known(KnownOperation::PlaceComponent(_)),
            Operation::Known(KnownOperation::DetachComponent(_)),
        ) => Ok(()),
        _ => Err(OP_TYPE_EDIT_REASON),
    }
}

/// Rewrites the legacy SCHEMA §7.3 `mode` string to agree with the op's own
/// `opType` after a sanctioned Fillet⇄Chamfer swap.
///
/// `mode` is redundant with the authoritative `opType` tag and is NOT modelled;
/// it round-trips through the `extra` flatten (`record.rs:743-744`) whenever a
/// document carried one. A swap that left it alone would persist the
/// self-contradicting `{"opType":"Chamfer","params":{"mode":"Fillet"}}` across
/// save/reload. The key is only WRITTEN when one of the two records already
/// carried it — a document that never had a `mode` never grows one.
fn normalize_edge_op_mode(op: &mut Operation, prior: &Operation) {
    if !edge_op_has_mode(op) && !edge_op_has_mode(prior) {
        return;
    }
    let op_type = serde_json::Value::String(op.op_type().to_string());
    let extra = match op {
        Operation::Known(KnownOperation::Fillet(p)) => &mut p.extra,
        Operation::Known(KnownOperation::Chamfer(p)) => &mut p.extra,
        _ => return,
    };
    extra.insert("mode".into(), op_type);
}

/// Whether a fillet/chamfer op carries the legacy `mode` string in its `extra`.
fn edge_op_has_mode(op: &Operation) -> bool {
    match op {
        Operation::Known(KnownOperation::Fillet(p)) => p.extra.contains_key("mode"),
        Operation::Known(KnownOperation::Chamfer(p)) => p.extra.contains_key("mode"),
        _ => false,
    }
}

/// True iff two operations have the same `opType` — the base case of
/// [`op_type_edit_allowed`], which owns the one sanctioned exception.
fn same_op_type(a: &Operation, b: &Operation) -> bool {
    match (a, b) {
        (Operation::Known(x), Operation::Known(y)) => {
            std::mem::discriminant(x) == std::mem::discriminant(y)
        }
        (Operation::Opaque(x), Operation::Opaque(y)) => x.raw.get("opType") == y.raw.get("opType"),
        _ => false,
    }
}

/// The timeline-record plane ref for a sketch frame + attachment. World
/// attachments keep their named kind; host-face / datum frames serialize the
/// resolved custom basis (the frame is frozen with the sketch — MODEL-OPS W2).
///
/// MIRRORS `document_runtime::plane_ref_of` (which builds the same ref when a
/// sketch session finishes), so a record restamped by
/// [`DocumentSession::restamp_sketch_record`] is byte-identical to one minted at
/// finish time for the same sketch. Pinned by
/// `reattaching_a_sketch_moves_the_timeline_record_plane`.
fn plane_ref_of(
    plane: crate::sketch::SketchPlane,
    attachment: &SketchAttachment,
) -> crate::document::record::SketchPlaneRef {
    use crate::document::record::{PlaneKind, SketchPlaneRef};
    use crate::sketch::WorldPlane;
    let kind = match attachment {
        SketchAttachment::World { plane } => match plane {
            WorldPlane::XY => PlaneKind::Xy,
            WorldPlane::XZ => PlaneKind::Xz,
            WorldPlane::YZ => PlaneKind::Yz,
        },
        _ => PlaneKind::Custom,
    };
    SketchPlaneRef {
        kind,
        origin: plane.origin,
        x_axis: plane.x_axis,
        y_axis: plane.y_axis,
        normal: plane.normal,
        extra: Default::default(),
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
            match reference {
                // Repair candidates are semantic edge refs. Write both the typed
                // evidence and the legacy mirror in one transaction; later replay
                // always takes the typed path when it exists.
                InputRef::Element(edge_ref) => {
                    let primary = edge_ref.primary.as_ref().ok_or_else(|| {
                        DomainError::InvalidReference(
                            "revolve axis edge ref requires a primary binding".into(),
                        )
                    })?;
                    if primary.kind != crate::document::refs::ElementKind::Edge {
                        return Err(DomainError::InvalidReference(
                            "revolve axis ref must be an edge".into(),
                        ));
                    }
                    p.axis = Some(crate::document::refs::AxisRef::Element {
                        body: primary.body,
                        edge: primary.element.clone(),
                        edge_ref: Some(edge_ref.clone()),
                        extra: Default::default(),
                    });
                }
                _ => p.axis = Some(want_axis(reference)?),
            }
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
        (InputPath::ShellOpenFaces { index }, KnownOperation::Shell(p)) => {
            let face = want_element(reference)?;
            if p.target_body.is_none() {
                p.target_body = face.primary.as_ref().map(|primary| primary.body);
            }
            set_shell_open_face(&mut p.faces, &mut p.open_faces, *index, face)?;
        }
        (InputPath::HoleFace, KnownOperation::Hole(p)) => {
            let face = want_element(reference)?;
            // A hole seats its drill on this face and validates its frozen `point`
            // against it, so a ref with no identity would silently fall through to
            // the ladder on a slot the user just re-picked BY HAND. Same bar as
            // `set_fillet_edge`: an explicit rebind must carry an explicit id.
            if face.primary.is_none() {
                return Err(DomainError::InvalidReference(
                    "a hole host-face ref must carry a primary element id".into(),
                ));
            }
            p.face = face;
        }
        (InputPath::OffsetFaceFace { index }, KnownOperation::OffsetFace(p)) => {
            let old_id = p.face_ids.get(*index).cloned();
            let face = want_element(reference)?;
            let new_id = face
                .primary
                .as_ref()
                .map(|primary| primary.element.clone())
                .ok_or_else(|| {
                    DomainError::InvalidReference(
                        "an offset-face ref must carry a primary element id".into(),
                    )
                })?;
            set_offset_face(&mut p.faces, &mut p.face_ids, *index, face)?;
            if let Some(old_id) = old_id {
                for primary in &mut p.primary_face_ids {
                    if *primary == old_id {
                        *primary = new_id.clone();
                    }
                }
            }
        }
        (InputPath::OffsetFaceOpposite, KnownOperation::OffsetFace(p)) => {
            let face = want_element(reference)?;
            // Same bar as `set_offset_face`/`set_fillet_edge`: an explicit rebind
            // must carry an explicit id, or the slot the user just re-picked BY
            // HAND would fall straight back through to the ladder.
            let element = face
                .primary
                .as_ref()
                .map(|p| p.element.clone())
                .ok_or_else(|| {
                    DomainError::InvalidReference(
                        "an offset-face opposite ref must carry a primary element id".into(),
                    )
                })?;
            p.opposite_face_id = Some(element);
            p.opposite_face = Some(face);
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

/// Sets OffsetFace operative face `index`, keeping the typed `faces` ref and the
/// bare `face_ids` element id consistent (both must be populated — see
/// [`OffsetFaceParams`](crate::document::record::OffsetFaceParams)). The typed ref
/// must carry a `primary` element id.
///
/// Same bounds/append discipline as [`set_fillet_edge`]: overwrite a slot, append
/// exactly at the end, refuse a gap past it.
fn set_offset_face(
    faces: &mut Vec<ElementRef>,
    face_ids: &mut Vec<crate::ids::ElementId>,
    index: usize,
    reference: ElementRef,
) -> Result<(), DomainError> {
    let element = reference
        .primary
        .as_ref()
        .map(|p| p.element.clone())
        .ok_or_else(|| {
            DomainError::InvalidReference(
                "an offset-face operative ref must carry a primary element id".into(),
            )
        })?;
    let len = faces.len().max(face_ids.len());
    if index > len {
        return Err(DomainError::InvalidReference(format!(
            "offset face index {index} out of range (len {len})"
        )));
    }
    if index == faces.len() {
        faces.push(reference);
    } else {
        faces[index] = reference;
    }
    if index == face_ids.len() {
        face_ids.push(element);
    } else {
        face_ids[index] = element;
    }
    Ok(())
}

/// Sets shell open face `index`, preserving the typed ref and keeping its primary
/// element id in lockstep with `open_faces`.
fn set_shell_open_face(
    faces: &mut Vec<ElementRef>,
    open_faces: &mut Vec<crate::ids::ElementId>,
    index: usize,
    reference: ElementRef,
) -> Result<(), DomainError> {
    let primary = reference.primary.as_ref().ok_or_else(|| {
        DomainError::InvalidReference(
            "a shell open-face ref must carry a primary element id".into(),
        )
    })?;
    if primary.kind != ElementKind::Face {
        return Err(DomainError::InvalidReference(
            "a shell open-face ref must carry a FACE primary".into(),
        ));
    }
    let element = primary.element.clone();
    let upgrades_legacy_single = faces.is_empty() && open_faces.len() == 1 && index == 0;
    if faces.len() != open_faces.len() && !upgrades_legacy_single {
        return Err(DomainError::InvalidReference(
            "cannot partially rebind a legacy multi-face shell; re-author all typed face refs"
                .into(),
        ));
    }
    let len = faces.len().max(open_faces.len());
    if index > len {
        return Err(DomainError::InvalidReference(format!(
            "shell open face index {index} out of range (len {len})"
        )));
    }
    if index == faces.len() {
        faces.push(reference);
    } else {
        faces[index] = reference;
    }
    if index == open_faces.len() {
        open_faces.push(element);
    } else {
        open_faces[index] = element;
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

/// The redo command for a squashed sketch session: full-replace `prior`'s body
/// with `final_sketch`'s, in ONE batch (WP-P F5).
///
/// Order is load-bearing. The unlock prefix comes first because the reference
/// lock refuses `RemoveEntity`, so a session that started with projected geometry
/// could otherwise never be redone at all. The provenance rows come last because
/// `AddEntity` carries the entity's `referenceLocked` flag but NOT its row, and a
/// redo that restored the geometry without the row would leave the user with
/// locked curves that no longer track — or can be updated from — anything.
///
/// Only geometry [`is_projected_geometry`] permits is unlocked: the prefix must
/// not become a way around the guard `apply_sketch_ops` enforces.
fn squash_redo_ops(prior: &Sketch, final_sketch: &Sketch) -> Vec<SketchEditOp> {
    let mut ops: Vec<SketchEditOp> = Vec::new();
    for e in prior.entities() {
        if e.is_reference_locked() && is_projected_geometry(prior, e.id()) {
            ops.push(SketchEditOp::SetEntityReferenceLocked {
                entity: e.id(),
                locked: false,
            });
        }
    }
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
    for (entity, source) in &final_sketch.projections {
        ops.push(SketchEditOp::SetEntityProjection {
            entity: *entity,
            source: Some(source.clone()),
        });
    }
    ops
}

/// Whether `entity` is part of `sketch`'s PROJECTED reference geometry: a curve
/// with a provenance row, or a point one of those curves is built from.
///
/// The points carry no row of their own — the merge that closes an outline gives
/// a shared corner no single source — so they have to be recognised through the
/// curves that name them, or a detach could never free them.
fn is_projected_geometry(sketch: &Sketch, entity: crate::ids::EntityId) -> bool {
    sketch.projections.contains_key(&entity)
        || sketch.projections.keys().any(|owner| {
            sketch
                .get_entity(*owner)
                .is_some_and(|e| e.referenced_entities().contains(&entity))
        })
}

/// The unlock guard (WP-P F11): `SetEntityReferenceLocked { locked: false }` is
/// legal only for geometry the sketch holds as PROJECTED reference geometry.
///
/// `sketch_upsert` takes a `Vec<SketchEditOp>` straight from the frontend, so
/// without this the one named exit from the lock is an exit for ANY locked
/// entity — including a sketch-on-face's host boundary, which carries no
/// provenance row and exists precisely so the profile cannot be dragged off the
/// face it was authored against.
///
/// Judged against `prior`, the sketch as it was at the START of the batch,
/// because a detach clears the provenance row and unlocks in ONE transaction and
/// an update unlocks, moves and re-locks in another.
fn guard_unlock(prior: &Sketch, entity: crate::ids::EntityId) -> Result<(), DomainError> {
    if is_projected_geometry(prior, entity) {
        return Ok(());
    }
    Err(DomainError::Validation(format!(
        "entity {entity} is reference-locked host geometry, not a projection — only a projected \
         entity (or a point one is built from) can be unlocked"
    )))
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
///
/// [`SketchEditOp::SetEntityReferenceLocked`] is the ONE op that is exempt: it is
/// the named exit from the lock (WP-P detach). Everything else stays absolute, so
/// unlocking is always an explicit, undoable step and never a side effect. Ops
/// apply IN ORDER, so a batch may unlock and then edit within one transaction.
/// The exit is SCOPED by [`guard_unlock`] — only projected geometry may take it.
fn apply_sketch_ops(prior: &Sketch, ops: &[SketchEditOp]) -> Result<Sketch, DomainError> {
    let mut entities: Vec<SketchEntity> = prior.entities().to_vec();
    let mut constraints: Vec<Constraint> = prior.constraints().to_vec();
    let mut projections = prior.projections.clone();

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
            SketchEditOp::SetEntityReferenceLocked { entity, locked } => {
                if !*locked {
                    guard_unlock(prior, *entity)?;
                }
                let e = entities
                    .iter_mut()
                    .find(|e| e.id() == *entity)
                    .ok_or_else(|| DomainError::Validation(format!("entity {entity} not found")))?;
                e.set_reference_locked(*locked);
            }
            SketchEditOp::SetEntityProjection { entity, source } => {
                if !entities.iter().any(|e| e.id() == *entity) {
                    return Err(DomainError::Validation(format!(
                        "entity {entity} not found"
                    )));
                }
                match source {
                    Some(source) => projections.insert(*entity, source.clone()),
                    None => projections.remove(entity),
                };
            }
        }
    }
    rebuild_sketch(prior, entities, constraints, projections)
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
    projections: BTreeMap<crate::ids::EntityId, crate::sketch::ProjectedSource>,
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
    // A cascade-removed entity takes its provenance row with it: the map must
    // never name an entity the sketch does not have (WP-P).
    s.projections = projections
        .into_iter()
        .filter(|(id, _)| s.contains_entity(*id))
        .collect();
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
        // NOTE: the *_position fields ride through a value edit. Dropping them
        // would silently re-bind an arc-endpoint dimension to the arc's centre
        // the first time the user retyped its value.
        Constraint::Distance {
            id,
            entity1,
            entity1_position,
            entity2,
            entity2_position,
            ..
        } => Constraint::Distance {
            id: *id,
            entity1: *entity1,
            entity1_position: *entity1_position,
            entity2: *entity2,
            entity2_position: *entity2_position,
            value,
        },
        Constraint::HorizontalDistance {
            id,
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => Constraint::HorizontalDistance {
            id: *id,
            point1: *point1,
            point1_position: *point1_position,
            point2: *point2,
            point2_position: *point2_position,
            value,
        },
        Constraint::VerticalDistance {
            id,
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => Constraint::VerticalDistance {
            id: *id,
            point1: *point1,
            point1_position: *point1_position,
            point2: *point2,
            point2_position: *point2_position,
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
    use crate::ids::{ConstraintId, DocumentId, EntityId};
    use crate::sketch::CurvePosition;
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

    // ── module state (ADR-0004 / ADR-0005) ──────────────────────────────────

    fn module(id: &str) -> crate::document::modules::ModuleId {
        crate::document::modules::ModuleId::parse(id).unwrap()
    }

    #[test]
    fn module_state_is_written_verbatim_and_undoes_exactly() {
        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        let foo = module("com.example.foo");
        let payload = serde_json::json!({ "note": "keep me", "nested": [1, 2, 3] });

        session
            .apply(EditCommand::SetModuleState {
                module: foo.clone(),
                state: Some(ModuleState::new(2, payload.clone())),
            })
            .expect("writing module state is always valid");

        let stored = &session.document().modules[&foo];
        assert_eq!(stored.schema_version, 2);
        // Verbatim: the platform has no schema for this and must not normalize it.
        assert_eq!(stored.payload, payload);

        session.undo().expect("module writes are undoable");
        assert!(!session.document().modules.contains_key(&foo));

        session.redo().expect("and redoable");
        assert_eq!(session.document().modules[&foo].payload, payload);
    }

    #[test]
    fn overwriting_module_state_restores_the_prior_slice_on_undo() {
        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        let foo = module("com.example.foo");
        let first = ModuleState::new(1, serde_json::json!({ "v": 1 }));
        let second = ModuleState::new(2, serde_json::json!({ "v": 2 }));

        for state in [first.clone(), second.clone()] {
            session
                .apply(EditCommand::SetModuleState {
                    module: foo.clone(),
                    state: Some(state),
                })
                .unwrap();
        }
        assert_eq!(session.document().modules[&foo], second);

        session.undo().unwrap();
        assert_eq!(
            session.document().modules[&foo],
            first,
            "undo must restore the prior slice, not merely delete the new one"
        );
    }

    #[test]
    fn clearing_module_state_removes_it_and_undo_brings_it_back() {
        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        let foo = module("com.example.foo");
        let state = ModuleState::new(1, serde_json::json!({ "v": 1 }));

        session
            .apply(EditCommand::SetModuleState {
                module: foo.clone(),
                state: Some(state.clone()),
            })
            .unwrap();
        session
            .apply(EditCommand::SetModuleState {
                module: foo.clone(),
                state: None,
            })
            .unwrap();
        assert!(!session.document().modules.contains_key(&foo));

        session.undo().unwrap();
        assert_eq!(session.document().modules[&foo], state);
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
            entity1_position: CurvePosition::Arbitrary,
            entity2: eid(2),
            entity2_position: CurvePosition::Arbitrary,
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

    /// WP-P B2: the unlock op is the ONE exit from the lock, and a batch may
    /// unlock and then edit in the same transaction (ops apply in order).
    #[test]
    fn the_unlock_op_is_the_only_way_out_of_reference_locked() {
        // The exit is scoped to PROJECTED geometry (F11), so the circle carries a
        // provenance row — exactly what a detach unlocks.
        let mut s = locked_sketch();
        s.projections
            .insert(eid(3), projected_source("aaaaaaaaaaaaaaaa"));

        // Unlock alone — the flag flips, nothing else moves.
        let unlocked = apply_sketch_ops(
            &s,
            &[SketchEditOp::SetEntityReferenceLocked {
                entity: eid(3),
                locked: false,
            }],
        )
        .expect("the unlock op is exempt from the lock guard");
        assert!(!unlocked.get_entity(eid(3)).unwrap().is_reference_locked());
        assert!(
            unlocked.get_entity(eid(1)).unwrap().is_reference_locked(),
            "only the named entity is unlocked"
        );

        // Unlock THEN edit, in one batch — the guard sees the working state, so
        // the second op is no longer refused.
        let flipped = apply_sketch_ops(
            &s,
            &[
                SketchEditOp::SetEntityReferenceLocked {
                    entity: eid(3),
                    locked: false,
                },
                SketchEditOp::SetEntityConstruction {
                    entity: eid(3),
                    construction: true,
                },
            ],
        )
        .expect("an unlocked entity accepts a construction flip");
        assert!(flipped.get_entity(eid(3)).unwrap().is_construction());

        // Order matters: the same pair the other way round is still refused.
        assert!(apply_sketch_ops(
            &s,
            &[
                SketchEditOp::SetEntityConstruction {
                    entity: eid(3),
                    construction: true,
                },
                SketchEditOp::SetEntityReferenceLocked {
                    entity: eid(3),
                    locked: false,
                },
            ],
        )
        .is_err());
    }

    /// WP-P F11: the exit from the lock is SCOPED. `sketch_upsert` takes ops
    /// straight from the frontend, so an unlock that any locked entity could take
    /// would let the host boundary of a sketch-on-face be dragged off its face.
    #[test]
    fn the_unlock_op_refuses_geometry_that_is_not_projected() {
        let s = locked_sketch(); // locked host geometry, NO provenance rows
        let unlock = |entity| {
            apply_sketch_ops(
                &s,
                &[SketchEditOp::SetEntityReferenceLocked {
                    entity,
                    locked: false,
                }],
            )
        };
        for entity in [eid(3), eid(1)] {
            let err = unlock(entity).expect_err("unlocking host geometry is refused");
            assert!(
                matches!(&err, DomainError::Validation(m) if m.contains("not a projection")),
                "got {err:?}"
            );
        }

        // The SAME entity, once it is projected: allowed — and so is the point it
        // is built from, which carries no row of its own (the merge gives a shared
        // corner no single source).
        let mut projected = s.clone();
        projected
            .projections
            .insert(eid(3), projected_source("aaaaaaaaaaaaaaaa"));
        for entity in [eid(3), eid(1)] {
            assert!(
                apply_sketch_ops(
                    &projected,
                    &[SketchEditOp::SetEntityReferenceLocked {
                        entity,
                        locked: false,
                    }],
                )
                .is_ok(),
                "projected geometry and its points stay unlockable ({entity})"
            );
        }

        // Re-LOCKING is never guarded: an update unlocks, moves and re-locks.
        assert!(apply_sketch_ops(
            &s,
            &[SketchEditOp::SetEntityReferenceLocked {
                entity: eid(2),
                locked: true,
            }],
        )
        .is_ok());
    }

    #[test]
    fn the_unlock_op_rejects_an_unknown_entity() {
        let s = locked_sketch();
        assert!(matches!(
            apply_sketch_ops(
                &s,
                &[SketchEditOp::SetEntityReferenceLocked {
                    entity: eid(999),
                    locked: false,
                }]
            ),
            Err(DomainError::Validation(_))
        ));
    }

    fn projected_source(hash: &str) -> crate::sketch::ProjectedSource {
        crate::sketch::ProjectedSource {
            source_body: crate::ids::BodyId(uuid::Uuid::from_u128(0xB0D1)),
            source_element_id: crate::ids::ElementId::new("el_edge_7"),
            source_kind: crate::document::refs::ElementKind::Edge,
            source_ordinal: 0,
            projected_hash: hash.to_string(),
        }
    }

    /// WP-P: the provenance map is written, replaced and cleared through ops, so
    /// project / update / detach each ride ONE undoable transaction.
    #[test]
    fn projection_rows_are_set_replaced_and_cleared_by_ops() {
        let s = locked_sketch();

        let set = apply_sketch_ops(
            &s,
            &[SketchEditOp::SetEntityProjection {
                entity: eid(3),
                source: Some(projected_source("aaaaaaaaaaaaaaaa")),
            }],
        )
        .unwrap();
        assert_eq!(set.projections[&eid(3)].projected_hash, "aaaaaaaaaaaaaaaa");

        let replaced = apply_sketch_ops(
            &set,
            &[SketchEditOp::SetEntityProjection {
                entity: eid(3),
                source: Some(projected_source("bbbbbbbbbbbbbbbb")),
            }],
        )
        .unwrap();
        assert_eq!(
            replaced.projections[&eid(3)].projected_hash,
            "bbbbbbbbbbbbbbbb"
        );

        let cleared = apply_sketch_ops(
            &replaced,
            &[SketchEditOp::SetEntityProjection {
                entity: eid(3),
                source: None,
            }],
        )
        .unwrap();
        assert!(cleared.projections.is_empty());
    }

    #[test]
    fn a_projection_row_cannot_name_an_entity_the_sketch_lacks() {
        let s = locked_sketch();
        assert!(matches!(
            apply_sketch_ops(
                &s,
                &[SketchEditOp::SetEntityProjection {
                    entity: eid(999),
                    source: Some(projected_source("aaaaaaaaaaaaaaaa")),
                }]
            ),
            Err(DomainError::Validation(_))
        ));
    }

    /// A removed entity takes its provenance row with it — the full detach
    /// sequence (clear the row, unlock, remove) leaves nothing behind, and so
    /// does an unlock-then-remove that forgets to clear the row.
    #[test]
    fn removing_an_entity_drops_its_projection_row() {
        let s = apply_sketch_ops(
            &locked_sketch(),
            &[SketchEditOp::SetEntityProjection {
                entity: eid(3),
                source: Some(projected_source("aaaaaaaaaaaaaaaa")),
            }],
        )
        .unwrap();
        assert_eq!(s.projections.len(), 1);

        let gone = apply_sketch_ops(
            &s,
            &[
                SketchEditOp::SetEntityReferenceLocked {
                    entity: eid(3),
                    locked: false,
                },
                SketchEditOp::RemoveEntity { entity: eid(3) },
            ],
        )
        .unwrap();
        assert!(gone.get_entity(eid(3)).is_none());
        assert!(
            gone.projections.is_empty(),
            "the map may never name an entity the sketch does not have"
        );
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

        assert!(session.undo().is_some(), "undo available");
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
        assert!(session.redo().unwrap().is_some(), "redo available");
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

    /// WP-P F5: a squashed sketch session redoes into the same sketch it undid
    /// from — provenance rows and reference locks included — and a session that
    /// STARTED with projected locked geometry replays at all.
    #[test]
    fn squash_redo_restores_projections_and_replays_over_locked_geometry() {
        use crate::document::Document;
        use crate::ids::DocumentId;

        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        session
            .apply(EditCommand::AddSketch {
                sketch: base_sketch(),
            })
            .unwrap();
        let prior = session.document().sketch(sid()).unwrap().clone();
        let watermark = session.undo_depth();

        // One session step that PROJECTS: two locked points, a locked line, and
        // the provenance row that makes the line a projection.
        let (p0, p1, line) = (eid(60), eid(61), eid(62));
        session
            .apply(EditCommand::SketchEdit {
                sketch: sid(),
                ops: vec![
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::point(p0, Vec2::new_unchecked(0.0, 0.0), false, true),
                    },
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::point(p1, Vec2::new_unchecked(9.0, 0.0), false, true),
                    },
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::line(line, p0, p1, false).with_reference_locked(true),
                    },
                    SketchEditOp::SetEntityProjection {
                        entity: line,
                        source: Some(projected_source("aaaaaaaaaaaaaaaa")),
                    },
                ],
            })
            .unwrap();
        let count = session.undo_depth() - watermark;
        session.squash_sketch_session(sid(), prior, count);

        assert!(session.undo().is_some(), "undo the whole session");
        assert!(
            session
                .document()
                .sketch(sid())
                .unwrap()
                .projections
                .is_empty(),
            "undo restores the pre-session sketch verbatim"
        );

        session
            .redo()
            .expect("the net sketch edit replays")
            .expect("a step to redo");
        let redone = session.document().sketch(sid()).unwrap().clone();
        assert_eq!(
            redone
                .projections
                .get(&line)
                .map(|s| s.projected_hash.as_str()),
            Some("aaaaaaaaaaaaaaaa"),
            "the redo restores the provenance ROW, not just the geometry"
        );
        assert!(
            redone.get_entity(line).unwrap().is_reference_locked(),
            "…and the lock that goes with it"
        );

        // A SECOND session, this time starting FROM projected locked geometry: the
        // redo's remove-all prelude has to unlock it or the replay is refused.
        let watermark = session.undo_depth();
        session
            .apply(EditCommand::SketchEdit {
                sketch: sid(),
                ops: vec![SketchEditOp::AddEntity {
                    entity: SketchEntity::point(
                        eid(63),
                        Vec2::new_unchecked(4.0, 4.0),
                        false,
                        false,
                    ),
                }],
            })
            .unwrap();
        let count = session.undo_depth() - watermark;
        session.squash_sketch_session(sid(), redone, count);
        assert!(session.undo().is_some());
        session
            .redo()
            .expect("a session over prior locked geometry redoes without error")
            .expect("a step to redo");
        let redone = session.document().sketch(sid()).unwrap();
        assert!(
            redone.get_entity(eid(63)).is_some(),
            "the session's own edit"
        );
        assert!(
            redone.projections.contains_key(&line),
            "the pre-existing projection survives the replay"
        );
        assert!(redone.get_entity(line).unwrap().is_reference_locked());
    }

    /// A session that only moved the provenance map is NOT net-zero: squashing it
    /// away would drop an `update_projection` / `detach_projection` out of the
    /// undo stack entirely (WP-P F5).
    #[test]
    fn a_projection_only_session_is_not_net_zero() {
        use crate::document::Document;
        use crate::ids::DocumentId;

        let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(1))));
        let mut seeded = base_sketch();
        seeded.projections.insert(eid(3), projected_source("aaaa"));
        session
            .apply(EditCommand::AddSketch { sketch: seeded })
            .unwrap();
        let prior = session.document().sketch(sid()).unwrap().clone();
        let watermark = session.undo_depth();
        session
            .apply(EditCommand::SketchEdit {
                sketch: sid(),
                ops: vec![SketchEditOp::SetEntityProjection {
                    entity: eid(3),
                    source: None,
                }],
            })
            .unwrap();
        let count = session.undo_depth() - watermark;
        session.squash_sketch_session(sid(), prior, count);
        assert_eq!(session.undo_depth(), watermark + 1, "one squashed step");
        assert!(session.undo().is_some());
        assert!(
            session
                .document()
                .sketch(sid())
                .unwrap()
                .projections
                .contains_key(&eid(3)),
            "the detach is undoable — it was not squashed away as net-zero"
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
        assert!(session.undo().is_some(), "undo the head sketch edit");
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
                point1_position: CurvePosition::Arbitrary,
                point2: eid(2),
                point2_position: CurvePosition::Arbitrary,
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

    fn shell_face(element: &str, kind: ElementKind) -> ElementRef {
        ElementRef {
            primary: Some(crate::document::refs::PrimaryRef {
                body: BodyId(Uuid::from_u128(9)),
                element: ElementId::new(element),
                kind,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(crate::document::refs::AnchorIntent {
                world_point: crate::math::Vec3::new_unchecked(1.0, 2.0, 3.0),
                surface_uv: None,
                local_frame: None,
                adjacency_hint: Some("top".into()),
                extra: Default::default(),
            }),
            extra: Default::default(),
        }
    }

    fn shell_op(open_faces: Vec<ElementId>, faces: Vec<ElementRef>) -> Operation {
        Operation::Known(KnownOperation::Shell(
            crate::document::record::ShellParams {
                thickness: Scalar::new(2.0),
                open_faces,
                faces,
                target_body: Some(BodyId(Uuid::from_u128(9))),
                extra: Default::default(),
            },
        ))
    }

    #[test]
    fn shell_typed_faces_require_lockstep_face_primaries() {
        assert!(validate_shell_lockstep(&shell_op(vec![ElementId::new("f1")], vec![])).is_ok());
        assert!(validate_shell_lockstep(&shell_op(
            vec![ElementId::new("f1")],
            vec![shell_face("f1", ElementKind::Face)]
        ))
        .is_ok());
        for bad in [
            shell_op(
                vec![ElementId::new("f1"), ElementId::new("f2")],
                vec![shell_face("f1", ElementKind::Face)],
            ),
            shell_op(
                vec![ElementId::new("f1")],
                vec![shell_face("f2", ElementKind::Face)],
            ),
            shell_op(
                vec![ElementId::new("f1")],
                vec![shell_face("f1", ElementKind::Edge)],
            ),
        ] {
            assert!(validate_shell_lockstep(&bad).is_err());
        }

        let mut wrong_body = shell_face("f1", ElementKind::Face);
        wrong_body.primary.as_mut().unwrap().body = BodyId(Uuid::from_u128(10));
        let err = validate_shell_lockstep(&shell_op(vec![ElementId::new("f1")], vec![wrong_body]))
            .unwrap_err();
        assert!(err.to_string().contains("!= targetBodyId"));
    }

    #[test]
    fn edge_ops_require_one_typed_operated_body() {
        let edge_ids = vec![ElementId::new("e1"), ElementId::new("e2")];
        let mut foreign = shell_face("e2", ElementKind::Edge);
        foreign.primary.as_mut().unwrap().body = BodyId(Uuid::from_u128(10));
        for op in [
            Operation::Known(KnownOperation::Fillet(
                crate::document::record::FilletParams {
                    radius: Scalar::new(2.0),
                    edge_ids: edge_ids.clone(),
                    edges: vec![shell_face("e1", ElementKind::Edge), foreign.clone()],
                    chain_tangent_edges: false,
                    tangent_closure_version: None,
                    extra: Default::default(),
                },
            )),
            Operation::Known(KnownOperation::Chamfer(
                crate::document::record::ChamferParams {
                    radius: Scalar::new(2.0),
                    distance2: None,
                    angle_deg: None,
                    edge_ids: edge_ids.clone(),
                    edges: vec![shell_face("e1", ElementKind::Edge), foreign.clone()],
                    chain_tangent_edges: false,
                    tangent_closure_version: None,
                    extra: Default::default(),
                },
            )),
        ] {
            let err = validate_fillet_lockstep(&op).unwrap_err();
            assert!(err.to_string().contains("operated body"));
        }

        let intent_only = Operation::Known(KnownOperation::Fillet(
            crate::document::record::FilletParams {
                radius: Scalar::new(2.0),
                edge_ids: vec![ElementId::new("e1")],
                edges: vec![ElementRef {
                    primary: None,
                    intent: None,
                    anchor: None,
                    extra: Default::default(),
                }],
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Default::default(),
            },
        ));
        assert!(validate_fillet_lockstep(&intent_only)
            .unwrap_err()
            .to_string()
            .contains("EDGE primary"));
    }

    #[test]
    fn shell_rebind_preserves_typed_evidence_and_updates_bare_id() {
        let mut faces = Vec::new();
        let mut open_faces = vec![ElementId::new("legacy")];
        let reference = shell_face("fresh", ElementKind::Face);
        set_shell_open_face(&mut faces, &mut open_faces, 0, reference.clone()).unwrap();
        assert_eq!(faces, vec![reference]);
        assert_eq!(open_faces, vec![ElementId::new("fresh")]);
    }

    #[test]
    fn legacy_shell_append_is_rejected_without_panicking() {
        let mut faces = Vec::new();
        let mut open_faces = vec![ElementId::new("legacy")];
        let err = set_shell_open_face(
            &mut faces,
            &mut open_faces,
            1,
            shell_face("fresh", ElementKind::Face),
        )
        .unwrap_err();
        assert!(err.to_string().contains("re-author all typed face refs"));
        assert!(faces.is_empty());
        assert_eq!(open_faces, vec![ElementId::new("legacy")]);
    }
}
