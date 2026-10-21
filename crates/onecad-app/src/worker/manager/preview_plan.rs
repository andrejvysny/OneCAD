//! `PreviewPlan` transport (SCHEMA §7.6) — the ONECAD-369 edit preview.
//!
//! The job is driven by a SPAWNED task that owns the cancel watch, so dropping
//! the caller's future (a superseded or abandoned preview) cannot orphan a full
//! replay on the kernel lane: the runtime slot cancels the token and the task
//! still sends the `cancel` frame. Crashes feed the poison circuit exactly like
//! `ExecutePlan`; successes never heal it, because a Tier A preview proves
//! nothing about the Tier B commit that crashed.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{broadcast, oneshot};

use onecad_core::regen::{CancelToken, EngineError, PlanStepEvent};
use onecad_protocol::client::{ProtocolClient, WorkerEvent};
use onecad_protocol::messages::Lane;

use super::{
    not_connected, plan_op_keys, protocol_err, retired_err, Shared, StreamEnd, WorkerLifecycle,
};
use crate::worker::{
    plan_preview_unavailable, preview_plan as wire_preview, wire, PlanPreview, PreviewPlanRequest,
    PLAN_PREVIEW_CAPABILITY,
};

/// How long a cancelled caller waits for the worker's `CANCELLED` terminal
/// before returning; the spawned task keeps draining after that.
const CANCEL_DRAIN: Duration = Duration::from_secs(2);

pub(super) async fn preview_plan(
    shared: Arc<Shared>,
    req: PreviewPlanRequest,
    cancel: CancelToken,
) -> Result<PlanPreview, EngineError> {
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    if shared.is_retired() {
        return Err(retired_err());
    }
    let op_keys = plan_op_keys(&req.plan, &shared.fingerprint());
    if let Some((key, last)) = shared.plan_circuit_open(&op_keys) {
        return Err(EngineError::Crashed {
            message: format!(
                "crash circuit breaker open (poison) — preview abandoned (key {key}; last crash: {last})"
            ),
        });
    }
    let client = shared.client().ok_or_else(not_connected)?;
    let capabilities = &client.hello().capabilities;
    if !capabilities.iter().any(|c| c == PLAN_PREVIEW_CAPABILITY) {
        return Err(plan_preview_unavailable());
    }
    super::super::endpoint_tangency::plan(&req.plan, capabilities)?;
    super::super::boolean_tools::plan(&req.plan, capabilities)?;

    let (tx, mut rx) = oneshot::channel();
    tokio::spawn(drive(shared, client, req, op_keys, cancel.clone(), tx));
    tokio::select! {
        result = &mut rx => result.unwrap_or(Err(EngineError::Cancelled)),
        () = cancel.cancelled() => {
            match tokio::time::timeout(CANCEL_DRAIN, rx).await {
                Ok(Ok(Err(error))) => Err(error),
                _ => Err(EngineError::Cancelled),
            }
        }
    }
}

async fn drive(
    shared: Arc<Shared>,
    client: Arc<ProtocolClient>,
    req: PreviewPlanRequest,
    op_keys: Vec<String>,
    cancel: CancelToken,
    tx: oneshot::Sender<Result<PlanPreview, EngineError>>,
) {
    let result = run(&shared, &client, &req, &op_keys, &cancel).await;
    let _ = tx.send(result);
}

async fn run(
    shared: &Shared,
    client: &ProtocolClient,
    req: &PreviewPlanRequest,
    op_keys: &[String],
    cancel: &CancelToken,
) -> Result<PlanPreview, EngineError> {
    let job = req.plan.job_id;
    let mut events = client.subscribe();
    // Checked again here: the token may have fired while the task was queued, and
    // a frame never sent is the cheapest cancel there is.
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let inflight = client
        .start_request(
            "PreviewPlan",
            wire_preview::preview_plan_args(req),
            Lane::Control,
        )
        .await
        .map_err(protocol_err)?;
    let id = inflight.id;
    shared.inflight.lock().unwrap().insert(job, id);
    let mut resp_fut = Box::pin(inflight.response_with_bin());
    let mut steps: Vec<PlanStepEvent> = Vec::new();
    let mut cancel_sent = false;
    let terminal = loop {
        tokio::select! {
            biased;
            ev = events.recv() => match ev {
                Ok(WorkerEvent::Event(e)) if e.id == id && e.event == "planStep" => {
                    let step = e.step_index.map_or(0, |s| s as usize);
                    match wire::parse_plan_step(&e.payload, step) {
                        Ok(s) => steps.push(s),
                        Err(message) => break StreamEnd::Local(EngineError::Protocol { message }),
                    }
                }
                Ok(_) | Err(broadcast::error::RecvError::Closed) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    break StreamEnd::Local(EngineError::Protocol {
                        message: "PreviewPlan planStep stream lagged (dropped frame)".into(),
                    });
                }
            },
            () = cancel.cancelled(), if !cancel_sent => {
                cancel_sent = true;
                let _ = client.cancel(id).await;
            }
            resp = &mut resp_fut => break StreamEnd::Resp(resp),
        }
    };
    shared.inflight.lock().unwrap().remove(&job);
    finish(shared, op_keys, req, steps, terminal)
}

fn finish(
    shared: &Shared,
    op_keys: &[String],
    req: &PreviewPlanRequest,
    steps: Vec<PlanStepEvent>,
    terminal: StreamEnd,
) -> Result<PlanPreview, EngineError> {
    match terminal {
        StreamEnd::Local(error) => Err(error),
        StreamEnd::Resp(Ok((resp, tail))) if resp.ok => {
            let sections = resp.bin.clone().unwrap_or_default();
            let result = resp.result.unwrap_or(Value::Null);
            wire_preview::parse_plan_previewed(
                &result,
                &sections,
                &tail,
                req.expected_snapshot,
                steps,
            )
            .map_err(|message| EngineError::Protocol { message })
        }
        StreamEnd::Resp(Ok((resp, _))) => Err(resp.error.as_ref().map_or_else(
            || EngineError::Protocol {
                message: "PreviewPlan resp ok=false without error".into(),
            },
            wire::map_error,
        )),
        StreamEnd::Resp(Err(proto)) => {
            let crashing = steps.len().min(op_keys.len().saturating_sub(1));
            if let Some(key) = op_keys.get(crashing) {
                if shared.record_crash(key, &proto.to_string()) {
                    shared.emit(WorkerLifecycle::CircuitOpen { key: key.clone() });
                }
            }
            Err(EngineError::Crashed {
                message: format!(
                    "worker crashed mid-preview (job {}, {} steps streamed): {proto}",
                    req.plan.job_id.0,
                    steps.len()
                ),
            })
        }
    }
}
