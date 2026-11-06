//! One exact candidate at a time, isolated from the document actor's control loop.
use super::check_tag;
use crate::bridge::{Backend, Candidate, CandidateDelivery, Shared};
use onecad_app::{api, state::AppState};
use onecad_core::regen::Lod;
use onecad_interaction::Action;
use onecad_scene::{MeshLimits, SceneBody, SceneLimits, SceneSnapshot};
use std::sync::Arc;
#[path = "edit_preview_service.rs"]
mod edit_preview_service;

#[derive(Clone)]
pub(crate) struct CandidateBackend {
    pub state: Arc<AppState>,
    pub shared: Arc<Shared>,
}
impl CandidateBackend {
    fn current(&self, epoch: u64) -> bool {
        self.shared.is_current(epoch)
            && self.shared.valid.load(std::sync::atomic::Ordering::Acquire)
            && !self
                .shared
                .stopping
                .load(std::sync::atomic::Ordering::Acquire)
    }
    fn ensure_current(
        &self,
        epoch: u64,
        tag: &onecad_interaction::RequestTag,
    ) -> Result<(), String> {
        if self.current(epoch) {
            return Ok(());
        }
        crate::performance::record(
            crate::performance::Stage::PreviewObsolete,
            tag.context.scene.session,
            [tag.sequence, tag.context.scene.scene, tag.gesture],
        );
        Err("Preview was invalidated before preparation".into())
    }
}
impl From<&Backend> for CandidateBackend {
    fn from(backend: &Backend) -> Self {
        Self {
            state: backend.state.clone(),
            shared: backend.shared.clone(),
        }
    }
}

pub async fn candidate(context: CandidateBackend, generation: u64, candidate: Candidate) {
    let backend = &context;
    if !backend.shared.is_current(generation) {
        return;
    }
    let candidate = match candidate {
        #[cfg(test)]
        Candidate::Gate {
            started,
            release,
            finished,
            ..
        } => {
            let _ = started.send(());
            let _ = release.await;
            let _ = finished.send(());
            return;
        }
        Candidate::PreviewExistingEdit { tag, operation } => {
            edit_preview_service::candidate(
                backend,
                generation,
                tag,
                edit_preview_service::Edit::Parameters(operation),
            )
            .await;
            return;
        }
        Candidate::PreviewInputRepair { tag, repair } => {
            edit_preview_service::candidate(
                backend,
                generation,
                tag,
                edit_preview_service::Edit::Repair(repair),
            )
            .await;
            return;
        }
        candidate => candidate,
    };
    let mut prepared = None;
    let mut retained_scene = None;
    let mut preparation = None;
    let lease = if matches!(&candidate, Candidate::Preview { .. }) {
        match backend
            .shared
            .host_budget
            .admit_while(
                crate::budget::PREVIEW_RESERVATION,
                crate::budget::Kind::Preview,
                || {
                    backend.shared.is_current(generation)
                        && backend
                            .shared
                            .relieve_cpu_pressure(crate::budget::PREVIEW_RESERVATION)
                        && !backend
                            .shared
                            .stopping
                            .load(std::sync::atomic::Ordering::Acquire)
                },
            )
            .await
            .and_then(|lease| {
                lease.split_kinds(
                    crate::budget::PREVIEW_SOURCE_RESERVATION,
                    crate::budget::Kind::PreviewSource,
                    crate::budget::Kind::Preparation,
                )
            }) {
            Ok((source, prepared)) => {
                preparation = Some(Arc::new(prepared));
                Some(Arc::new(source))
            }
            Err(error) => {
                let Candidate::Preview { tag, .. } = candidate else {
                    return;
                };
                backend.shared.publish_candidate(
                    generation,
                    CandidateDelivery {
                        action: Action::PreviewFinished {
                            tag,
                            result: Err(error),
                        },
                        prepared: None,
                        lease: None,
                        scene: None,
                    },
                );
                return;
            }
        }
    } else {
        None
    };
    let action = match candidate {
        #[cfg(test)]
        Candidate::Gate { .. } => unreachable!("test gate routed above"),
        Candidate::PreviewExistingEdit { .. } | Candidate::PreviewInputRepair { .. } => {
            unreachable!("existing edit routed above")
        }
        Candidate::Evaluate {
            tag,
            expression,
            site,
        } => {
            let result = match check_tag(&backend.state, &tag).await {
                Ok(_) => api::evaluate_expression(&backend.state, expression, site.into())
                    .await
                    .map_err(|e| e.to_string()),
                Err(error) => Err(error),
            };
            Action::Evaluated { tag, result }
        }
        Candidate::Preview {
            tag,
            operation,
            sketch_id,
            snapshot,
        } => {
            crate::performance::record(
                crate::performance::Stage::PreviewSubmitted,
                tag.context.scene.session,
                [tag.sequence, tag.context.scene.scene, tag.gesture],
            );
            let _span = phase(crate::performance::Stage::PreviewService, &tag);
            let result = match preview(backend, &tag, *operation, sketch_id, snapshot).await {
                Ok(mut result) => {
                    let meshes = std::mem::take(&mut result.bodies);
                    match (lease.as_ref(), preparation.as_ref()) {
                        (Some(source), Some(preparation)) => match prepare_meshes(
                            backend,
                            generation,
                            tag.clone(),
                            meshes,
                            (source.clone(), preparation.clone()),
                        )
                        .await
                        {
                            Ok((meshes, packed, scene)) => {
                                result.bodies = meshes;
                                prepared = Some(packed);
                                retained_scene = Some(scene);
                                Ok(result)
                            }
                            Err(error) => Err(error),
                        },
                        _ => Err("Preview admission missing".into()),
                    }
                }
                Err(error) => Err(error),
            };
            Action::PreviewFinished { tag, result }
        }
    };
    backend.shared.publish_candidate(
        generation,
        CandidateDelivery {
            action,
            prepared,
            lease,
            scene: retained_scene,
        },
    );
}

type PackedMeshes = (
    Vec<onecad_app::dto::PreviewBodyDto>,
    onecad_render::PreparedScene,
    Arc<SceneSnapshot>,
);

async fn prepare_meshes(
    backend: &CandidateBackend,
    generation: u64,
    tag: onecad_interaction::RequestTag,
    meshes: Vec<onecad_app::dto::PreviewBodyDto>,
    guards: (Arc<crate::budget::Lease>, Arc<crate::budget::Lease>),
) -> Result<PackedMeshes, String> {
    backend.ensure_current(generation, &tag)?;
    let budget = backend
        .shared
        .preparation_budget()?
        .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES);
    let backend = backend.clone();
    tokio::task::spawn_blocking(move || {
        backend.ensure_current(generation, &tag)?;
        let scene = preview_scene(&backend, generation, &tag, &meshes, &guards.0)?;
        let mesh_bytes = meshes.iter().try_fold(0usize, |n, body| {
            n.checked_add(body.mesh.capacity())
                .ok_or("preview size overflow")
        })?;
        let geometry_bytes = scene.bodies().iter().try_fold(0usize, |sum, body| {
            sum.checked_add(body.geometry_bytes().map_err(|e| e.to_string())?)
                .ok_or("preview size overflow".to_string())
        })?;
        let source_bytes = scene
            .resident_bytes()
            .saturating_sub(geometry_bytes)
            .checked_add(mesh_bytes)
            .and_then(|n| n.checked_add(2 << 20))
            .ok_or("preview size overflow")?;
        guards.0.shrink_to(source_bytes)?;
        backend.ensure_current(generation, &tag)?;
        let _packing = phase(crate::performance::Stage::PreviewPacking, &tag);
        let retention = onecad_render::PreparationRetention {
            source: Some(guards.0.clone()),
            preparation: Some(guards.1.clone()),
        };
        let mut allocate = |bytes: u64| {
            let count = usize::try_from(bytes)
                .map_err(|_| onecad_render::RenderError::Budget("preparation size"))?;
            let guard = guards
                .1
                .carve_shared(count, crate::budget::Kind::PreparedGeometry)
                .map_err(|_| onecad_render::RenderError::Budget("preparation allocation"))?;
            Ok(guard as Arc<dyn Send + Sync>)
        };
        let packed = onecad_render::PreparedScene::new_cached_allocated(
            &scene,
            budget,
            &mut Default::default(),
            retention,
            &mut allocate,
        )
        .map_err(|error| error.to_string())?;
        guards.1.shrink_to(
            usize::try_from(packed.metadata_bytes()).map_err(|_| "preparation size overflow")?,
        )?;
        backend.ensure_current(generation, &tag)?;
        Ok((meshes, packed, Arc::new(scene)))
    })
    .await
    .map_err(|error| format!("preview preparation stopped: {error}"))?
}

async fn preview(
    backend: &CandidateBackend,
    tag: &onecad_interaction::RequestTag,
    operation: onecad_core::document::record::Operation,
    sketch: Option<String>,
    snapshot: onecad_core::ids::SnapshotId,
) -> Result<onecad_app::dto::PreviewResultDto, String> {
    let before = check_tag(&backend.state, tag).await?;
    let _kernel = phase(crate::performance::Stage::PreviewKernel, tag);
    let result = api::preview_operation(
        &backend.state,
        operation,
        tag.record_id,
        sketch,
        Some(snapshot),
        Lod::Coarse,
    )
    .await
    .map_err(|e| e.to_string())?;
    drop(_kernel);
    if api::preview_context(&backend.state)
        .await
        .map_err(|e| e.to_string())?
        != before
        || result.snapshot_id != before.snapshot_id.0
    {
        return Err("preview completion belongs to an obsolete worker head".into());
    }
    Ok(result)
}

fn preview_scene(
    backend: &CandidateBackend,
    generation: u64,
    tag: &onecad_interaction::RequestTag,
    meshes: &[onecad_app::dto::PreviewBodyDto],
    source: &Arc<crate::budget::Lease>,
) -> Result<SceneSnapshot, String> {
    let _span = phase(crate::performance::Stage::PreviewScene, tag);
    if meshes.len() > crate::budget::MAX_BODIES {
        return Err("Native preview body count budget exceeded".into());
    }
    let mut bytes = 0;
    let mut geometry_bytes = 0;
    for body in meshes {
        backend.ensure_current(generation, tag)?;
        crate::budget::charge(
            &mut bytes,
            body.mesh.len(),
            crate::budget::MAX_SOURCE_BYTES,
            "preview mesh",
        )?;
        let estimate = SceneBody::preflight_geometry_bytes(&body.mesh, MeshLimits::default())
            .map_err(|error| error.to_string())?;
        crate::budget::charge(
            &mut geometry_bytes,
            estimate,
            crate::budget::MAX_PREVIEW_SCENE_BYTES,
            "preview checked geometry",
        )?;
    }
    let bodies = meshes
        .iter()
        .map(|body| {
            backend.ensure_current(generation, tag)?;
            let id = body
                .body_id
                .parse()
                .map_err(|error| format!("preview body id: {error}"))?;
            let mut geometry = SceneBody::new(
                id,
                tag.sequence,
                Arc::from(body.mesh.as_slice()),
                MeshLimits::default(),
            )
            .map_err(|error| error.to_string())?;
            let bytes = geometry
                .geometry_bytes()
                .map_err(|error| error.to_string())?;
            let guard = source.carve_shared(bytes, crate::budget::Kind::PreviewGeometry)?;
            geometry
                .retain_geometry(guard)
                .map_err(|error| error.to_string())?;
            Ok(geometry)
        })
        .collect::<Result<Vec<_>, String>>()?;
    backend.ensure_current(generation, tag)?;
    SceneSnapshot::new(
        tag.context.scene,
        bodies,
        SceneLimits {
            max_resident_bytes: crate::budget::MAX_PREVIEW_SCENE_BYTES,
            max_bodies: crate::budget::MAX_BODIES,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())
}

fn phase(
    stage: crate::performance::Stage,
    tag: &onecad_interaction::RequestTag,
) -> crate::performance::Span {
    crate::performance::span(
        stage,
        tag.context.scene.session,
        [tag.sequence, tag.context.scene.scene, tag.gesture],
    )
}
