//! Gesture RPCs pin the concrete connection rather than re-resolving the live manager.
use super::*;
use crate::worker::{NativeGestureWorker, SolverEngine};

struct BoundGestureClient {
    manager: WorkerManager,
    client: Arc<ProtocolClient>,
    epoch: WorkerEpoch,
}

pub(super) fn bind(manager: &WorkerManager) -> Result<Arc<dyn NativeGestureWorker>, EngineError> {
    let connection = manager.shared.connection.read().unwrap();
    let client = connection
        .client
        .as_ref()
        .cloned()
        .ok_or_else(not_connected)?;
    if connection.state != WorkerState::Ready || client.is_closed() {
        return Err(not_connected());
    }
    if manager.shared.is_retired() {
        return Err(retired_err());
    }
    Ok(Arc::new(BoundGestureClient {
        manager: manager.clone(),
        client,
        epoch: WorkerEpoch(connection.epoch),
    }))
}

impl BoundGestureClient {
    async fn request(&self, verb: &str, args: Value) -> Result<Value, EngineError> {
        if !self.is_current() {
            return Err(protocol("Native gesture connection was replaced"));
        }
        let response = if verb == "SketchUpsert" {
            self.manager
                .shared
                .solver_round_trip(&self.client, verb, args)
                .await?
                .0
        } else {
            self.client
                .request(verb, args)
                .await
                .map_err(protocol_err)?
        };
        let result = ok_result(response)?;
        if !self.is_current() {
            return Err(protocol("Native gesture connection was replaced"));
        }
        Ok(result)
    }
}

#[async_trait]
impl NativeGestureWorker for BoundGestureClient {
    async fn gesture_worker_head(&self) -> Result<WorkerHead, EngineError> {
        let result = self.request("GetWorkerHead", json!({})).await?;
        if ["documentRevision", "workerEpoch", "snapshotId"]
            .into_iter()
            .any(|key| result.get(key).and_then(Value::as_u64).is_none())
            || result
                .get("historyPrefixHash")
                .and_then(Value::as_str)
                .is_none()
        {
            return Err(protocol("Native gesture: malformed worker head"));
        }
        Ok(wire::parse_worker_head(&result))
    }

    fn is_current(&self) -> bool {
        !self.manager.shared.is_retired()
            && self.manager.epoch() == self.epoch
            && self
                .manager
                .shared
                .client()
                .is_some_and(|client| Arc::ptr_eq(&client, &self.client))
    }

    async fn quarantine(&self) -> bool {
        {
            let mut connection = self.manager.shared.connection.write().unwrap();
            if connection.epoch != self.epoch.0
                || !connection
                    .client
                    .as_ref()
                    .is_some_and(|client| Arc::ptr_eq(client, &self.client))
            {
                return true;
            }
            // Holding the connection guard closes the check/retire replacement race.
            self.manager.shared.latch_retired();
            connection.state = WorkerState::Retired;
            self.manager.shared.retire_signal.notify_waiters();
        }
        self.manager.wait_torn_down(Duration::from_secs(5)).await
    }
}

#[async_trait]
impl SolverEngine for BoundGestureClient {
    async fn sketch_upsert(
        &self,
        sketch: &onecad_core::sketch::Sketch,
    ) -> Result<SketchUpsertDto, EngineError> {
        super::super::endpoint_tangency::sketch(sketch, &self.client.hello().capabilities)?;
        let result = self
            .request("SketchUpsert", wire::sketch_upsert_args(sketch))
            .await?;
        wire::parse_sketch_upsert_checked(sketch, &result).map_err(|message| protocol(&message))
    }

    async fn begin_gesture(
        &self,
        sketch_id: &str,
        revision: u64,
        id: u64,
        point: EntityId,
        target: wire::GestureTarget,
        policy: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        let result = self
            .request(
                "BeginGesture",
                wire::begin_gesture_args(sketch_id, revision, id, point, &target, policy),
            )
            .await?;
        super::super::gesture_result::begin(id, &result).map_err(|message| protocol(&message))
    }

    async fn solve_drag(
        &self,
        id: u64,
        seq: u64,
        point: EntityId,
        target: [f64; 2],
    ) -> Result<DragSolveDto, EngineError> {
        let result = self
            .request("SolveDrag", wire::solve_drag_args(id, seq, point, target))
            .await?;
        check_channels(&result)?;
        Ok(wire::parse_solve_drag(&result))
    }

    async fn end_gesture(
        &self,
        sketch_id: &str,
        id: u64,
        target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        let result = self
            .request("EndGesture", wire::end_gesture_args(id, target))
            .await?;
        if result.get("gestureId").and_then(Value::as_u64) != Some(id) {
            return Err(protocol("Native EndGesture: invalid gesture receipt"));
        }
        check_channels(&result)?;
        wire::parse_sketch_upsert_truth_checked(sketch_id, &result)
            .map_err(|message| protocol(&message))
    }

    async fn cancel_gesture(&self, id: u64) -> Result<(), EngineError> {
        // Cleanup may address the captured old client, but can never address a replacement.
        let result = ok_result(
            self.client
                .request("EndGesture", json!({"gestureId":id,"cancel":true}))
                .await
                .map_err(protocol_err)?,
        )?;
        if result.get("gestureId").and_then(Value::as_u64) != Some(id)
            || result.get("cancelled").and_then(Value::as_bool) != Some(true)
        {
            return Err(protocol("Native EndGesture: invalid cancellation receipt"));
        }
        Ok(())
    }

    async fn sketch_regions(&self, _sketch_id: &str) -> Result<FinishSketchDto, EngineError> {
        Err(protocol(
            "Bound native gesture connection does not own profile queries",
        ))
    }
}

fn check_channels(result: &Value) -> Result<(), EngineError> {
    let mut checked = result.clone();
    let object = checked
        .as_object_mut()
        .ok_or_else(|| protocol("Native gesture result must be an object"))?;
    object.insert("gestureId".into(), json!(1));
    object.insert("ready".into(), json!(true));
    super::super::gesture_result::begin(1, &checked).map_err(|message| protocol(&message))?;
    Ok(())
}
