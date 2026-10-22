//! Fresh Hole operands freeze the kernel-classified point, never the plane origin.
use super::{operands::stage_error, preview_context, PreviewContext, StageAddress, StagePick};
use crate::{error::ApiError, state::AppState, worker::wire};
use onecad_core::{
    document::refs::{AnchorIntent, ElementKind, ElementRef},
    ids::TopoKey,
    math::Vec3,
};

/// Stages one planar face with its exact IN/ON point without changing the document.
pub async fn stage_hole(
    state: &AppState,
    scope: &PreviewContext,
    pick: StagePick,
) -> Result<(ElementRef, Vec3), ApiError> {
    if pick.kind != ElementKind::Face {
        return Err(refused("Select a planar face for Hole"));
    }
    current(state, scope).await?;
    let ticket = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("stageHole".into()))?
            .stage_ticket(scope)
            .map_err(stage_error)?
    };
    let key = face_key(state, scope, &pick).await?;
    current(state, scope).await?;
    let frame = state
        .face_projection()
        .project_face_boundary_frame(
            scope.snapshot_id,
            pick.body,
            wire::FaceAddress::TopoKey(&key),
        )
        .await?
        .ok_or_else(|| refused("Selected Hole face is absent or not planar"))?;
    current(state, scope).await?;
    let point = checked_point(frame)?;
    let reference = stage_face(state, &ticket, pick.body, key, point).await?;
    current(state, scope).await?;
    validate_identity(&pick, &reference)?;
    Ok((reference, point))
}

async fn stage_face(
    state: &AppState,
    ticket: &crate::document_runtime::StageTicket,
    body: onecad_core::ids::BodyId,
    key: String,
    point: Vec3,
) -> Result<ElementRef, ApiError> {
    let mut guard = state.runtime.lock().await;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("stageHole".into()))?;
    if !runtime.stage_ticket_is_current(ticket) {
        return Err(ApiError::StalePreview(
            "Hole face belongs to an obsolete head; re-pick".into(),
        ));
    }
    runtime
        .stage_anchored(
            ticket,
            body,
            vec![(
                TopoKey::new(key),
                Some(AnchorIntent {
                    world_point: point,
                    surface_uv: None,
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
            )],
        )
        .await
        .map_err(stage_error)?
        .pop()
        .ok_or_else(|| refused("Hole face staging returned no reference"))
}

async fn face_key(
    state: &AppState,
    scope: &PreviewContext,
    pick: &StagePick,
) -> Result<String, ApiError> {
    match &pick.address {
        StageAddress::TopoKey(key) if !key.is_empty() => Ok(key.clone()),
        StageAddress::Element(id) if !id.is_empty() => {
            let info = state
                .element_query()
                .query_element(scope.snapshot_id, pick.body, id)
                .await?
                .ok_or_else(|| refused("Selected Hole face no longer exists"))?;
            if info.body_id != wire::body_id_wire(pick.body)
                || info.kind != "face"
                || info.surface_type != 0
            {
                return Err(refused(
                    "Selected Hole operand is not a planar face on the requested body",
                ));
            }
            Ok(info.topo_key)
        }
        _ => Err(refused("Select a face with a valid topology identity")),
    }
}

fn checked_point(frame: onecad_core::sketch::FaceFrame) -> Result<Vec3, ApiError> {
    let point = frame.anchor.ok_or_else(|| {
        refused("Kernel could not classify a point on this face; select another Hole face")
    })?;
    let normal_squared = frame.normal.x.powi(2) + frame.normal.y.powi(2) + frame.normal.z.powi(2);
    if !frame.origin.is_finite()
        || !frame.normal.is_finite()
        || !point.is_finite()
        || !normal_squared.is_finite()
        || normal_squared <= 0.0
    {
        return Err(refused("Selected Hole face has an invalid planar frame"));
    }
    Ok(point)
}

fn validate_identity(pick: &StagePick, reference: &ElementRef) -> Result<(), ApiError> {
    let primary = reference
        .primary
        .as_ref()
        .ok_or_else(|| refused("Staged Hole face has no identity"))?;
    let same_element = match &pick.address {
        StageAddress::Element(id) => primary.element.as_str() == id,
        StageAddress::TopoKey(_) => true,
    };
    if primary.body != pick.body || primary.kind != ElementKind::Face || !same_element {
        return Err(refused("Staged Hole face changed body, kind or identity"));
    }
    Ok(())
}

async fn current(state: &AppState, scope: &PreviewContext) -> Result<(), ApiError> {
    if preview_context(state).await? != *scope {
        return Err(ApiError::StalePreview(
            "Hole face belongs to an obsolete head; re-pick".into(),
        ));
    }
    Ok(())
}

fn refused(message: &str) -> ApiError {
    ApiError::InvalidCommand(format!("{message}; re-pick"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::sketch::FaceFrame;

    #[test]
    fn hole_point_never_falls_back_to_origin_or_invalid_frame() {
        let frame = FaceFrame {
            origin: Vec3::new_unchecked(0.0, 0.0, 25.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            anchor: None,
        };
        assert!(checked_point(frame).is_err());
        let point = Vec3::new_unchecked(20.0, 10.0, 25.0);
        assert_eq!(
            checked_point(FaceFrame {
                anchor: Some(point),
                ..frame
            })
            .unwrap(),
            point
        );
        assert!(checked_point(FaceFrame {
            anchor: Some(Vec3::new_unchecked(f64::NAN, 0.0, 0.0)),
            ..frame
        })
        .is_err());
        assert!(checked_point(FaceFrame {
            anchor: Some(point),
            normal: Vec3::new_unchecked(0.0, 0.0, 0.0),
            ..frame
        })
        .is_err());
    }
}
