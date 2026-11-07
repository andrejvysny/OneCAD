//! Fenced application reads and CPU preparation happen off the native event loop.
mod assembly;
use crate::bridge::{Backend, Display, record_id};
use onecad_app::api;
use onecad_scene::{MeshLimits, SceneBody, SceneIdentity, SceneLimits, SceneSnapshot};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub async fn load_display(backend: &mut Backend) -> Result<Display, String> {
    let result = load_display_inner(backend).await;
    crate::document_state::refresh(backend).await;
    result
}

async fn load_display_inner(backend: &mut Backend) -> Result<Display, String> {
    let epoch = backend
        .shared
        .display_epoch
        .load(std::sync::atomic::Ordering::Acquire);
    let (source, preparation) = backend
        .shared
        .host_budget
        .admit_while(
            crate::budget::DISPLAY_RESERVATION,
            crate::budget::Kind::Display,
            || {
                let current = !backend
                    .shared
                    .stopping
                    .load(std::sync::atomic::Ordering::Acquire)
                    && backend
                        .shared
                        .display_epoch
                        .load(std::sync::atomic::Ordering::Acquire)
                        == epoch;
                if current
                    && !backend
                        .shared
                        .host_budget
                        .has_capacity(crate::budget::DISPLAY_RESERVATION)
                {
                    // Reused geometry can keep an older admission in this cache.
                    // Live prepared scenes and GPU sources retain their own guards.
                    drop(backend.shared.take_preparation_cache());
                }
                current
            },
        )
        .await?
        .split_kinds(
            crate::budget::DISPLAY_SOURCE_RESERVATION,
            crate::budget::Kind::DisplaySource,
            crate::budget::Kind::Preparation,
        )?;
    let mut source_pool = crate::budget::LeasePool::new(
        source,
        crate::budget::Kind::DisplayGeometry,
        crate::budget::Kind::DisplaySource,
    );
    let preparation = Arc::new(preparation);
    let projection = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    let context = api::preview_context(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    if context.runtime_session != projection.runtime_session
        || context.revision.0 != projection.revision
    {
        return Err("document changed while capturing scene".into());
    }
    let mut bodies = Vec::new();
    let mut mesh_bytes = 0;
    let mut geometry_budget = 0;
    let projection_bytes = crate::projection_budget::check(&projection)?;
    for body in projection.bodies.values().filter(|body| body.visible) {
        let bytes = api::get_mesh_shared_bounded(
            &backend.state,
            body.id.clone(),
            "fine".into(),
            // Worker snapshot IDs restart after a session reset; they are not
            // model publication generations. The surrounding context checks fence
            // this current-mesh read before and after the complete capture.
            None,
            Some(context.runtime_session.clone()),
            crate::budget::MAX_SOURCE_BYTES.saturating_sub(mesh_bytes),
        )
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("missing checked mesh for {}", body.name))?;
        crate::budget::charge(
            &mut mesh_bytes,
            bytes.len(),
            crate::budget::MAX_SOURCE_BYTES,
            "scene source mesh",
        )?;
        if bytes.is_empty() {
            return Err(format!("missing checked mesh for {}", body.name));
        }
        let estimate = SceneBody::preflight_geometry_bytes(bytes.as_slice(), MeshLimits::default())
            .map_err(|e| e.to_string())?;
        crate::budget::charge(
            &mut geometry_budget,
            estimate,
            crate::budget::MAX_DISPLAY_SCENE_BYTES,
            "scene checked geometry",
        )?;
        let body_id = body.id.parse().map_err(|e| format!("body id: {e}"))?;
        let previous = backend
            .previous_scene
            .as_ref()
            .and_then(|(scene, _)| scene.bodies().iter().find(|body| body.body_id() == body_id));
        let mut captured = SceneBody::new_reusing(
            body_id,
            projection.revision,
            Arc::from(bytes.as_slice()),
            MeshLimits::default(),
            previous,
        )
        .map_err(|e| e.to_string())?;
        if captured.geometry_retention().is_none() {
            let bytes = captured.geometry_bytes().map_err(|e| e.to_string())?;
            captured
                .retain_geometry(source_pool.carve(bytes)?)
                .map_err(|e| e.to_string())?;
        }
        bodies.push(captured);
    }
    let (assembly_generation, assembly_bytes, assembly_projection) = assembly::capture(
        backend,
        &context.runtime_session,
        &mut bodies,
        &mut mesh_bytes,
        &mut geometry_budget,
        &mut source_pool,
    )
    .await?;
    let mut records = Vec::new();
    let mut record_budget = crate::record_budget::RecordBudget::default();
    for feature in &projection.features {
        let record = api::get_operation_record(&backend.state, record_id(&feature.id)?)
            .await
            .map_err(|error| error.to_string())?;
        record_budget.charge(&record)?;
        records.push(record);
    }
    let profiles = crate::profile_capture::load(backend, &projection, &records).await?;
    if !assembly::unchanged(backend, &context.runtime_session, assembly_generation).await?
        || api::preview_context(&backend.state)
            .await
            .map_err(|e| e.to_string())?
            != context
    {
        return Err("document head changed while capturing scene".into());
    }
    backend.scene_revision += 1;
    let mut hash = DefaultHasher::new();
    context.runtime_session.hash(&mut hash);
    let identity = SceneIdentity {
        session: hash.finish(),
        scene: backend.scene_revision,
    };
    let scene = Arc::new(
        SceneSnapshot::new(
            identity,
            bodies,
            SceneLimits {
                max_resident_bytes: crate::budget::MAX_DISPLAY_SCENE_BYTES,
                max_bodies: crate::budget::MAX_BODIES,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
    );
    let geometry_bytes =
        scene
            .bodies()
            .iter()
            .enumerate()
            .try_fold(0usize, |sum, (index, body)| {
                if scene.bodies()[..index]
                    .iter()
                    .any(|other| other.shares_geometry(body))
                {
                    return Ok(sum);
                }
                sum.checked_add(body.geometry_bytes().map_err(|e| e.to_string())?)
                    .ok_or("scene geometry overflow".to_string())
            })?;
    let retained_source = scene
        .resident_bytes()
        .saturating_sub(geometry_bytes)
        .checked_add(projection_bytes)
        .and_then(|v| v.checked_add(record_budget.bytes()))
        .and_then(|v| v.checked_add(profiles.charged_bytes))
        .and_then(|v| v.checked_add(assembly_bytes))
        .ok_or("scene retention size overflow")?;
    let lease = source_pool.finish(retained_source)?;
    let retention = onecad_render::PreparationRetention {
        source: Some(lease.clone()),
        preparation: Some(preparation.clone()),
    };
    let budget = backend
        .shared
        .preparation_budget()?
        .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES);
    let mut cache = backend.shared.take_preparation_cache();
    let mut allocate = |bytes: u64| {
        let count = usize::try_from(bytes)
            .map_err(|_| onecad_render::RenderError::Budget("preparation size"))?;
        let guard = preparation
            .carve_shared(count, crate::budget::Kind::PreparedGeometry)
            .map_err(|_| onecad_render::RenderError::Budget("preparation allocation"))?;
        Ok(guard as Arc<dyn Send + Sync>)
    };
    let prepared = onecad_render::PreparedScene::new_cached_allocated(
        &scene,
        budget,
        &mut cache,
        retention,
        &mut allocate,
    )
    .map_err(|e| e.to_string());
    let cache_bytes = (cache.source_bytes(), cache.cpu_bytes());
    let cache_metadata = cache.metadata_bytes();
    backend.shared.restore_preparation_cache(cache);
    let prepared = prepared?;
    if prepared.cpu_bytes() > crate::budget::MAX_PREPARED_BYTES {
        return Err("Native CPU preparation budget exceeded".into());
    }
    if cache_bytes.0 > crate::budget::MAX_SOURCE_BYTES as u64
        || cache_bytes.1 > crate::budget::MAX_PREPARED_BYTES
    {
        return Err("Native preparation cache budget exceeded".into());
    }
    preparation.shrink_to(
        usize::try_from(prepared.metadata_bytes().saturating_add(cache_metadata))
            .map_err(|_| "preparation size overflow")?,
    )?;
    if !assembly::unchanged(backend, &context.runtime_session, assembly_generation).await?
        || api::preview_context(&backend.state)
            .await
            .map_err(|error| error.to_string())?
            != context
        || backend
            .shared
            .display_epoch
            .load(std::sync::atomic::Ordering::Acquire)
            != epoch
    {
        return Err("document changed during CPU scene preparation".into());
    }
    let render_receipt = backend
        .shared
        .render_receipt
        .lock()
        .ok()
        .and_then(|receipt| {
            receipt.as_ref()?.capture(
                &projection.document_id,
                &context.runtime_session,
                context.revision.0,
                context.snapshot_id.0,
                scene.bodies().iter().map(|body| body.body_id().to_string()),
            )
        });
    backend.previous_scene = Some((scene.clone(), lease.clone()));
    Ok(Display {
        epoch,
        _lease: lease,
        projection,
        assembly: assembly_projection,
        context,
        scene,
        profiles: profiles.profiles,
        profile_anchors: profiles.anchors,
        profile_failures: profiles.failures,
        records,
        render_receipt,
        prepared: std::sync::Mutex::new(Some(prepared)),
    })
}
