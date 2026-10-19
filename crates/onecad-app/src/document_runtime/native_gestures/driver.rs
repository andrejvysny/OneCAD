use super::{control::*, *};
use crate::{api::CommandCompletion, dto::*, error::ApiError, host::AppEvent, state::AppState};
use std::{future::Future, time::Duration};
use tokio::sync::{watch, Mutex};

const RPC_BUDGET: Duration = Duration::from_secs(10);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_id() -> Result<u64, ApiError> {
    allocate_id(&NEXT_ID)
}

fn allocate_id(counter: &AtomicU64) -> Result<u64, ApiError> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| refused("Native gesture identifiers exhausted"))
}

struct Driver {
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    registry: Registry,
    scheduler: crate::state::SharedScheduler,
    autosave: Arc<watch::Sender<u64>>,
    control: Arc<Control>,
    prepared: PreparedNativeGesture,
    target: wire::GestureTarget,
    scratch: bool,
    store_ahead: bool,
}

pub(crate) fn spawn(
    state: &AppState,
    prepared: PreparedNativeGesture,
    control: Arc<Control>,
    target: wire::GestureTarget,
) {
    let driver = Driver {
        runtime: state.runtime.clone(),
        registry: state.native_gestures.clone(),
        scheduler: state.scheduler.clone(),
        autosave: state.autosave_tick.clone(),
        prepared,
        control,
        target,
        scratch: false,
        store_ahead: false,
    };
    // Exactly one slot is reserved before this task starts. Caller futures never own this task.
    tokio::spawn(driver.run());
}

impl Driver {
    async fn run(mut self) {
        let result = self.execute().await;
        let result = if self.control.cancelled() {
            Ok(())
        } else {
            result
        };
        let cleanup = self.cleanup().await;
        let poisoned = cleanup.is_err();
        let result = cleanup.and(result);
        if poisoned {
            *self.control.poisoned_claim.lock().unwrap() = self.prepared.claim.take();
        } else {
            self.release_owner().await;
            self.prepared.claim.take();
            let mut registry = self.registry.lock().unwrap();
            if registry
                .as_ref()
                .is_some_and(|control| control.id == self.control.id)
            {
                registry.take();
            }
        }
        self.control.finish(result);
    }

    async fn execute(&mut self) -> Result<(), ApiError> {
        let (baseline, pending) = self.open().await?;
        let mut preview = baseline.clone();
        loop {
            let control = self.control.clone();
            let notified = control.wake.notified();
            match self.control.next() {
                Next::Cancel => return Ok(()),
                Next::Update(update) => self.update(&mut preview, &pending, update).await?,
                Next::End(end) => return self.end(&baseline, pending, end).await,
                Next::Wait => notified.await,
            }
        }
    }

    async fn check(&self) -> Result<(), ApiError> {
        if self.control.cancelled() {
            return Err(refused("Native gesture cancelled"));
        }
        {
            let guard = self.runtime.lock().await;
            guard
                .as_ref()
                .ok_or_else(|| ApiError::NoDocument("nativeGesture".into()))?
                .validate_native_gesture(&self.prepared, self.control.id)?;
        }
        let head = bounded(self.prepared.worker.gesture_worker_head()).await?;
        let scope = &self.prepared.scope;
        if head.worker_epoch != scope.worker_epoch
            || head.snapshot_id != scope.snapshot.unwrap_or(SnapshotId(0))
            || head.history_prefix_hash != scope.expected_base_hash
        {
            return Err(refused(
                "Native gesture: stale worker snapshot or history prefix",
            ));
        }
        if self.control.cancelled() {
            return Err(refused("Native gesture cancelled"));
        }
        let guard = self.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("nativeGesture".into()))?
            .validate_native_gesture(&self.prepared, self.control.id)?;
        Ok(())
    }

    async fn open(&mut self) -> Result<(Sketch, SketchUpsertDto), ApiError> {
        self.check().await?;
        let solved = bounded(self.prepared.worker.sketch_upsert(&self.prepared.before)).await?;
        self.check().await?;
        let baseline = {
            let guard = self.runtime.lock().await;
            guard
                .as_ref()
                .ok_or_else(|| ApiError::NoDocument("nativeGesture".into()))?
                .adopt_candidate_solve(&self.prepared.before, &solved)?
        };
        self.scratch = true;
        let mut ready = bounded(self.prepared.worker.begin_gesture(
            &self.prepared.scope.sketch_id.to_string(),
            solved.sketch_revision,
            self.control.id,
            self.target.entity,
            self.target,
            "",
        ))
        .await?;
        self.check().await?;
        let (baseline, pending) = validation::begin(&baseline, &solved, &ready, self.control.id)?;
        ready.positions.clone_from(&pending.solved_positions);
        ready.curves.clone_from(&pending.solved_curves);
        let guard = self.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("nativeGesture".into()))?;
        runtime.validate_native_gesture(&self.prepared, self.control.id)?;
        if self.control.cancelled() {
            return Err(refused("Native gesture cancelled"));
        }
        self.control
            .ready
            .send_replace(Some(Ok(NativeGesturePreview {
                solve: ready,
                sketch: baseline.clone(),
            })));
        Ok((baseline, pending))
    }

    async fn update(
        &self,
        preview: &mut Sketch,
        pending: &SketchUpsertDto,
        update: UpdateRequest,
    ) -> Result<(), ApiError> {
        let result = self.solve_update(preview, pending, &update).await;
        let guard = self.runtime.lock().await;
        let result = result.and_then(|checked| {
            let runtime = guard
                .as_ref()
                .ok_or_else(|| ApiError::NoDocument("nativeGesture".into()))?;
            runtime.validate_native_gesture(&self.prepared, self.control.id)?;
            if self.control.cancelled() {
                return Err(refused("Native gesture cancelled"));
            }
            Ok(checked)
        });
        if let Ok(checked) = &result {
            *preview = checked.sketch.clone();
        }
        let error = result.as_ref().err().cloned();
        let _ = update.reply.send(result.map(Some));
        error.map_or(Ok(()), Err)
    }

    async fn solve_update(
        &self,
        baseline: &Sketch,
        pending: &SketchUpsertDto,
        update: &UpdateRequest,
    ) -> Result<NativeGesturePreview<DragSolveDto>, ApiError> {
        self.check().await?;
        let solved = bounded(self.prepared.worker.solve_drag(
            self.control.id,
            update.seq,
            self.target.entity,
            update.target,
        ))
        .await?;
        self.check().await?;
        let sketch = validation::update(baseline, pending, &solved, self.control.id, update.seq)?;
        Ok(NativeGesturePreview {
            solve: solved,
            sketch,
        })
    }

    async fn end(
        &mut self,
        baseline: &Sketch,
        pending: SketchUpsertDto,
        end: EndRequest,
    ) -> Result<(), ApiError> {
        let result = self.finish(baseline, pending, &end).await;
        let error = result.as_ref().err().cloned();
        let _ = end.reply.send(result);
        error.map_or(Ok(()), Err)
    }

    async fn finish(
        &mut self,
        baseline: &Sketch,
        pending: SketchUpsertDto,
        end: &EndRequest,
    ) -> Result<EndCompletion, ApiError> {
        self.check().await?;
        self.store_ahead = true;
        let solved = bounded(self.prepared.worker.end_gesture(
            &self.prepared.scope.sketch_id.to_string(),
            self.control.id,
            end.target,
        ))
        .await?;
        self.scratch = false;
        self.check().await?;
        let (solve, outcome, projection) = {
            let mut guard = self.runtime.lock().await;
            if self.control.cancelled() {
                return Err(refused("Native gesture cancelled"));
            }
            let runtime = guard
                .as_mut()
                .ok_or_else(|| ApiError::NoDocument("nativeGesture".into()))?;
            let (solve, outcome) = runtime.commit_native_gesture(
                &self.prepared,
                self.control.id,
                baseline,
                solved,
                pending,
            )?;
            (solve, outcome, runtime.projection())
        };
        self.store_ahead = false;
        let moved = outcome.is_some();
        let regen_expected = self.publish(&end.host, &solve, &projection, outcome);
        Ok(CommandCompletion {
            result: Ok(self.control.receipt(NativeGestureEnd {
                solve,
                moved,
                cleanup_error: None,
            })),
            projection,
            regen_expected,
        })
    }

    fn publish(
        &self,
        host: &crate::host::HostContext,
        solve: &SketchUpsertDto,
        projection: &DocumentProjection,
        outcome: Option<CommandOutcome>,
    ) -> bool {
        let _ = host.emit(AppEvent::SketchSolved(solve.clone()));
        let _ = host.emit(AppEvent::ProjectionUpdated(projection.clone()));
        let Some(outcome) = outcome else {
            return false;
        };
        self.autosave
            .send_modify(|tick| *tick = tick.wrapping_add(1));
        if let Some(scheduler) = self.scheduler.get() {
            scheduler.handle(&outcome);
            return outcome.regen != onecad_core::edit::RegenHint::None;
        }
        false
    }

    async fn cleanup(&mut self) -> Result<(), ApiError> {
        let worker = &self.prepared.worker;
        if !worker.is_current() {
            return self.quarantine().await;
        }
        if self.scratch
            && bounded(worker.cancel_gesture(self.control.id))
                .await
                .is_err()
        {
            return self.quarantine().await;
        }
        if self.store_ahead && self.reseed().await.is_err() {
            return self.quarantine().await;
        }
        Ok(())
    }

    async fn reseed(&self) -> Result<(), ApiError> {
        let (sketch, revision, geometry) = {
            let guard = self.runtime.lock().await;
            let runtime = guard
                .as_ref()
                .ok_or_else(|| ApiError::NoDocument("nativeGesture cleanup".into()))?;
            if runtime.runtime_session() != self.prepared.scope.runtime_session {
                return Err(refused(
                    "Native gesture cleanup belongs to a replaced document",
                ));
            }
            let sketch =
                runtime.sketch_or_err(self.prepared.scope.sketch_id, "Native gesture cleanup")?;
            let geometry = sketch_geometry_token(&sketch);
            (sketch, runtime.revision(), geometry)
        };
        bounded(self.prepared.worker.sketch_upsert(&sketch)).await?;
        let guard = self.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("nativeGesture cleanup".into()))?;
        if runtime.runtime_session() != self.prepared.scope.runtime_session
            || runtime.revision() != revision
            || !self.prepared.worker.is_current()
            || sketch_geometry_token(&runtime.sketch_or_err(sketch.id, "Native gesture cleanup")?)
                != geometry
        {
            return Err(refused("Native gesture cleanup was superseded"));
        }
        // Reseeding is a worker-cache repair, never a document geometry adoption.
        Ok(())
    }

    async fn quarantine(&self) -> Result<(), ApiError> {
        if self.prepared.worker.quarantine().await {
            Ok(())
        } else {
            Err(ApiError::Worker(
                "Native gesture cleanup failed; solver lane remains quarantined".into(),
            ))
        }
    }

    async fn release_owner(&self) {
        let mut guard = self.runtime.lock().await;
        if let Some(runtime) = guard.as_mut() {
            if runtime
                .native_gesture
                .as_ref()
                .is_some_and(|owner| owner.id == self.control.id)
            {
                runtime.native_gesture.take();
            }
        }
    }
}

async fn bounded<T>(future: impl Future<Output = Result<T, EngineError>>) -> Result<T, ApiError> {
    tokio::time::timeout(RPC_BUDGET, future)
        .await
        .map_err(|_| ApiError::Worker("Native gesture worker deadline exceeded".into()))?
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gesture_identifier_exhaustion_never_wraps_or_reuses() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_id(&counter).unwrap(), u64::MAX - 1);
        assert!(allocate_id(&counter).is_err());
        assert!(allocate_id(&counter).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
