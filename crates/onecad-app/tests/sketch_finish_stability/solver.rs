use async_trait::async_trait;
use onecad_app::{
    dto::{BeginGestureDto, DragSolveDto, FinishSketchDto, SketchUpsertDto},
    worker::{wire::GestureTarget, SolverEngine, WorkerManager},
};
use onecad_core::{ids::EntityId, regen::EngineError, sketch::Sketch};
use std::sync::Mutex;

pub struct RecordedSolver {
    worker: WorkerManager,
    last: Mutex<Option<SketchUpsertDto>>,
}

impl RecordedSolver {
    pub fn new(worker: &WorkerManager) -> Self {
        Self {
            worker: worker.clone(),
            last: Mutex::new(None),
        }
    }

    pub fn last(&self) -> SketchUpsertDto {
        self.last
            .lock()
            .unwrap()
            .clone()
            .expect("worker solve receipt")
    }
}

#[async_trait]
impl SolverEngine for RecordedSolver {
    async fn sketch_upsert(&self, sketch: &Sketch) -> Result<SketchUpsertDto, EngineError> {
        let receipt = self.worker.sketch_upsert(sketch).await?;
        *self.last.lock().unwrap() = Some(receipt.clone());
        Ok(receipt)
    }

    async fn begin_gesture(
        &self,
        sketch_id: &str,
        sketch_revision: u64,
        gesture_id: u64,
        drag_point: EntityId,
        target: GestureTarget,
        solver_policy_hash: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        self.worker
            .begin_gesture(
                sketch_id,
                sketch_revision,
                gesture_id,
                drag_point,
                target,
                solver_policy_hash,
            )
            .await
    }

    async fn solve_drag(
        &self,
        gesture_id: u64,
        seq: u64,
        drag_point: EntityId,
        target: [f64; 2],
    ) -> Result<DragSolveDto, EngineError> {
        self.worker
            .solve_drag(gesture_id, seq, drag_point, target)
            .await
    }

    async fn end_gesture(
        &self,
        sketch_id: &str,
        gesture_id: u64,
        final_target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        self.worker
            .end_gesture(sketch_id, gesture_id, final_target)
            .await
    }

    async fn cancel_gesture(&self, gesture_id: u64) -> Result<(), EngineError> {
        self.worker.cancel_gesture(gesture_id).await
    }

    async fn sketch_regions(&self, sketch_id: &str) -> Result<FinishSketchDto, EngineError> {
        self.worker.sketch_regions(sketch_id).await
    }
}
