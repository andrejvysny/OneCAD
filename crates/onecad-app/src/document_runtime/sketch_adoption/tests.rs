use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::dto::{BeginGestureDto, DragSolveDto};
use crate::worker::PendingBackend;
use onecad_core::sketch::{SketchEntity, WorldPlane};
mod constraints;
mod epoch_backend;
mod history;
mod region_race;
mod truth;
use epoch_backend::EpochBackend;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Correct,
    Unchanged,
    Foreign,
    Nonfinite,
    Failure,
    StaleRevision,
    StaleEpoch,
    ReplayedRevision,
    WorkerRestart,
    EndMalformedTruth,
    OverConstrained,
    Conflicting,
    NonfiniteTruth,
    Unassessed,
    Pending,
}

struct Solver {
    mode: Mutex<Mode>,
    calls: AtomicU64,
    fencing: Mutex<Option<Arc<FencingCell>>>,
    regions_fail: Mutex<bool>,
    backend: Arc<EpochBackend>,
}

fn entity() -> EntityId {
    EntityId(Uuid::from_u128(20))
}
fn sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(10))
}

fn response(id: SketchId, revision: u64) -> SketchUpsertDto {
    SketchUpsertDto {
        truth: Default::default(),
        sketch_id: id.to_string(),
        sketch_revision: revision,
        dof: 2,
        status: SketchSolveStatus::UnderConstrained,
        conflicting: Vec::new(),
        solved_positions: BTreeMap::new(),
        solved_curves: BTreeMap::new(),
        entity_states: BTreeMap::new(),
    }
}

#[async_trait]
impl SolverEngine for Solver {
    async fn sketch_upsert(&self, sketch: &Sketch) -> Result<SketchUpsertDto, EngineError> {
        let revision = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mut solved = response(sketch.id, revision);
        let mode = *self.mode.lock().unwrap();
        match mode {
            Mode::Correct => {
                solved
                    .solved_positions
                    .insert(entity().to_string(), [1.0, 0.0]);
            }
            Mode::Foreign => solved.sketch_id = "foreign".into(),
            Mode::Nonfinite => {
                solved
                    .solved_positions
                    .insert(entity().to_string(), [f64::NAN, 0.0]);
            }
            Mode::Failure => return Err(op_failed("injected transport failure")),
            Mode::StaleRevision => self
                .fencing
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .bump_revision(),
            Mode::StaleEpoch => self.fencing.lock().unwrap().as_ref().unwrap().set_epoch(2),
            Mode::WorkerRestart => self.backend.epoch.store(2, Ordering::SeqCst),
            Mode::ReplayedRevision => solved.sketch_revision = 1,
            Mode::OverConstrained => solved.status = SketchSolveStatus::OverConstrained,
            Mode::Conflicting => solved.status = SketchSolveStatus::Conflicting,
            Mode::NonfiniteTruth => solved.truth.normalized_max_residual = Some(f64::NAN),
            Mode::Unassessed => solved.truth.unassessed.push("unassessed".into()),
            Mode::Pending => std::future::pending::<()>().await,
            Mode::Unchanged | Mode::EndMalformedTruth => {}
        }
        Ok(solved)
    }
    async fn begin_gesture(
        &self,
        _: &str,
        _: u64,
        gesture_id: u64,
        _: EntityId,
        _: wire::GestureTarget,
        _: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        Ok(BeginGestureDto {
            gesture_id,
            ready: true,
            point_offset: None,
            positions: Default::default(),
            curves: Default::default(),
            entity_states: BTreeMap::new(),
        })
    }
    async fn solve_drag(
        &self,
        _: u64,
        _: u64,
        _: EntityId,
        _: [f64; 2],
    ) -> Result<DragSolveDto, EngineError> {
        Err(op_failed("unused drag"))
    }
    async fn end_gesture(
        &self,
        id: &str,
        _: u64,
        _: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        let mut solved = response(
            SketchId::from_str(id).unwrap(),
            self.calls.load(Ordering::SeqCst),
        );
        if *self.mode.lock().unwrap() == Mode::EndMalformedTruth {
            solved.truth.normalized_max_residual = Some(f64::NAN);
            solved
                .solved_positions
                .insert(entity().to_string(), [99.0, 99.0]);
        }
        Ok(solved)
    }
    async fn cancel_gesture(&self, _: u64) -> Result<(), EngineError> {
        Ok(())
    }

    async fn sketch_regions(&self, _: &str) -> Result<FinishSketchDto, EngineError> {
        if *self.regions_fail.lock().unwrap() {
            return Err(op_failed("injected regions refusal"));
        }
        Ok(FinishSketchDto {
            region_identity_version: 3,
            regions: Vec::new(),
            diagnostics: Vec::new(),
        })
    }
}

fn runtime(mode: Mode) -> (DocumentRuntime, Arc<Solver>) {
    let backend = Arc::new(EpochBackend {
        epoch: AtomicU64::new(1),
    });
    let solver = Arc::new(Solver {
        mode: Mutex::new(mode),
        calls: AtomicU64::new(0),
        fencing: Mutex::new(None),
        regions_fail: Mutex::new(false),
        backend: backend.clone(),
    });
    let mut runtime = DocumentRuntime::new_blank(backend, Arc::new(PendingBackend), solver.clone());
    let mut sketch = Sketch::on_world_plane(sketch_id(), "test", WorldPlane::XY);
    sketch
        .add_entity(SketchEntity::point(
            entity(),
            Vec2::new(1.0, 1.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    *solver.fencing.lock().unwrap() = Some(runtime.fencing());
    (runtime, solver)
}

fn moved_ops() -> Vec<SketchEditOp> {
    vec![SketchEditOp::SetEntityPositions {
        positions: vec![(entity(), Vec2::new(2.0, 3.0).unwrap())],
    }]
}

#[tokio::test]
async fn edit_and_correction_are_one_revision_and_one_undo() {
    let (mut runtime, _) = runtime(Mode::Correct);
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    let revision = runtime.revision();
    let depth = runtime.undo_depth();
    let (_, outcome) = runtime
        .sketch_upsert_with_outcome(sketch_id(), moved_ops())
        .await
        .unwrap();
    assert!(outcome
        .unwrap()
        .projection_delta
        .sketches
        .contains(&sketch_id()));
    assert_eq!(runtime.revision().0, revision.0 + 1);
    assert_eq!(runtime.undo_depth(), depth + 1);
    let after = runtime.get_sketch(sketch_id()).unwrap().entities;
    runtime.undo().unwrap();
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, after);
}

#[tokio::test]
async fn failed_stale_or_foreign_edits_leave_geometry_and_history_unchanged() {
    for mode in [
        Mode::Failure,
        Mode::Foreign,
        Mode::Nonfinite,
        Mode::StaleRevision,
        Mode::StaleEpoch,
        Mode::WorkerRestart,
    ] {
        let (mut runtime, _) = runtime(mode);
        let before = runtime.get_sketch(sketch_id()).unwrap().entities;
        let depth = runtime.undo_depth();
        assert!(runtime
            .sketch_upsert_with_outcome(sketch_id(), moved_ops())
            .await
            .is_err());
        assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
        assert_eq!(runtime.undo_depth(), depth);
        assert_eq!(runtime.redo_depth(), 0);
    }
}

#[tokio::test]
async fn changed_refresh_owns_resolve_entry_unchanged_refresh_preserves_redo() {
    let (mut runtime, _) = runtime(Mode::Correct);
    let depth = runtime.undo_depth();
    let (_, outcome) = runtime
        .sketch_upsert_with_outcome(sketch_id(), Vec::new())
        .await
        .unwrap();
    assert!(outcome.is_some());
    assert_eq!(runtime.session.undo_label(), Some("Resolve sketch"));
    assert_eq!(runtime.undo_depth(), depth + 1);
    runtime.undo().unwrap();
    let (mut unchanged, _) = self::runtime(Mode::Unchanged);
    unchanged
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: moved_ops(),
        })
        .unwrap();
    unchanged.undo().unwrap();
    let state = (
        unchanged.undo_depth(),
        unchanged.redo_depth(),
        unchanged.revision(),
    );
    assert!(unchanged
        .sketch_upsert_with_outcome(sketch_id(), Vec::new())
        .await
        .unwrap()
        .1
        .is_none());
    assert_eq!(
        (
            unchanged.undo_depth(),
            unchanged.redo_depth(),
            unchanged.revision()
        ),
        state
    );
}

#[tokio::test]
async fn entry_watermark_covers_correction_and_failed_entry_preserves_session() {
    let (mut runtime, solver) = runtime(Mode::Correct);
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    let (dto, outcome) = runtime
        .enter_sketch_with_outcome(sketch_id())
        .await
        .unwrap();
    assert!(outcome.is_some());
    assert_eq!(
        dto.entities,
        runtime.get_sketch(sketch_id()).unwrap().entities
    );
    let watermark = runtime.sketch_session.as_ref().unwrap().undo_watermark;
    *solver.mode.lock().unwrap() = Mode::Failure;
    assert!(runtime.enter_sketch(sketch_id()).await.is_err());
    assert_eq!(
        runtime.sketch_session.as_ref().unwrap().undo_watermark,
        watermark
    );
    let cancelled = runtime.cancel_sketch(sketch_id(), true).await.unwrap();
    assert!(cancelled.discarded);
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
}

#[tokio::test]
async fn read_only_entry_refuses_correction_but_allows_unchanged_read() {
    let (mut runtime, solver) = runtime(Mode::Correct);
    runtime.read_only = true;
    assert!(runtime.enter_sketch(sketch_id()).await.is_err());
    assert!(runtime.sketch_session.is_none());
    *solver.mode.lock().unwrap() = Mode::Unchanged;
    assert!(runtime
        .enter_sketch_with_outcome(sketch_id())
        .await
        .unwrap()
        .1
        .is_none());
}

#[tokio::test]
async fn active_gesture_refuses_before_authored_mutation() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    runtime
        .begin_gesture(sketch_id(), entity(), wire::GestureTarget::point(entity()))
        .await
        .unwrap();
    assert!(runtime.enter_sketch(sketch_id()).await.is_err());
    assert!(runtime
        .sketch_upsert_with_outcome(sketch_id(), moved_ops())
        .await
        .is_err());
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
    runtime
        .finish_sketch_with_outcome(sketch_id())
        .await
        .unwrap();
    assert!(runtime.active_gesture.is_none());
}

#[tokio::test]
async fn region_refusal_carries_committed_adoption_and_record_outcome() {
    let (mut runtime, solver) = runtime(Mode::Correct);
    *solver.regions_fail.lock().unwrap() = true;
    let error = runtime
        .finish_sketch_with_outcome(sketch_id())
        .await
        .unwrap_err();
    let outcome = error.committed.unwrap();
    assert!(outcome.projection_delta.timeline_changed);
    assert!(outcome.projection_delta.sketches.contains(&sketch_id()));
    assert!(runtime.get_sketch(sketch_id()).unwrap().solve_current);
}

#[tokio::test]
async fn replayed_solver_revision_preserves_geometry_history_and_redo() {
    let (mut runtime, solver) = runtime(Mode::Correct);
    runtime
        .sketch_upsert_with_outcome(sketch_id(), Vec::new())
        .await
        .unwrap();
    runtime
        .apply(EditCommand::SketchEdit {
            sketch: sketch_id(),
            ops: moved_ops(),
        })
        .unwrap();
    runtime.undo().unwrap();
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    let state = (
        runtime.revision(),
        runtime.undo_depth(),
        runtime.redo_depth(),
    );
    *solver.mode.lock().unwrap() = Mode::ReplayedRevision;
    assert!(runtime
        .sketch_upsert_with_outcome(sketch_id(), moved_ops())
        .await
        .is_err());
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
    assert_eq!(
        (
            runtime.revision(),
            runtime.undo_depth(),
            runtime.redo_depth()
        ),
        state
    );
    solver.backend.epoch.store(2, Ordering::SeqCst);
    runtime.adopt_current_epoch();
    assert!(runtime
        .sketch_upsert_with_outcome(sketch_id(), Vec::new())
        .await
        .unwrap()
        .1
        .is_none());
}

#[tokio::test]
async fn foreign_sketch_gesture_blocks_finish_without_draining_it() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    let other = SketchId(Uuid::from_u128(11));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(other, "other", WorldPlane::XY),
        })
        .unwrap();
    runtime
        .begin_gesture(sketch_id(), entity(), wire::GestureTarget::point(entity()))
        .await
        .unwrap();
    let gesture = runtime.active_gesture.as_ref().unwrap().gesture_id;
    assert!(runtime.finish_sketch_with_outcome(other).await.is_err());
    assert_eq!(runtime.active_gesture.as_ref().unwrap().gesture_id, gesture);
}

#[test]
fn final_curve_members_override_pending_members_without_dropping_others() {
    use crate::dto::CurveParamsDto;
    let mut begin = response(sketch_id(), 1);
    begin
        .solved_positions
        .insert(entity().to_string(), [1.0, 0.0]);
    begin.solved_curves.insert(
        "curve".into(),
        CurveParamsDto {
            radius: Some(5.0),
            start_angle: Some(0.5),
            end_angle: None,
        },
    );
    let mut end = response(sketch_id(), 1);
    end.solved_positions
        .insert(entity().to_string(), [2.0, 0.0]);
    end.solved_curves.insert(
        "curve".into(),
        CurveParamsDto {
            radius: None,
            start_angle: Some(1.0),
            end_angle: Some(2.0),
        },
    );
    merge_pending_solve(&mut end, begin);
    assert_eq!(end.solved_positions[&entity().to_string()], [2.0, 0.0]);
    assert_eq!(end.solved_curves["curve"].radius, Some(5.0));
    assert_eq!(end.solved_curves["curve"].start_angle, Some(1.0));
    assert_eq!(end.solved_curves["curve"].end_angle, Some(2.0));
}
