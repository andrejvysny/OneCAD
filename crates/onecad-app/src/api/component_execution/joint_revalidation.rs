//! Revalidates persisted joint origins after a definition is replayed.
use super::*;
use onecad_core::document::component::{JointBinding, JointSide};

pub(super) struct Validated {
    pub generation: u64,
    pub head: crate::document_runtime::joint_execution::JointHead,
}

pub(super) async fn revalidate(
    state: &AppState,
    session: &str,
    definition: ComponentDefId,
    mesh_generation: u64,
    projection: &AssemblyProjectionDto,
) -> Result<Validated, ApiError> {
    let mut generation = projection.persisted_generation;
    let head = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .filter(|r| {
                r.matches_runtime_session(session) && r.persisted_generation() == generation
            })
            .ok_or_else(|| ApiError::StalePreview("joint revalidation document changed".into()))?
            .joint_head()
    };
    let bindings = get_joint_bindings(state, session, generation).await?;
    for binding in bindings
        .iter()
        .filter(|binding| binding.definition == definition && !binding.needs_repair)
    {
        let joint = projection
            .joints
            .iter()
            .find(|joint| joint.id == binding.joint)
            .ok_or_else(|| ApiError::UnresolvedTopology("joint binding has no joint".into()))?;
        let frame = match binding.side {
            JointSide::A => joint.a.frame,
            JointSide::B => joint.b.frame,
        };
        let primary = binding
            .element
            .primary
            .as_ref()
            .ok_or_else(|| ApiError::UnresolvedTopology("joint face lacks identity".into()))?;
        let result = super::joint_origin::component_joint_origin_pending(
            state,
            JointOriginRequest {
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
        let needs_repair = match result {
            Ok(value) => {
                value.definition != binding.definition
                    || value.reference != binding.element
                    || value.surface != binding.surface
                    || value.radius != binding.radius
                    || value.frame != frame
            }
            Err(ApiError::UnresolvedTopology(_)) => true,
            Err(error) => return Err(error),
        };
        if needs_repair {
            let (marked, current) =
                mark(state, session, generation, binding, frame, head.clone()).await?;
            let _ = marked;
            generation = current;
        }
    }
    let guard = state.runtime.lock().await;
    if guard.as_ref().is_none_or(|r| {
        !r.matches_runtime_session(session)
            || r.persisted_generation() != generation
            || r.joint_head() != head
    }) {
        return Err(ApiError::StalePreview(
            "joint revalidation head changed".into(),
        ));
    }
    Ok(Validated { generation, head })
}

async fn mark(
    state: &AppState,
    session: &str,
    generation: u64,
    binding: &JointBinding,
    frame: onecad_assembly::math::Frame,
    head: crate::document_runtime::joint_execution::JointHead,
) -> Result<(bool, u64), ApiError> {
    let mut guard = state.runtime.lock().await;
    state.ensure_not_closing()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("joint revalidation".into()))?;
    let read_only = runtime.is_read_only();
    let changed = runtime.mark_joint_origin_repair(session, generation, head, binding, frame)?;
    let current = runtime.persisted_generation();
    drop(guard);
    if changed && !read_only {
        state.note_mutation();
    }
    Ok((changed, current))
}
