//! Wire lowering and terminal validation for `PreviewPlan` (SCHEMA §7.6).
//!
//! The op list is lowered by [`wire::execute_plan_args`] verbatim, so a preview
//! and the commit it predicts can never be built by two mappers. Only the keys a
//! preview must not carry are removed and its read-only fence added.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{json, Value};

use onecad_core::ids::{BodyId, SnapshotId};
use onecad_core::regen::{PlanStepEvent, PreparedMeshRef, StepStatus, StoppedReason};
use onecad_protocol::messages::BinSection;

use super::{lod_from_str, lod_str, wire, PlanPreview, PreviewPlanRequest};

/// Keys `ExecutePlan` sends that a preview must not: it never fences on the head
/// hash, never claims the restored base, ships no artifacts, and is an ordinary
/// (never an F12 fallback) replay.
const EXECUTE_ONLY_KEYS: [&str; 5] = [
    "documentRevision",
    "expectedBaseHash",
    "baseCheckpoint",
    "artifacts",
    "checkpointFallbackReplay",
];

/// `PreviewPlan.args` for `req`.
#[must_use]
pub(crate) fn preview_plan_args(req: &PreviewPlanRequest) -> Value {
    let mut args = wire::execute_plan_args(&req.plan);
    if let Value::Object(map) = &mut args {
        for key in EXECUTE_ONLY_KEYS {
            map.remove(key);
        }
        map.insert("expectedSnapshotId".into(), json!(req.expected_snapshot.0));
        map.insert("lod".into(), json!(lod_str(req.lod)));
    }
    args
}

/// Validates a `planPreviewed` terminal against the request and the streamed
/// steps. Any defect fails the whole preview: unlike the advisory `ExecutePlan`
/// artifacts there is no pull to fall back to.
///
/// # Errors
/// A human reason naming the first defect.
pub(crate) fn parse_plan_previewed(
    result: &Value,
    sections: &[BinSection],
    tail: &[u8],
    expected: SnapshotId,
    steps: Vec<PlanStepEvent>,
) -> Result<PlanPreview, String> {
    if result.get("planPreviewed") != Some(&Value::Bool(true)) {
        return Err("PreviewPlan: missing planPreviewed".into());
    }
    let snapshot = result.get("snapshotId").and_then(Value::as_u64);
    if snapshot != Some(expected.0) {
        return Err(format!(
            "PreviewPlan: snapshotId {snapshot:?} is not the fenced head {}",
            expected.0
        ));
    }
    let last_valid_step = parse_last_valid(result.get("lastValidStep"))?;
    let stopped_reason = parse_stopped(result.get("stoppedReason"))?;
    let per_step = wire::parse_per_step(result.get("perStepResults"))?;
    validate_step_summary(&per_step, last_valid_step, stopped_reason)?;
    validate_step_events(&per_step, &steps)?;
    let bodies = parse_bodies(result.get("bodies"))?;
    let meshes = parse_meshes(result.get("meshes"), sections, tail, expected, &bodies)?;
    Ok(PlanPreview {
        snapshot_id: expected,
        last_valid_step,
        stopped_reason,
        per_step,
        steps,
        bodies,
        meshes,
    })
}

fn parse_last_valid(value: Option<&Value>) -> Result<Option<usize>, String> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| "PreviewPlan: invalid lastValidStep".into()),
        _ => Err("PreviewPlan: missing/invalid lastValidStep".into()),
    }
}

fn parse_stopped(value: Option<&Value>) -> Result<StoppedReason, String> {
    match value.and_then(Value::as_str) {
        Some("completed") => Ok(StoppedReason::Completed),
        Some("opFailed") => Ok(StoppedReason::OpFailed),
        Some("needsRepair") => Ok(StoppedReason::NeedsRepair),
        other => Err(format!("PreviewPlan: invalid stoppedReason {other:?}")),
    }
}

/// The §7.2 shape: every step Ok except possibly the last, which then names the
/// stop reason; `lastValidStep` is the last Ok step.
fn validate_step_summary(
    per_step: &[onecad_core::regen::StepResult],
    last_valid: Option<usize>,
    stopped: StoppedReason,
) -> Result<(), String> {
    let (stop, body) = match per_step.split_last() {
        Some((last, rest)) if last.status != StepStatus::Ok => (Some(last), rest),
        _ => (None, per_step),
    };
    if body.iter().any(|s| s.status != StepStatus::Ok) {
        return Err("PreviewPlan: a non-final step is not ok".into());
    }
    let expected_stop = match stop.map(|s| s.status) {
        None => StoppedReason::Completed,
        Some(StepStatus::OpFailed) => StoppedReason::OpFailed,
        Some(_) => StoppedReason::NeedsRepair,
    };
    if expected_stop != stopped {
        return Err("PreviewPlan: stoppedReason disagrees with perStepResults".into());
    }
    if body.last().map(|s| s.step_index) != last_valid {
        return Err("PreviewPlan: lastValidStep disagrees with perStepResults".into());
    }
    Ok(())
}

/// The streamed `planStep`s are the preview == commit evidence and carry the
/// edited step's diagnostics, so a dropped, repeated or reordered one is a
/// protocol defect, not short evidence. Every Ok step emits exactly one, in
/// order; the stopping step MAY emit one (a NeedsRepair step and a §3.4
/// substituted event do, a plain op failure does not).
fn validate_step_events(
    per_step: &[onecad_core::regen::StepResult],
    steps: &[PlanStepEvent],
) -> Result<(), String> {
    let ok: Vec<usize> = per_step
        .iter()
        .filter(|s| s.status == StepStatus::Ok)
        .map(|s| s.step_index)
        .collect();
    let streamed: Vec<usize> = steps.iter().map(|s| s.step_index).collect();
    let stop = per_step
        .last()
        .filter(|s| s.status != StepStatus::Ok)
        .map(|s| s.step_index);
    let with_stop = stop.map(|i| [ok.as_slice(), &[i]].concat());
    if streamed == ok || with_stop.is_some_and(|expected| streamed == expected) {
        Ok(())
    } else {
        Err(format!(
            "PreviewPlan: planStep events {streamed:?} disagree with perStepResults (ok steps {ok:?})"
        ))
    }
}

fn parse_bodies(value: Option<&Value>) -> Result<Vec<BodyId>, String> {
    let items = value
        .and_then(Value::as_array)
        .ok_or("PreviewPlan: missing bodies array")?;
    let mut seen = HashSet::with_capacity(items.len());
    items
        .iter()
        .map(|item| {
            let id = item
                .as_str()
                .ok_or("PreviewPlan: bodies holds a non-string")?;
            let body = wire::parse_body_id(id)?;
            if !seen.insert(body) {
                return Err(format!("PreviewPlan: duplicate body {id:?}"));
            }
            Ok(body)
        })
        .collect()
}

fn parse_meshes(
    value: Option<&Value>,
    sections: &[BinSection],
    tail: &[u8],
    expected: SnapshotId,
    bodies: &[BodyId],
) -> Result<Vec<PreparedMeshRef>, String> {
    let handles = value
        .and_then(Value::as_array)
        .ok_or("PreviewPlan: missing meshes array")?;
    let by_name = wire::validate_bin_sections("PreviewPlan", sections, tail)?;
    let known: HashSet<BodyId> = bodies.iter().copied().collect();
    let mut referenced = HashSet::with_capacity(handles.len());
    let mut meshed = HashSet::with_capacity(handles.len());
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        let body = handle
            .get("bodyId")
            .and_then(Value::as_str)
            .ok_or("PreviewPlan: mesh handle missing bodyId")
            .and_then(|id| wire::parse_body_id(id).map_err(|_| "PreviewPlan: bad mesh bodyId"))?;
        if !known.contains(&body) || !meshed.insert(body) {
            return Err(format!(
                "PreviewPlan: mesh for {body} is outside bodies or repeated"
            ));
        }
        if handle.get("snapshotId").and_then(Value::as_u64) != Some(expected.0) {
            return Err(format!(
                "PreviewPlan: mesh for {body} names another snapshot"
            ));
        }
        let name = handle
            .get("bin")
            .and_then(Value::as_str)
            .ok_or("PreviewPlan: mesh handle missing bin")?;
        let section = by_name
            .get(name)
            .ok_or_else(|| format!("PreviewPlan: mesh section {name:?} absent"))?;
        let start = section.off as usize;
        let blob = tail
            .get(start..start + section.len as usize)
            .ok_or("PreviewPlan: mesh section out of range")?;
        let total = handle.get("totalBytes").and_then(Value::as_u64);
        let sha = handle.get("sha256").and_then(Value::as_str);
        wire::verify_mesh(blob, total, sha).map_err(|e| format!("PreviewPlan: {e}"))?;
        referenced.insert(name.to_string());
        out.push(PreparedMeshRef {
            body,
            lod: lod_from_str(handle.get("lod").and_then(Value::as_str).unwrap_or("")),
            bytes: Arc::new(blob.to_vec()),
        });
    }
    wire::reject_unreferenced_sections("PreviewPlan", &by_name, &referenced)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::ids::{DocumentRevision, JobId, WorkerEpoch};
    use onecad_core::regen::{Lod, PlanArtifacts, PlanRequest, PolicyVersions};

    fn request() -> PreviewPlanRequest {
        PreviewPlanRequest {
            plan: PlanRequest {
                job_id: JobId(uuid::Uuid::from_u128(7)),
                document_revision: DocumentRevision(4),
                worker_epoch: WorkerEpoch(2),
                expected_base_hash: onecad_core::regen::HistoryPrefixHash::empty(),
                prefix_hashes: Vec::new(),
                base_checkpoint: None,
                base_checkpoint_artifacts: None,
                ops: Vec::new(),
                policy_versions: PolicyVersions::default(),
                target_step: 3,
                artifacts: PlanArtifacts::default(),
                edited_from: Some(1),
                checkpoint_fallback_replay: true,
            },
            expected_snapshot: SnapshotId(9),
            lod: Lod::Medium,
        }
    }

    #[test]
    fn args_reuse_execute_plan_lowering_minus_publish_keys() {
        let args = preview_plan_args(&request());
        for key in EXECUTE_ONLY_KEYS {
            assert!(args.get(key).is_none(), "{key} must not be sent");
        }
        assert_eq!(args["expectedSnapshotId"], 9);
        assert_eq!(args["workerEpoch"], 2);
        assert_eq!(args["editedFrom"], 1);
        assert_eq!(args["lod"], "medium");
        assert_eq!(args["jobId"], 7);
    }

    fn terminal(per_step: Value, last: Value, stopped: &str) -> Value {
        json!({"planPreviewed": true, "snapshotId": 9, "lastValidStep": last,
               "stoppedReason": stopped, "perStepResults": per_step,
               "bodies": [], "meshes": []})
    }

    #[test]
    fn terminal_summary_must_be_self_consistent() {
        let ok = json!([{"stepIndex":0,"status":"ok"},{"stepIndex":1,"status":"opFailed"}]);
        let parsed = parse_plan_previewed(
            &terminal(ok.clone(), json!(0), "opFailed"),
            &[],
            &[],
            SnapshotId(9),
            vec![event(0)],
        )
        .unwrap();
        assert_eq!(parsed.last_valid_step, Some(0));
        assert!(parse_plan_previewed(
            &terminal(ok.clone(), json!(1), "opFailed"),
            &[],
            &[],
            SnapshotId(9),
            vec![]
        )
        .is_err());
        assert!(parse_plan_previewed(
            &terminal(ok, json!(0), "completed"),
            &[],
            &[],
            SnapshotId(9),
            vec![]
        )
        .is_err());
        let stale = terminal(json!([]), Value::Null, "completed");
        assert!(parse_plan_previewed(&stale, &[], &[], SnapshotId(8), vec![]).is_err());
    }

    fn event(step_index: usize) -> PlanStepEvent {
        wire::parse_plan_step(
            &json!({"stepIndex": step_index, "bodyEvents": []}),
            step_index,
        )
        .unwrap()
    }

    #[test]
    fn streamed_steps_must_match_the_summary() {
        let ok = json!([{"stepIndex":0,"status":"ok"},{"stepIndex":1,"status":"ok"},
                        {"stepIndex":2,"status":"needsRepair"}]);
        let result = terminal(ok, json!(1), "needsRepair");
        let parse = |steps| parse_plan_previewed(&result, &[], &[], SnapshotId(9), steps);
        assert!(parse(vec![event(0), event(1)]).is_ok());
        assert!(
            parse(vec![event(0), event(1), event(2)]).is_ok(),
            "the stopping step may stream"
        );
        let dropped = parse(vec![event(0)]).unwrap_err();
        assert!(dropped.contains("disagree"), "{dropped}");
        assert!(parse(vec![event(1), event(0)]).is_err(), "out of order");
        assert!(
            parse(vec![event(0), event(0), event(1)]).is_err(),
            "repeated"
        );
    }

    #[test]
    fn a_mesh_outside_the_body_set_is_refused() {
        let mut result = terminal(json!([]), Value::Null, "completed");
        result["meshes"] = json!([{"bodyId": format!("body_{}", uuid::Uuid::from_u128(1)),
                                   "bin": "mesh:x", "snapshotId": 9}]);
        let err = parse_plan_previewed(&result, &[], &[], SnapshotId(9), vec![]).unwrap_err();
        assert!(err.contains("outside bodies"), "{err}");
    }
}
