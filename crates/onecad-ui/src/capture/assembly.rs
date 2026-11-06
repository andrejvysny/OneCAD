//! Capture definition-local geometry once and attach occurrence authority.
use crate::bridge::Backend;
use onecad_app::{api, dto::AssemblyProjectionDto};
use onecad_core::{
    document::component::{ComponentDefId, OccurrenceId, OccurrencePath},
    ids::BodyId,
};
use onecad_scene::{
    DMat4, MeshLimits, SceneBody,
    instances::{OccurrenceKey, OccurrencePath as ScenePath},
};
use std::{
    collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

pub(super) async fn capture(
    backend: &Backend,
    session: &str,
    bodies: &mut Vec<SceneBody>,
    bytes_total: &mut usize,
    geometry_budget: &mut usize,
    source_pool: &mut crate::budget::LeasePool,
) -> Result<(u64, usize, AssemblyProjectionDto), String> {
    const MAX_METADATA: usize = 2 << 20;
    let (mut projection, mut metadata_bytes) =
        api::get_assembly_projection_bounded(&backend.state, session, MAX_METADATA)
            .await
            .map_err(|e| e.to_string())?;
    check_instance_budget(&projection, bodies.len())?;
    let needed: BTreeSet<_> = projection
        .occurrences
        .iter()
        .filter(|o| o.visible)
        .map(|o| o.definition.clone())
        .collect();
    for id in needed {
        let definition = projection
            .definitions
            .iter()
            .find(|d| d.id == id)
            .ok_or("missing occurrence definition")?;
        if definition.mesh_generation.is_none() {
            let id = ComponentDefId(uuid::Uuid::parse_str(&id).map_err(|e| e.to_string())?);
            drop(projection);
            let _ = api::regenerate_component_bounded(&backend.state, session, id, MAX_METADATA)
                .await
                .map_err(|e| e.to_string())?;
            (projection, metadata_bytes) =
                api::get_assembly_projection_bounded(&backend.state, session, MAX_METADATA)
                    .await
                    .map_err(|e| e.to_string())?;
        }
    }
    check_instance_budget(&projection, bodies.len())?;
    let generation = projection.persisted_generation;
    let mut definitions = BTreeMap::new();
    let mut ids: BTreeSet<BodyId> = bodies.iter().map(SceneBody::body_id).collect();
    for occurrence in projection.occurrences.iter().filter(|o| o.visible) {
        let definition = projection
            .definitions
            .iter()
            .find(|d| d.id == occurrence.definition)
            .ok_or("missing occurrence definition")?;
        let generation = definition
            .mesh_generation
            .ok_or("component geometry unavailable")?;
        let (path, scene_path) = paths(&occurrence.path)?;
        let model = DMat4::from_cols_array(&occurrence.local.to_cols_array());
        for body in definition.bodies.iter().filter(|b| b.visible) {
            let source: BodyId = body
                .id
                .parse()
                .map_err(|e| format!("component body id: {e}"))?;
            let key = (definition.id.clone(), source);
            if !definitions.contains_key(&key) {
                let bytes = api::component_mesh_for_path(
                    &backend.state,
                    session,
                    &path,
                    source,
                    generation,
                )
                .await
                .map_err(|e| e.to_string())?;
                crate::budget::charge(
                    bytes_total,
                    bytes.len(),
                    crate::budget::MAX_SOURCE_BYTES,
                    "scene source mesh",
                )?;
                let estimate =
                    SceneBody::preflight_geometry_bytes(bytes.as_slice(), MeshLimits::default())
                        .map_err(|e| e.to_string())?;
                crate::budget::charge(
                    geometry_budget,
                    estimate,
                    crate::budget::MAX_DISPLAY_SCENE_BYTES,
                    "scene checked geometry",
                )?;
                let mut geometry = SceneBody::new(
                    source,
                    generation,
                    Arc::from(bytes.as_slice()),
                    MeshLimits::default(),
                )
                .map_err(|e| e.to_string())?;
                let bytes = geometry.geometry_bytes().map_err(|e| e.to_string())?;
                geometry
                    .retain_geometry(source_pool.carve(bytes)?)
                    .map_err(|e| e.to_string())?;
                definitions.insert(key.clone(), geometry);
            }
            let geometry = definitions
                .get(&key)
                .ok_or("component geometry unavailable")?;
            bodies.push(instance(
                geometry,
                &occurrence.path,
                scene_path.clone(),
                model,
                &mut ids,
            )?);
        }
    }
    Ok((generation, metadata_bytes, projection))
}

fn checked_instance_count(
    root: usize,
    mut counts: impl Iterator<Item = usize>,
) -> Result<(), String> {
    let total = counts
        .try_fold(root, |total, count| total.checked_add(count))
        .ok_or("Native scene body count overflow")?;
    if total > crate::budget::MAX_BODIES {
        return Err("Native scene body budget exceeded".into());
    }
    Ok(())
}

fn check_instance_budget(projection: &AssemblyProjectionDto, root: usize) -> Result<(), String> {
    checked_instance_count(
        root,
        projection
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.visible)
            .map(|occurrence| {
                projection
                    .definitions
                    .iter()
                    .find(|definition| definition.id == occurrence.definition)
                    .map_or(usize::MAX, |definition| {
                        definition.bodies.iter().filter(|body| body.visible).count()
                    })
            }),
    )
}

fn instance(
    geometry: &SceneBody,
    path: &[String],
    scene_path: ScenePath,
    model: DMat4,
    ids: &mut BTreeSet<BodyId>,
) -> Result<SceneBody, String> {
    let display = display_id(path, geometry.source_body_id());
    if !ids.insert(display) {
        return Err("component display identity collision".into());
    }
    geometry
        .instance(display, scene_path, model, true)
        .map_err(|e| e.to_string())
}

fn paths(path: &[String]) -> Result<(OccurrencePath, ScenePath), String> {
    let ids: Vec<_> = path
        .iter()
        .map(|id| uuid::Uuid::parse_str(id).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    Ok((
        OccurrencePath(ids.iter().copied().map(OccurrenceId).collect()),
        ScenePath(
            ids.into_iter()
                .map(|id| OccurrenceKey(id.as_u128()))
                .collect(),
        ),
    ))
}

fn display_id(path: &[String], body: BodyId) -> BodyId {
    let digest = |domain: u8| {
        let mut h = DefaultHasher::new();
        (domain, path, body).hash(&mut h);
        h.finish()
    };
    BodyId(uuid::Uuid::from_u128(
        (u128::from(digest(0)) << 64) | u128::from(digest(1)),
    ))
}

pub(super) async fn unchanged(
    backend: &Backend,
    session: &str,
    generation: u64,
) -> Result<bool, String> {
    let current = api::assembly_generation(&backend.state, session)
        .await
        .map_err(|e| e.to_string())?;
    Ok(current == generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_body_budget_refuses_before_instance_allocation() {
        assert!(checked_instance_count(0, std::iter::repeat_n(2, 128)).is_ok());
        assert!(checked_instance_count(1, std::iter::repeat_n(2, 128)).is_err());
        assert!(checked_instance_count(0, std::iter::repeat_n(4096, 2000)).is_err());
        assert!(checked_instance_count(1, [usize::MAX].into_iter()).is_err());
    }

    #[test]
    fn repeated_instances_share_geometry_and_retain_path_pose_and_collision_check() {
        let source = BodyId(uuid::Uuid::from_u128(10));
        let geometry = SceneBody::new(
            source,
            7,
            Arc::from(&include_bytes!("../../../../fixtures/render/edge-policy/box.mesh1")[..]),
            MeshLimits::default(),
        )
        .unwrap();
        let a = vec![uuid::Uuid::from_u128(11).to_string()];
        let b = vec![uuid::Uuid::from_u128(12).to_string()];
        let mut ids = BTreeSet::new();
        let first = instance(
            &geometry,
            &a,
            paths(&a).unwrap().1,
            DMat4::IDENTITY,
            &mut ids,
        )
        .unwrap();
        let pose = DMat4::from_translation(onecad_scene::DVec3::new(5.0, 0.0, 0.0));
        let second = instance(&geometry, &b, paths(&b).unwrap().1, pose, &mut ids).unwrap();
        assert!(first.shares_geometry(&second));
        assert_ne!(first.body_id(), second.body_id());
        assert_eq!(second.source_body_id(), source);
        assert_eq!(second.occurrence_path(), Some(&paths(&b).unwrap().1));
        assert_eq!(second.model(), pose);
        assert!(instance(&geometry, &b, paths(&b).unwrap().1, pose, &mut ids).is_err());
    }
}
