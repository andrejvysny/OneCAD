use super::*;
use onecad_core::regen::*;
use tokio::sync::mpsc;

pub(super) struct EpochBackend {
    pub epoch: AtomicU64,
}
impl EpochBackend {
    fn not_ready() -> EngineError {
        op_failed("unused geometry engine")
    }
}

#[async_trait]
impl GeometryEngine for EpochBackend {
    fn current_epoch(&self) -> WorkerEpoch {
        WorkerEpoch(self.epoch.load(Ordering::SeqCst))
    }
    async fn execute_plan(&self, _request: PlanRequest) -> mpsc::Receiver<PlanEvent> {
        let (tx, rx) = mpsc::channel(1);
        let _ = tx.send(PlanEvent::Failed(Self::not_ready())).await;
        rx
    }
    async fn open_session(&self, _req: OpenSessionRequest) -> Result<WorkerHead, EngineError> {
        Err(Self::not_ready())
    }
    async fn close_session(&self, _d: DocumentId, _e: WorkerEpoch) -> Result<(), EngineError> {
        Ok(())
    }
    async fn reset(&self, _d: DocumentId, e: WorkerEpoch) -> Result<WorkerEpoch, EngineError> {
        Ok(WorkerEpoch(e.0 + 1))
    }
    async fn accept_prepared(&self, _j: JobId, _f: Fencing) -> Result<AcceptResult, EngineError> {
        Err(Self::not_ready())
    }
    async fn discard_prepared(&self, _j: JobId) -> Result<(), EngineError> {
        Ok(())
    }
    async fn get_worker_head(&self) -> Result<WorkerHead, EngineError> {
        Err(Self::not_ready())
    }
    async fn tessellate(&self, _r: TessellateRequest) -> Result<TessellateResult, EngineError> {
        Err(Self::not_ready())
    }
    async fn save_checkpoint(&self, _s: usize) -> Result<CheckpointArtifacts, EngineError> {
        Err(Self::not_ready())
    }
    async fn restore_checkpoint(&self, _r: RestoreRequest) -> Result<RestoreResult, EngineError> {
        Err(Self::not_ready())
    }
    async fn acquire_element_ids(
        &self,
        _r: AcquireRequest,
    ) -> Result<Vec<WorkerElementEvidence>, EngineError> {
        Err(Self::not_ready())
    }
    async fn bind_element_ids(&self, _r: BindElementIdsRequest) -> Result<(), EngineError> {
        Err(Self::not_ready())
    }
    async fn resolve_refs(&self, _r: ResolveRequest) -> Result<Vec<RefResolution>, EngineError> {
        Err(Self::not_ready())
    }
    async fn cancel(&self, _j: JobId) -> Result<(), EngineError> {
        Ok(())
    }
    async fn ping(&self) -> Result<(), EngineError> {
        Err(Self::not_ready())
    }
}
