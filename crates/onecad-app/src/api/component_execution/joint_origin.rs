//! Exact occurrence-qualified joint datum queries.
use super::*;

/// Immutable occurrence-qualified face and explicit orientation request.
pub struct JointOriginRequest<'a> {
    pub session: &'a str,
    pub generation: u64,
    pub path: &'a onecad_core::document::component::OccurrencePath,
    pub body: onecad_core::ids::BodyId,
    pub mesh_generation: u64,
    pub element: &'a str,
    pub x: onecad_assembly::math::Vec3d,
}

/// Exact worker-classified local datum and durable identity evidence.
pub struct JointOriginResolution {
    pub definition: ComponentDefId,
    pub reference: onecad_core::document::refs::ElementRef,
    pub surface: onecad_core::document::component::JointSurface,
    pub radius: Option<f64>,
    pub frame: onecad_assembly::math::Frame,
}

/// Resolve an anchored occurrence face into a local datum without guessing its x axis.
pub async fn component_joint_origin(
    state: &AppState,
    request: JointOriginRequest<'_>,
) -> Result<JointOriginResolution, ApiError> {
    resolve(state, request, false).await
}

pub(super) async fn component_joint_origin_pending(
    state: &AppState,
    request: JointOriginRequest<'_>,
) -> Result<JointOriginResolution, ApiError> {
    resolve(state, request, true).await
}

async fn resolve(
    state: &AppState,
    request: JointOriginRequest<'_>,
    allow_pending: bool,
) -> Result<JointOriginResolution, ApiError> {
    let JointOriginRequest {
        session,
        generation,
        path,
        body,
        mesh_generation,
        element,
        x,
    } = request;
    let (definition, input, reference, head) = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .filter(|r| {
                r.matches_runtime_session(session) && r.persisted_generation() == generation
            })
            .ok_or_else(|| ApiError::StalePreview("joint origin document is stale".into()))?;
        if (if allow_pending {
            runtime.component_mesh_for_validation(path, body, mesh_generation)
        } else {
            runtime.component_mesh_for_path(path, body, mesh_generation)
        })
        .is_none()
        {
            return Err(ApiError::StalePreview("joint origin mesh is stale".into()));
        }
        let (definition, reference) = runtime.joint_face_reference(path, body, element)?;
        (
            definition,
            runtime.prepare_component_replay(definition, session)?,
            reference,
            runtime.joint_head(),
        )
    };
    let lease = open_worker(state, &input).await?;
    let output = input
        .drive(lease.worker.clone(), lease.epoch, false)
        .await?;
    let snapshot = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .filter(|r| r.persisted_generation() == generation && r.joint_head() == head)
            .ok_or_else(|| ApiError::StalePreview("joint origin document changed".into()))?;
        runtime.validate_component_query(
            &output,
            lease.worker.current_epoch(),
            path,
            body,
            mesh_generation,
            allow_pending,
        )?
    };
    let info = lease
        .worker
        .query_element(snapshot, body, element)
        .await
        .map_err(|e| ApiError::Worker(format!("joint identity query: {e}")))?
        .ok_or_else(|| ApiError::UnresolvedTopology("joint face needs repair".into()))?;
    if info.element_id != element
        || info.kind != "face"
        || crate::worker::wire::parse_body_id(&info.body_id).ok() != Some(body)
    {
        return Err(ApiError::UnresolvedTopology(
            "joint face identity mismatch".into(),
        ));
    }
    let classification = lease
        .worker
        .classify_element(body, element)
        .await
        .map_err(|e| ApiError::Worker(format!("joint datum query: {e}")))?
        .ok_or_else(|| ApiError::UnresolvedTopology("joint face needs repair".into()))?;
    let (surface, frame) = joint_datum_frame(&classification, x)?;
    let radius = classification.frame.as_ref().and_then(|datum| datum.radius);
    {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .filter(|r| r.persisted_generation() == generation && r.joint_head() == head)
            .ok_or_else(|| ApiError::StalePreview("joint origin document changed".into()))?;
        runtime.validate_component_query(
            &output,
            lease.worker.current_epoch(),
            path,
            body,
            mesh_generation,
            allow_pending,
        )?;
    }
    lease.drain().await;
    Ok(JointOriginResolution {
        definition,
        reference,
        surface,
        radius,
        frame,
    })
}

/// Captures bounded joint bindings from the exact displayed document generation.
pub async fn get_joint_bindings(
    state: &AppState,
    session: &str,
    generation: u64,
) -> Result<Vec<onecad_core::document::component::JointBinding>, ApiError> {
    let guard = state.runtime.lock().await;
    guard
        .as_ref()
        .filter(|r| r.matches_runtime_session(session) && r.persisted_generation() == generation)
        .ok_or_else(|| ApiError::StalePreview("joint bindings are stale".into()))?
        .joint_bindings()
}

fn joint_datum_frame(
    classification: &crate::dto::ClassifyElementDto,
    x: onecad_assembly::math::Vec3d,
) -> Result<
    (
        onecad_core::document::component::JointSurface,
        onecad_assembly::math::Frame,
    ),
    ApiError,
> {
    use onecad_assembly::math::{Frame, Vec3d};
    use onecad_core::document::component::JointSurface;
    let datum = classification
        .frame
        .as_ref()
        .ok_or_else(|| ApiError::UnresolvedTopology("joint face has no exact datum".into()))?;
    let (surface, z) = match classification.surface_type.as_str() {
        "plane" => (JointSurface::Plane, datum.normal),
        "cylinder" => (JointSurface::Cylinder, datum.axis),
        _ => {
            return Err(ApiError::UnresolvedTopology(
                "joint origin requires a planar or cylindrical face".into(),
            ));
        }
    };
    let z =
        z.ok_or_else(|| ApiError::UnresolvedTopology("joint face has no exact direction".into()))?;
    if surface == JointSurface::Cylinder && !datum.radius.is_some_and(|r| r.is_finite() && r > 0.0)
    {
        return Err(ApiError::UnresolvedTopology(
            "joint cylinder has no exact radius".into(),
        ));
    }
    let frame = Frame {
        origin: Vec3d::new(datum.origin[0], datum.origin[1], datum.origin[2]),
        z: Vec3d::new(z[0], z[1], z[2]),
        x,
    };
    if classification.kind != "face"
        || !x.is_finite()
        || (x.norm() - 1.0).abs() > 1e-8
        || (frame.z.norm() - 1.0).abs() > 1e-8
        || frame.z.dot(x).abs() > 1e-8
        || frame.to_transform().is_none()
    {
        return Err(ApiError::InvalidCommand(
            "joint datum requires unit perpendicular x and z axes".into(),
        ));
    }
    Ok((surface, frame))
}
