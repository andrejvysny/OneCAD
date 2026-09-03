//! Undo/redo via **inverse mementos** with transaction batching.
//!
//! Design (plan "Rust core specifics"; C++ `CommandProcessor`):
//!
//! * Each applied [`EditCommand`] captures an [`Inverse`] — a *memento* that
//!   restores the exact prior state, **never** a replayed inverse geometry op
//!   (plan invariant: "undo = history change + replay, never inverse OCCT
//!   mutation"). Because `document.json` persists only `{records, cursor}` (regen
//!   states are derived — see [`crate::document`]), the mementos here restore the
//!   authoritative state exactly.
//! * [`Txn`] groups several applied edits into one user-visible undo step
//!   (C++ `beginTransaction`/`endTransaction`; a group undoes its edits in
//!   reverse).
//! * [`UndoStack`] is bounded to [`UNDO_CAP`] (200) and evicts the oldest step
//!   on overflow (C++ `CommandProcessor.cpp:79`, `undoStack_.erase(begin())`);
//!   the redo stack is cleared on every new apply.
//!
//! Undo restores state via [`Inverse::apply`]; redo **re-executes** the forward
//! command (recomputing a fresh inverse), matching C++ `redo()` calling
//! `execute()` again.

use crate::document::body::BodyRegistry;
use crate::document::datum::DatumPlane;
use crate::document::modules::{ModuleId, ModuleState};
use crate::document::record::OperationRecord;
use crate::document::repair::RepairState;
use crate::document::variables::VariableTable;
use crate::document::Document;
use crate::history::Timeline;
use crate::ids::{DatumPlaneId, RecordId, SketchId};
use crate::sketch::Sketch;

use super::command::EditCommand;

/// Maximum retained undo steps (C++ `kMaxUndoDepth`, `CommandProcessor.cpp:79`).
pub const UNDO_CAP: usize = 200;

/// A memento that restores the exact document state prior to one applied
/// command (or one composite sub-step).
///
/// Timeline-structural inverses operate over `{records, cursor}` (the persisted
/// authoritative fields). Subsystem inverses (`RestoreBodies`,
/// `RestoreVariables`) memento the whole small collection — exact and cheap
/// (bodies/variables are few per document). Where a single command touches
/// several subsystems its inverse is a [`Inverse::Composite`], applied in
/// reverse.
#[derive(Debug, Clone)]
pub enum Inverse {
    /// Undo of `AddOperation`: delete the added record (Timeline.remove restores
    /// the cursor).
    RemoveRecord {
        /// Id of the record to remove.
        record: RecordId,
    },
    /// Undo of `RemoveOperation`: re-insert the record at its original index,
    /// then restore the cursor.
    InsertRecord {
        /// Original timeline index.
        index: usize,
        /// The removed record.
        record: Box<OperationRecord>,
        /// Cursor to restore after re-insertion.
        cursor: usize,
    },
    /// Undo of a record-content edit (params / input / suppression): replace the
    /// record at `index` with its prior full form.
    RestoreRecord {
        /// Timeline index to overwrite.
        index: usize,
        /// The prior record.
        record: Box<OperationRecord>,
    },
    /// Undo of `SetRollback`: restore the cursor.
    RestoreCursor {
        /// Cursor to restore.
        cursor: usize,
    },
    /// Undo of a sketch add/delete/edit/rename/retarget/drag: restore (`Some`)
    /// or remove (`None` — undo of an add) the sketch.
    RestoreSketch {
        /// Target sketch id.
        id: SketchId,
        /// The prior sketch, or `None` if it did not exist.
        prior: Option<Box<Sketch>>,
    },
    /// Undo of a sketch-visibility change.
    RestoreSketchVisibility {
        /// Target sketch id.
        id: SketchId,
        /// The prior override (`None` = no override = visible default).
        prior: Option<bool>,
    },
    /// Undo of a body add/delete/rename/visibility: restore the whole registry.
    RestoreBodies {
        /// The prior body registry.
        registry: Box<BodyRegistry>,
    },
    /// Undo of `AddDatumPlane` (or a datum change): restore (`Some`) or remove
    /// (`None`) the datum.
    RestoreDatum {
        /// Target datum id.
        id: DatumPlaneId,
        /// The prior datum, or `None` if it did not exist.
        prior: Option<Box<DatumPlane>>,
    },
    /// Undo of an add/remove/set variable: restore the whole variable table.
    RestoreVariables {
        /// The prior variable table.
        table: Box<VariableTable>,
    },
    /// Undo of a module-state write: restore that module's prior slice.
    RestoreModuleState {
        /// Owning module/addon.
        module: ModuleId,
        /// The prior slice, or `None` if the module had no state.
        prior: Option<Box<ModuleState>>,
    },
    /// Undo of an edit that changed the repair state.
    RestoreRepair {
        /// The prior repair state.
        state: Box<RepairState>,
    },
    /// Several inverses, applied in **reverse** order (multi-subsystem command).
    Composite(Vec<Inverse>),
    /// Nothing to undo.
    Noop,
}

impl Inverse {
    /// The lowest timeline step whose regen result applying this memento
    /// invalidates — the **dirty floor** the undo/redo replay must start from.
    /// `None` means the memento moves nothing the timeline regenerates (pure
    /// display metadata).
    ///
    /// **Must be evaluated BEFORE [`apply`](Self::apply)**: `RemoveRecord` resolves
    /// its index by looking the record up while it is still in the timeline, and the
    /// sketch arms read the producer/consumer steps of the pre-undo document.
    ///
    /// ## The conservative rule — do not weaken it
    ///
    /// An inverse whose reach over the timeline is not **exactly** known MUST return
    /// `Some(0)`. A floor that is too LOW costs a slower replay; a floor that is too
    /// HIGH is a correctness bug: both the checkpoint eviction
    /// ([`CheckpointStore::invalidate_from`](crate::regen::CheckpointStore::invalidate_from))
    /// and the follow-up regen would then skip steps the undo actually changed, so the
    /// viewport — and the next save — keep geometry the user just reverted. Every new
    /// [`Inverse`] variant needs an arm here; the match is exhaustive on purpose so the
    /// compiler forces the decision.
    #[must_use]
    pub fn dirty_floor(&self, doc: &Document) -> Option<usize> {
        match self {
            // Undo of an add: the record is still present, so its index is exact.
            // A missing record should be impossible (the memento is applied against
            // the state it was captured over) — floor to 0 rather than guess.
            Inverse::RemoveRecord { record } => Some(doc.timeline.index_of(*record).unwrap_or(0)),
            // Both carry the exact index they were captured at.
            Inverse::InsertRecord { index, .. } | Inverse::RestoreRecord { index, .. } => {
                Some(*index)
            }
            // The cursor moves in one direction or the other; everything between the
            // two positions changes applied-ness, so the floor is the lower of them.
            Inverse::RestoreCursor { cursor } => Some((*cursor).min(doc.timeline.cursor())),
            // Same answer the edit lane's `sketch_dirty_outcome` computes.
            Inverse::RestoreSketch { id, .. } => doc.sketch_dirty_step(*id),
            // Display-only: no timeline step regenerates differently.
            Inverse::RestoreSketchVisibility { .. } => None,
            // Module state is not an input to any timeline step — the platform
            // cannot interpret it, so no operation can depend on it. Restoring it
            // regenerates nothing.
            Inverse::RestoreModuleState { .. } => None,
            // Whole-collection mementos. `RestoreBodies` (names/visibility, but also
            // every row adopted from regen since the reverted edit), `RestoreDatum`
            // (a datum a sketch frame was stamped from), `RestoreVariables` (a bare
            // variable name can drive any dimensioned op — the same conservative span
            // the forward `variable_outcome` takes) and `RestoreRepair` (the repair
            // state IS the regen execution ceiling) all restore state whose reach over
            // the timeline this layer cannot bound. Conservative floor, per the rule.
            Inverse::RestoreBodies { .. }
            | Inverse::RestoreDatum { .. }
            | Inverse::RestoreVariables { .. }
            | Inverse::RestoreRepair { .. } => Some(0),
            // The composite's members all apply, so the floor is the lowest of them
            // (all-`None` members ⇒ `None` — nothing regenerates).
            Inverse::Composite(list) => list.iter().filter_map(|i| i.dirty_floor(doc)).min(),
            Inverse::Noop => None,
        }
    }

    /// Restores the mementoed prior state onto `doc`.
    pub(crate) fn apply(self, doc: &mut Document) {
        match self {
            Inverse::RemoveRecord { record } => {
                // The memento is applied against the exact state it was captured
                // over, so the record must be present. Assert in debug to catch a
                // future memento bug; release keeps the graceful no-op on absence.
                let removed = doc.timeline.remove(record);
                debug_assert!(
                    removed.is_ok(),
                    "RemoveRecord inverse: record {record} not found"
                );
            }
            Inverse::InsertRecord {
                index,
                record,
                cursor,
            } => insert_record_at(&mut doc.timeline, index, *record, cursor),
            Inverse::RestoreRecord { index, record } => {
                replace_record_at(&mut doc.timeline, index, *record);
            }
            Inverse::RestoreCursor { cursor } => {
                doc.timeline.set_cursor(cursor);
            }
            Inverse::RestoreSketch { id, prior } => match prior {
                Some(s) => {
                    doc.sketches.insert(id, *s);
                }
                None => {
                    doc.sketches.remove(&id);
                }
            },
            Inverse::RestoreSketchVisibility { id, prior } => match prior {
                Some(v) => {
                    doc.sketch_visibility.insert(id, v);
                }
                None => {
                    doc.sketch_visibility.remove(&id);
                }
            },
            Inverse::RestoreModuleState { module, prior } => match prior {
                Some(state) => {
                    doc.modules.insert(module, *state);
                }
                None => {
                    doc.modules.remove(&module);
                }
            },
            Inverse::RestoreBodies { registry } => doc.bodies = *registry,
            Inverse::RestoreDatum { id, prior } => match prior {
                Some(d) => {
                    doc.datum_planes.insert(id, *d);
                }
                None => {
                    doc.datum_planes.remove(&id);
                }
            },
            Inverse::RestoreVariables { table } => doc.variables = *table,
            Inverse::RestoreRepair { state } => doc.repair = *state,
            Inverse::Composite(list) => {
                for inv in list.into_iter().rev() {
                    inv.apply(doc);
                }
            }
            Inverse::Noop => {}
        }
    }
}

/// What one undo/redo step moved (HISTORY-HARDEN H8).
///
/// Returned by [`DocumentSession::undo`](crate::edit::DocumentSession::undo) /
/// [`redo`](crate::edit::DocumentSession::redo) so the caller can evict checkpoints
/// and replay from a REAL floor instead of blindly regenerating the whole history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoOutcome {
    /// The lowest timeline step whose regen result the step invalidated — folded
    /// (min) across every edit in the transaction. `None` ⇒ the step touched no
    /// regenerated state at all, so no regen is needed.
    ///
    /// Undo derives it from the mementos ([`Inverse::dirty_floor`], evaluated before
    /// each is applied); redo derives it from the re-executed forward commands'
    /// [`CommandOutcome::dirty`](crate::edit::CommandOutcome::dirty) — the same spans
    /// the normal edit lane schedules from.
    pub dirty_floor: Option<usize>,
}

/// Folds a new floor into an accumulator (the lower wins; `None` is "no claim").
pub(crate) fn fold_floor(acc: Option<usize>, next: Option<usize>) -> Option<usize> {
    match (acc, next) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// One applied command: its forward intent plus the memento that undoes it.
#[derive(Debug, Clone)]
pub struct AppliedEdit {
    /// The forward command (replayed on redo).
    pub forward: EditCommand,
    /// The memento that undoes it.
    pub inverse: Inverse,
}

/// A batch of applied edits that undo/redo as one user-visible step.
#[derive(Debug, Clone)]
pub struct Txn {
    /// Human-facing label.
    pub label: String,
    /// The applied edits, in application order.
    pub edits: Vec<AppliedEdit>,
}

impl Txn {
    /// A transaction with a single edit.
    #[must_use]
    pub fn single(label: impl Into<String>, edit: AppliedEdit) -> Self {
        Self {
            label: label.into(),
            edits: vec![edit],
        }
    }
}

/// The bounded undo/redo stacks (data structure only — the session drives the
/// state restoration).
#[derive(Debug, Default, Clone)]
pub struct UndoStack {
    undo: Vec<Txn>,
    redo: Vec<Txn>,
    /// Monotonic count of committed steps EVICTED past [`UNDO_CAP`] since this
    /// stack was created. A depth-based watermark (the sketch-session squash) is
    /// only valid while this is unchanged: once the bottom shifts out, a
    /// `undo_depth − watermark` count no longer addresses the session's steps.
    evicted: u64,
}

impl UndoStack {
    /// An empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an undo step is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a redo step is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Current undo depth.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Current redo depth.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// The monotonic count of committed steps evicted past [`UNDO_CAP`]. The
    /// sketch-session squash records this at enter and REFUSES the squash if it has
    /// moved since (the bottom shifted out, so the depth watermark is stale).
    #[must_use]
    pub fn evictions(&self) -> u64 {
        self.evicted
    }

    /// Records a newly committed transaction: clears the redo stack and evicts
    /// the oldest undo step past [`UNDO_CAP`].
    pub fn push_committed(&mut self, txn: Txn) {
        self.undo.push(txn);
        self.redo.clear();
        self.enforce_cap();
    }

    /// Replaces the newest `count` committed steps with `replacement` (undo-stack
    /// surgery for the sketch-session squash — see
    /// [`DocumentSession::squash_sketch_session`]). Pops `count` (clamped to the
    /// current depth) **without applying** their inverses — the document already
    /// holds the net result — then pushes `replacement` (if any), clears the redo
    /// stack (a fresh committed step), and re-caps.
    ///
    /// [`DocumentSession::squash_sketch_session`]: crate::edit::DocumentSession::squash_sketch_session
    pub fn squash(&mut self, count: usize, replacement: Option<Txn>) {
        let keep = self.undo.len().saturating_sub(count);
        self.undo.truncate(keep);
        if let Some(txn) = replacement {
            self.undo.push(txn);
        }
        self.redo.clear();
        self.enforce_cap();
    }

    /// The length of the **trailing run** (capped at `max`) of committed
    /// transactions, counting from the newest, that satisfy `pred`.
    ///
    /// Used by the sketch-session squash (B1) to decide *all-or-nothing*: it may
    /// squash the newest `count` steps ONLY when `trailing_run(count, pred) ==
    /// count` — i.e. the whole watermark→head range is one contiguous run of
    /// same-sketch edits. The first non-matching txn (e.g. an interleaved model op)
    /// stops the count, so a clamped/partial squash (which would pair a shortened
    /// pop-count with the full prior-restore inverse and corrupt undo) is refused.
    #[must_use]
    pub fn trailing_run(&self, max: usize, pred: impl Fn(&Txn) -> bool) -> usize {
        let mut run = 0;
        for txn in self.undo.iter().rev() {
            if run >= max || !pred(txn) {
                break;
            }
            run += 1;
        }
        run
    }

    /// Pops the newest undo transaction (the session applies its inverses).
    pub fn pop_for_undo(&mut self) -> Option<Txn> {
        self.undo.pop()
    }

    /// Pushes an undone transaction onto the redo stack.
    pub fn push_undone(&mut self, txn: Txn) {
        self.redo.push(txn);
    }

    /// Pops the newest redo transaction (the session replays its forwards).
    pub fn pop_for_redo(&mut self) -> Option<Txn> {
        self.redo.pop()
    }

    /// Pushes a redone transaction back onto the undo stack **without** clearing
    /// redo (redo is a navigation, not a new edit); still caps.
    pub fn push_redone(&mut self, txn: Txn) {
        self.undo.push(txn);
        self.enforce_cap();
    }

    /// Clears both stacks.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn enforce_cap(&mut self) {
        while self.undo.len() > UNDO_CAP {
            self.undo.remove(0);
            self.evicted += 1;
        }
    }
}

// ── Timeline mutation helpers (public-API-only; see `crate::document`) ────────

/// Replaces the record at `index` (record-content edits) and restores the
/// cursor. Uses only the public [`Timeline`] API (rebuild via
/// [`Timeline::from_records`]); regen states reset to `Dirty`, which is
/// irrelevant because they are not persisted.
pub(crate) fn replace_record_at(tl: &mut Timeline, index: usize, record: OperationRecord) {
    let cursor = tl.cursor();
    let mut recs = tl.records().to_vec();
    // Mementos carry the exact index they were captured at, so it is always in
    // range; assert in debug to catch a future memento bug, guard in release.
    debug_assert!(
        index < recs.len(),
        "replace_record_at: index {index} out of bounds (len {})",
        recs.len()
    );
    if index < recs.len() {
        recs[index] = record;
    }
    *tl = Timeline::from_records(recs);
    tl.set_cursor(cursor.min(tl.len()));
}

/// Re-inserts `record` at `index` and restores `cursor` (undo of a removal).
pub(crate) fn insert_record_at(
    tl: &mut Timeline,
    index: usize,
    record: OperationRecord,
    cursor: usize,
) {
    let at = index.min(tl.len());
    tl.set_cursor(at);
    tl.insert_at_cursor(record); // inserts at min(cursor, len) == `at`
    tl.set_cursor(cursor.min(tl.len()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DocumentId;
    use uuid::Uuid;

    #[test]
    fn cap_evicts_oldest_and_redo_cleared_on_new_apply() {
        let mut s = UndoStack::new();
        for i in 0..(UNDO_CAP + 5) {
            s.push_committed(Txn {
                label: format!("t{i}"),
                edits: vec![],
            });
        }
        assert_eq!(s.undo_depth(), UNDO_CAP, "capped at 200");
        // Undo one, then a new apply clears redo.
        let t = s.pop_for_undo().unwrap();
        s.push_undone(t);
        assert!(s.can_redo());
        s.push_committed(Txn {
            label: "new".into(),
            edits: vec![],
        });
        assert!(!s.can_redo(), "redo cleared on new apply");
    }

    #[test]
    fn enforce_cap_counts_evictions_monotonically() {
        let mut s = UndoStack::new();
        assert_eq!(s.evictions(), 0, "a fresh stack evicted nothing");
        // Fill exactly to the cap — no eviction yet.
        for i in 0..UNDO_CAP {
            s.push_committed(Txn {
                label: format!("t{i}"),
                edits: vec![],
            });
        }
        assert_eq!(s.evictions(), 0, "at the cap nothing is evicted");
        // Five more push the bottom out one-for-one.
        for i in 0..5 {
            s.push_committed(Txn {
                label: format!("x{i}"),
                edits: vec![],
            });
        }
        assert_eq!(s.undo_depth(), UNDO_CAP, "still capped");
        assert_eq!(
            s.evictions(),
            5,
            "five committed steps evicted the bottom five"
        );
        // redo re-push (push_redone) also enforces the cap and counts.
        let t = s.pop_for_undo().unwrap();
        s.push_undone(t);
        let t = s.pop_for_redo().unwrap();
        s.push_redone(t); // back to full — no new eviction (depth was one below cap)
        assert_eq!(
            s.evictions(),
            5,
            "a redo that does not overflow evicts nothing new"
        );
    }

    #[test]
    fn trailing_run_counts_the_contiguous_matching_suffix() {
        let mut s = UndoStack::new();
        let txn = |label: &str| Txn {
            label: label.to_string(),
            edits: vec![],
        };
        // model, sk, sk  (newest last)
        s.push_committed(txn("model"));
        s.push_committed(txn("sk1"));
        s.push_committed(txn("sk2"));
        let pred = |t: &Txn| t.label.starts_with("sk");
        assert_eq!(
            s.trailing_run(10, pred),
            2,
            "sk2 + sk1 are the trailing run"
        );
        assert_eq!(s.trailing_run(1, pred), 1, "capped at max");
        assert_eq!(s.trailing_run(0, pred), 0, "max 0 counts nothing");
        // A non-matching head breaks the run entirely.
        s.push_committed(txn("model2"));
        assert_eq!(s.trailing_run(10, pred), 0, "head is non-matching");
    }

    #[test]
    fn composite_applies_in_reverse() {
        // A composite that inserts then renames a sketch visibility flag; undo
        // must reverse the order. Here we just check apply order via visibility.
        let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
        let sid = SketchId(Uuid::from_u128(9));
        doc.set_sketch_visible(sid, false);
        let inv = Inverse::Composite(vec![
            Inverse::RestoreSketchVisibility {
                id: sid,
                prior: Some(false),
            },
            Inverse::RestoreSketchVisibility {
                id: sid,
                prior: None,
            },
        ]);
        // Reverse order: the None (remove) is applied first, then Some(false)
        // last → final state visible=false.
        inv.apply(&mut doc);
        assert!(!doc.sketch_visible(sid));
    }

    // ── H8: dirty floors ─────────────────────────────────────────────────────

    use crate::document::record::{
        BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
        PlaneKind, SketchOpParams, SketchPlaneRef,
    };
    use crate::document::refs::SketchRegionRef;
    use crate::document::variables::Scalar;
    use crate::ids::RegionId;
    use crate::math::Vec3;

    fn rec_id(seed: u128) -> RecordId {
        RecordId(Uuid::from_u128(seed))
    }

    fn extrude_rec(seed: u128, sketch: SketchId) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(10.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        }));
        OperationRecord::new(rec_id(seed), 0, "Extrude", op)
    }

    fn sketch_rec(seed: u128, sketch: SketchId) -> OperationRecord {
        let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch,
            plane: SketchPlaneRef {
                kind: PlaneKind::Xy,
                origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                x_axis: Vec3::new_unchecked(1.0, 0.0, 0.0),
                y_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
                normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
                extra: Default::default(),
            },
            entities: Vec::new(),
            constraints: Vec::new(),
            host_face: None,
            extra: Default::default(),
        }));
        OperationRecord::new(rec_id(seed), 0, "Sketch", op)
    }

    /// A 4-record document: `[sketchA, extrudeA, sketchB, extrudeB]`, cursor at the
    /// end.
    fn doc4() -> (Document, SketchId, SketchId) {
        let mut doc = Document::new(DocumentId(Uuid::from_u128(1)));
        let sa = SketchId(Uuid::from_u128(0xA));
        let sb = SketchId(Uuid::from_u128(0xB));
        for r in [
            sketch_rec(0x10, sa),
            extrude_rec(0x11, sa),
            sketch_rec(0x20, sb),
            extrude_rec(0x21, sb),
        ] {
            doc.timeline.insert_at_cursor(r);
        }
        assert_eq!(doc.timeline.cursor(), 4);
        (doc, sa, sb)
    }

    /// Every structural inverse floors at the exact step it touches — read against
    /// the PRE-undo document (`RemoveRecord` needs the record still present).
    #[test]
    fn dirty_floor_is_exact_for_the_structural_inverses() {
        let (doc, _sa, _sb) = doc4();
        assert_eq!(
            Inverse::RemoveRecord {
                record: rec_id(0x21)
            }
            .dirty_floor(&doc),
            Some(3),
            "undo of an append floors at the appended index"
        );
        assert_eq!(
            Inverse::RemoveRecord {
                record: rec_id(0x10)
            }
            .dirty_floor(&doc),
            Some(0),
        );
        assert_eq!(
            Inverse::RemoveRecord {
                // A record the timeline does not hold cannot be located — the
                // conservative default, never a guess.
                record: rec_id(0xDEAD),
            }
            .dirty_floor(&doc),
            Some(0),
            "an unlocatable record takes the conservative floor"
        );
        assert_eq!(
            Inverse::InsertRecord {
                index: 2,
                record: Box::new(sketch_rec(0x30, SketchId(Uuid::from_u128(0xC)))),
                cursor: 4,
            }
            .dirty_floor(&doc),
            Some(2),
        );
        assert_eq!(
            Inverse::RestoreRecord {
                index: 1,
                record: Box::new(extrude_rec(0x11, SketchId(Uuid::from_u128(0xA)))),
            }
            .dirty_floor(&doc),
            Some(1),
        );
    }

    /// A cursor restore dirties from the LOWER of the two positions: everything
    /// between them changes applied-ness in one direction or the other.
    #[test]
    fn dirty_floor_of_a_cursor_restore_is_the_lower_position() {
        let (mut doc, _, _) = doc4();
        assert_eq!(
            Inverse::RestoreCursor { cursor: 1 }.dirty_floor(&doc),
            Some(1),
            "undo of a roll-back: cursor 4 → 1"
        );
        doc.timeline.set_cursor(1);
        assert_eq!(
            Inverse::RestoreCursor { cursor: 4 }.dirty_floor(&doc),
            Some(1),
            "undo of a roll-forward: cursor 1 → 4 still floors at 1"
        );
    }

    /// A sketch restore floors at `min(producing Sketch op, first consumer)` — the
    /// producer matters because its own regen re-runs region detection.
    #[test]
    fn dirty_floor_of_a_sketch_restore_covers_its_producer() {
        let (doc, sa, sb) = doc4();
        assert_eq!(
            Inverse::RestoreSketch {
                id: sa,
                prior: None
            }
            .dirty_floor(&doc),
            Some(0),
            "sketch A is produced at step 0 and consumed at step 1"
        );
        assert_eq!(
            Inverse::RestoreSketch {
                id: sb,
                prior: None
            }
            .dirty_floor(&doc),
            Some(2),
            "sketch B is produced at step 2"
        );
        assert_eq!(
            Inverse::RestoreSketch {
                id: SketchId(Uuid::from_u128(0xFFFF)),
                prior: None,
            }
            .dirty_floor(&doc),
            None,
            "a sketch no timeline step produces or consumes regenerates nothing"
        );
    }

    /// The whole-collection mementos take the conservative floor; display-only
    /// state claims nothing at all.
    #[test]
    fn dirty_floor_defaults_conservatively_for_whole_collection_mementos() {
        let (doc, sa, _) = doc4();
        let conservative: Vec<Inverse> = vec![
            Inverse::RestoreBodies {
                registry: Box::new(BodyRegistry::new()),
            },
            Inverse::RestoreDatum {
                id: DatumPlaneId(Uuid::from_u128(3)),
                prior: None,
            },
            Inverse::RestoreVariables {
                table: Box::new(VariableTable::new()),
            },
            Inverse::RestoreRepair {
                state: Box::new(RepairState::new()),
            },
        ];
        for inv in conservative {
            assert_eq!(
                inv.dirty_floor(&doc),
                Some(0),
                "{inv:?} must take the conservative floor — a too-HIGH floor is a \
                 correctness bug, a too-low one is only slow"
            );
        }
        assert_eq!(
            Inverse::RestoreSketchVisibility {
                id: sa,
                prior: None,
            }
            .dirty_floor(&doc),
            None,
            "sketch visibility is display-only"
        );
        assert_eq!(Inverse::Noop.dirty_floor(&doc), None);
    }

    /// A composite takes the MINIMUM over its members (all-`None` ⇒ `None`).
    #[test]
    fn dirty_floor_of_a_composite_is_the_minimum_over_its_members() {
        let (doc, sa, _) = doc4();
        let inv = Inverse::Composite(vec![
            Inverse::RestoreRecord {
                index: 3,
                record: Box::new(extrude_rec(0x21, SketchId(Uuid::from_u128(0xB)))),
            },
            Inverse::RestoreSketchVisibility {
                id: sa,
                prior: None,
            },
            Inverse::RestoreRecord {
                index: 1,
                record: Box::new(extrude_rec(0x11, sa)),
            },
        ]);
        assert_eq!(inv.dirty_floor(&doc), Some(1), "the lowest member wins");

        // A repair fold (the VF-M8 shape) drags the whole composite to 0.
        let with_repair = Inverse::Composite(vec![
            Inverse::RestoreRepair {
                state: Box::new(RepairState::new()),
            },
            Inverse::RestoreRecord {
                index: 3,
                record: Box::new(extrude_rec(0x21, SketchId(Uuid::from_u128(0xB)))),
            },
        ]);
        assert_eq!(with_repair.dirty_floor(&doc), Some(0));

        assert_eq!(
            Inverse::Composite(vec![
                Inverse::Noop,
                Inverse::RestoreSketchVisibility {
                    id: sa,
                    prior: None,
                },
            ])
            .dirty_floor(&doc),
            None,
            "a composite of claim-free members claims nothing"
        );
        assert_eq!(Inverse::Composite(vec![]).dirty_floor(&doc), None);
    }

    /// The fold is a plain minimum with `None` meaning "no claim".
    #[test]
    fn fold_floor_takes_the_lower_claim() {
        assert_eq!(fold_floor(None, None), None);
        assert_eq!(fold_floor(None, Some(3)), Some(3));
        assert_eq!(fold_floor(Some(3), None), Some(3));
        assert_eq!(fold_floor(Some(3), Some(1)), Some(1));
        assert_eq!(fold_floor(Some(1), Some(3)), Some(1));
    }
}
