//! Fenced service execution for geometry-bound joint edits.
use super::*;
use crate::assembly_controls::OriginPick;
use onecad_assembly::joint::{Joint, JointId};
use onecad_core::document::component::{JointBinding, JointSide, OccurrencePath};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) struct Request {
    pub session: String,
    pub generation: u64,
    pub joints: Vec<Joint>,
    pub drives: Vec<(JointId, f64)>,
    pub joint: JointId,
    pub picks: Vec<OriginPick>,
    pub unbound: Vec<JointSide>,
    pub cancellation: Option<Arc<AtomicBool>>,
}

pub(super) async fn solve(
    backend: &Backend,
    mut request: Request,
) -> Result<(Option<joint_execution::JointSolveReport>, bool), String> {
    let mut bindings =
        api::get_joint_bindings(&backend.state, &request.session, request.generation)
            .await
            .map_err(|error| error.to_string())?;
    bindings.retain(|binding| {
        request.joints.iter().any(|joint| joint.id == binding.joint)
            && !(binding.joint == request.joint && request.unbound.contains(&binding.side))
    });
    for pick in &request.picks {
        let binding = resolve_pick(
            backend,
            &mut request.joints,
            request.joint,
            pick,
            &request.session,
            request.generation,
        )
        .await?;
        bindings.retain(|current| current.joint != binding.joint || current.side != binding.side);
        bindings.push(binding);
    }
    let token = request
        .cancellation
        .ok_or("missing joint cancellation token")?;
    let report = joint_execution::solve_bound_joints_at_generation_cancellable(
        &backend.state,
        &request.session,
        request.generation,
        request.joints,
        request.drives,
        bindings,
        token,
    )
    .await
    .map_err(|error| error.to_string())?;
    let changed = api::get_assembly_projection(&backend.state, &request.session)
        .await
        .map_or(true, |projection| {
            projection.persisted_generation != request.generation
        });
    Ok((Some(report), changed))
}

async fn resolve_pick(
    backend: &Backend,
    joints: &mut [Joint],
    joint: JointId,
    pick: &OriginPick,
    session: &str,
    generation: u64,
) -> Result<JointBinding, String> {
    let scene = backend
        .shared
        .displayed_scene
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if scene != Some((pick.selection.scene, session.to_owned(), generation)) {
        return Err("Selected face belongs to a stale scene".into());
    }
    let joint_data = joints
        .iter_mut()
        .find(|item| item.id == joint)
        .ok_or("joint no longer exists")?;
    let origin = match pick.side {
        JointSide::A => &mut joint_data.a,
        JointSide::B => &mut joint_data.b,
    };
    let path = OccurrencePath(
        pick.selection
            .path
            .0
            .iter()
            .map(|key| onecad_assembly::occurrence::OccurrenceId(uuid::Uuid::from_u128(key.0)))
            .collect(),
    );
    if path.0.last() != Some(&origin.occurrence) {
        return Err("Selected face belongs to another occurrence".into());
    }
    let resolved = api::component_joint_origin(
        &backend.state,
        api::JointOriginRequest {
            session,
            generation,
            path: &path,
            body: pick.selection.source_body,
            mesh_generation: pick.mesh_generation,
            element: &pick.selection.topology_id,
            x: origin.frame.x,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    origin.frame = resolved.frame;
    Ok(JointBinding {
        joint,
        side: pick.side,
        path,
        definition: resolved.definition,
        body: pick.selection.source_body,
        element: resolved.reference,
        surface: resolved.surface,
        radius: resolved.radius,
        orientation_x: resolved.frame.x,
        needs_repair: false,
    })
}
