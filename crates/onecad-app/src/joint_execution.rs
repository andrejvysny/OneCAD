//! Bounded latest-only joint solves over the document's single writer.
use crate::{error::ApiError, state::AppState};
use onecad_assembly::{
    joint::{Joint, JointId, JointKind},
    occurrence::OccurrenceId,
    solver::{self, SolveOutcome, SolverBody},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::atomic::Ordering,
};

/// Numerical result from one committed grounded-tree solve.
#[derive(Debug, Clone)]
pub struct JointSolveReport {
    pub iterations: u32,
    pub max_residual: f64,
    pub remaining_dof: usize,
    pub redundant_constraints: usize,
}

/// Cancel this host's queued and in-flight joint commits.
pub fn cancel_joint_solves(state: &AppState) -> Result<(), ApiError> {
    next_ticket(state).map(|_| ())
}

fn next_ticket(state: &AppState) -> Result<u64, ApiError> {
    let _guard = state
        .joint_gate
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state
        .joint_ticket
        .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        .map(|n| n + 1)
        .map_err(|_| ApiError::InvalidCommand("joint request generation exhausted".into()))
}

/// Solve and atomically commit an entire joint set and occurrence poses.
/// Origins are already-resolved local frames; persistent picked origins are not supported.
pub async fn solve_joints(
    state: &AppState,
    session: &str,
    joints: Vec<Joint>,
    drives: Vec<(JointId, f64)>,
) -> Result<JointSolveReport, ApiError> {
    solve_joints_inner(state, session, None, joints, drives, None, None).await
}

/// Refuses stale displayed joint edits before immutable solve preparation.
pub async fn solve_joints_at_generation(
    state: &AppState,
    session: &str,
    generation: u64,
    joints: Vec<Joint>,
    drives: Vec<(JointId, f64)>,
) -> Result<JointSolveReport, ApiError> {
    solve_joints_inner(state, session, Some(generation), joints, drives, None, None).await
}

/// Cancellation remains authoritative even if it precedes ticket admission.
pub async fn solve_joints_at_generation_cancellable(
    state: &AppState,
    session: &str,
    generation: u64,
    joints: Vec<Joint>,
    drives: Vec<(JointId, f64)>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<JointSolveReport, ApiError> {
    solve_joints_inner(
        state,
        session,
        Some(generation),
        joints,
        drives,
        Some(cancelled),
        None,
    )
    .await
}

/// Revalidates every persistent origin before committing the solved joint set.
pub async fn solve_bound_joints_at_generation_cancellable(
    state: &AppState,
    session: &str,
    generation: u64,
    joints: Vec<Joint>,
    drives: Vec<(JointId, f64)>,
    bindings: Vec<onecad_core::document::component::JointBinding>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<JointSolveReport, ApiError> {
    solve_joints_inner(
        state,
        session,
        Some(generation),
        joints,
        drives,
        Some(cancelled),
        Some(bindings),
    )
    .await
}

async fn solve_joints_inner(
    state: &AppState,
    session: &str,
    expected_generation: Option<u64>,
    joints: Vec<Joint>,
    drives: Vec<(JointId, f64)>,
    cancelled: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    bindings: Option<Vec<onecad_core::document::component::JointBinding>>,
) -> Result<JointSolveReport, ApiError> {
    require_not_cancelled(cancelled.as_deref())?;
    let slot = state
        .joint_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::InvalidCommand("joint solve capacity is busy".into()))?;
    let ticket = next_ticket(state)?;
    let origin_head = if let Some(bindings) = &bindings {
        Some(
            validate_bound_origins(
                state,
                session,
                expected_generation.ok_or_else(|| {
                    ApiError::InvalidCommand("bound origins require a generation".into())
                })?,
                &joints,
                bindings,
                cancelled.as_deref(),
                ticket,
            )
            .await?,
        )
    } else {
        None
    };
    let capture = {
        let guard = state.runtime.lock().await;
        if expected_generation.is_some_and(|generation| {
            guard
                .as_ref()
                .is_none_or(|runtime| runtime.persisted_generation() != generation)
        }) {
            return Err(ApiError::StalePreview(
                "joint edit display generation is stale".into(),
            ));
        }

        state.ensure_not_closing()?;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("joint solve".into()))?;
        if origin_head.is_some_and(|head| head != runtime.joint_head()) {
            return Err(ApiError::StalePreview("joint origin head changed".into()));
        }
        match bindings {
            Some(bindings) => {
                runtime.prepare_bound_joint_solve(session, joints, drives, Some(bindings))?
            }
            None => runtime.prepare_joint_solve(session, joints, drives)?,
        }
    };
    let lane = state.joint_lane.clone().lock_owned().await;
    if state.joint_ticket.load(Ordering::Acquire) != ticket {
        return Err(ApiError::StalePreview("joint solve superseded".into()));
    }
    let before = capture
        .engine
        .get_worker_head()
        .await
        .map_err(|error| ApiError::Worker(error.to_string()))?;
    if before.worker_epoch != capture.epoch
        || before.snapshot_id != capture.snapshot
        || before.inflight.is_some()
        || before.history_prefix_hash != capture.base
        || before.has_scratch
        || before.has_restored_base
    {
        return Err(ApiError::StalePreview(
            "joint solve worker base changed".into(),
        ));
    }
    let request = capture.request.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let _lane = lane;
        let _slot = slot;
        solver::solve(&request)
    })
    .await
    .map_err(|error| ApiError::Worker(format!("joint solver task: {error}")))?;
    let (poses, report) = match outcome {
        SolveOutcome::Converged {
            poses,
            iterations,
            max_residual,
            remaining_dof,
            redundant_constraints,
        } => (
            poses,
            JointSolveReport {
                iterations,
                max_residual,
                remaining_dof,
                redundant_constraints,
            },
        ),
        other => {
            return Err(ApiError::InvalidCommand(format!(
                "joint solve refused: {other:?}"
            )));
        }
    };
    let after = capture
        .engine
        .get_worker_head()
        .await
        .map_err(|error| ApiError::Worker(error.to_string()))?;
    if after.worker_epoch != before.worker_epoch
        || after.snapshot_id != before.snapshot_id
        || after.history_prefix_hash != before.history_prefix_hash
        || after.inflight.is_some()
        || after.has_scratch
        || after.has_restored_base
    {
        return Err(ApiError::StalePreview("joint solve worker changed".into()));
    }
    #[cfg(test)]
    let hook = state.joint_before_commit.lock().unwrap().take();
    #[cfg(test)]
    if let Some((started, release)) = hook {
        let _ = started.send(());
        let _ = release.await;
    }
    let mut joints = capture.request.joints.clone();
    for (id, value) in &capture.request.drive {
        if let Some(joint) = joints.iter_mut().find(|joint| joint.id == *id) {
            joint.value = *value;
        }
    }
    let changed = {
        let mut guard = state.runtime.lock().await;
        state.ensure_not_closing()?;
        let _ticket_guard = state
            .joint_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_not_cancelled(cancelled.as_deref())?;
        if state.joint_ticket.load(Ordering::Acquire) != ticket {
            return Err(ApiError::StalePreview("joint solve superseded".into()));
        }
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("joint solve".into()))?;
        runtime.commit_joint_solve(&capture, joints, poses)?;
        runtime.persisted_generation() != capture.generation
    };
    if changed {
        state.note_mutation();
    }
    Ok(report)
}

async fn validate_bound_origins(
    state: &AppState,
    session: &str,
    generation: u64,
    joints: &[Joint],
    bindings: &[onecad_core::document::component::JointBinding],
    cancelled: Option<&std::sync::atomic::AtomicBool>,
    ticket: u64,
) -> Result<crate::document_runtime::joint_execution::JointHead, ApiError> {
    use onecad_core::document::component::JointSide;
    if bindings.len() > 40 {
        return Err(ApiError::InvalidCommand(
            "too many bound joint origins".into(),
        ));
    }
    let head = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .filter(|r| {
                r.matches_runtime_session(session) && r.persisted_generation() == generation
            })
            .ok_or_else(|| ApiError::StalePreview("joint origin document changed".into()))?;
        runtime.validate_joint_binding_structure(joints, bindings)?;
        runtime.joint_head()
    };
    for binding in bindings {
        require_not_cancelled(cancelled)?;
        let joint = joints
            .iter()
            .find(|j| j.id == binding.joint)
            .ok_or_else(|| ApiError::InvalidCommand("joint binding has no joint".into()))?;
        if joint.suppressed {
            continue;
        }
        if binding.needs_repair {
            return Err(ApiError::InvalidCommand("joint origin needs repair".into()));
        }
        let origin = match binding.side {
            JointSide::A => &joint.a,
            JointSide::B => &joint.b,
        };
        let primary = binding
            .element
            .primary
            .as_ref()
            .ok_or_else(|| ApiError::InvalidCommand("joint face lacks identity".into()))?;
        let mesh_generation = {
            let guard = state.runtime.lock().await;
            guard
                .as_ref()
                .and_then(|r| r.component_generation(binding.definition))
                .ok_or_else(|| {
                    ApiError::InvalidCommand("joint origin needs component replay".into())
                })?
        };
        let result = crate::api::component_execution::component_joint_origin(
            state,
            crate::api::JointOriginRequest {
                session,
                generation,
                path: &binding.path,
                body: binding.body,
                mesh_generation,
                element: primary.element.as_str(),
                x: binding.orientation_x,
            },
        )
        .await;
        let resolved = match result {
            Ok(value) => value,
            Err(error @ ApiError::UnresolvedTopology(_)) => {
                mark_origin_repair(
                    state,
                    session,
                    generation,
                    binding,
                    origin.frame,
                    cancelled,
                    (ticket, head.clone()),
                )
                .await?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        if resolved.definition != binding.definition
            || resolved.reference != binding.element
            || resolved.surface != binding.surface
            || resolved.radius != binding.radius
            || resolved.frame != origin.frame
        {
            mark_origin_repair(
                state,
                session,
                generation,
                binding,
                origin.frame,
                cancelled,
                (ticket, head.clone()),
            )
            .await?;
            return Err(ApiError::InvalidCommand(
                "joint origin needs repair: datum changed".into(),
            ));
        }
    }
    require_not_cancelled(cancelled)?;
    Ok(head)
}

async fn mark_origin_repair(
    state: &AppState,
    session: &str,
    generation: u64,
    binding: &onecad_core::document::component::JointBinding,
    frame: onecad_assembly::math::Frame,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
    fence: (u64, crate::document_runtime::joint_execution::JointHead),
) -> Result<(), ApiError> {
    require_not_cancelled(cancelled)?;
    let changed = {
        let mut guard = state.runtime.lock().await;
        let _gate = state
            .joint_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_not_cancelled(cancelled)?;
        if state.joint_ticket.load(Ordering::Acquire) != fence.0 {
            return Err(ApiError::StalePreview("joint solve superseded".into()));
        }
        state.ensure_not_closing()?;
        guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("joint repair".into()))?
            .mark_joint_origin_repair(session, generation, fence.1, binding, frame)?
    };
    if changed {
        state.note_mutation();
    }
    Ok(())
}

fn require_not_cancelled(
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), ApiError> {
    if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire)) {
        return Err(ApiError::StalePreview("joint solve cancelled".into()));
    }
    Ok(())
}

pub(crate) fn validate_tree(
    bodies: &[SolverBody],
    joints: &[Joint],
    drives: &[(JointId, f64)],
) -> Result<(), ApiError> {
    let invalid = |message: &str| ApiError::InvalidCommand(message.into());
    if bodies.len() > 20 || joints.len() > 20 {
        return Err(invalid("joint solve exceeds 20 occurrences"));
    }
    let mut graph: BTreeMap<OccurrenceId, Vec<OccurrenceId>> =
        bodies.iter().map(|b| (b.id, Vec::new())).collect();
    let mut ids = BTreeSet::new();
    for joint in joints {
        if !ids.insert(joint.id) {
            return Err(invalid("duplicate joint identity"));
        }
        joint.validate().map_err(|e| invalid(&e.to_string()))?;
        if joint.suppressed {
            continue;
        }
        graph
            .get_mut(&joint.a.occurrence)
            .ok_or_else(|| invalid("joint endpoint missing"))?
            .push(joint.b.occurrence);
        graph
            .get_mut(&joint.b.occurrence)
            .ok_or_else(|| invalid("joint endpoint missing"))?
            .push(joint.a.occurrence);
    }
    let mut drive_ids = BTreeSet::new();
    for (id, value) in drives {
        let joint = joints
            .iter()
            .find(|j| j.id == *id && !j.suppressed)
            .ok_or_else(|| invalid("drive joint missing or suppressed"))?;
        if !drive_ids.insert(*id)
            || joint.kind == JointKind::Rigid
            || !value.is_finite()
            || joint.limits.is_some_and(|limits| !limits.contains(*value))
        {
            return Err(invalid("invalid or coupled joint drive"));
        }
    }
    let mut seen = BTreeSet::new();
    for body in bodies {
        if seen.contains(&body.id) {
            continue;
        }
        let mut queue = VecDeque::from([body.id]);
        let mut count = 0usize;
        let mut edges = 0usize;
        let mut grounded = 0usize;
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id) {
                continue;
            }
            count += 1;
            grounded += usize::from(bodies.iter().any(|b| b.id == id && b.grounded));
            let neighbours = graph
                .get(&id)
                .ok_or_else(|| invalid("joint endpoint missing"))?;
            edges += neighbours.len();
            queue.extend(neighbours.iter().copied());
        }
        if grounded != 1 || edges != 2 * (count - 1) {
            return Err(invalid(
                "joints require one grounded root per acyclic component",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
