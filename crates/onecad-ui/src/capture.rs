//! Fenced application reads and CPU preparation happen off the native event loop.
use crate::{
    bridge::{Backend, Display, record_id},
    profiles::{ProfileBudget, decode_profiles_budgeted},
};
use onecad_app::api;
use onecad_core::ids::SketchId;
use onecad_scene::{MeshLimits, SceneBody, SceneIdentity, SceneLimits, SceneSnapshot};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub async fn load_display(backend: &mut Backend) -> Result<Display, String> {
    let epoch = backend
        .shared
        .display_epoch
        .load(std::sync::atomic::Ordering::Acquire);
    let lease = Arc::new(
        backend
            .shared
            .host_budget
            .reserve(crate::budget::DISPLAY_RESERVATION)?,
    );
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
    crate::projection_budget::check(&projection)?;
    for body in projection.bodies.values().filter(|body| body.visible) {
        let bytes = api::get_mesh(
            &backend.state,
            body.id.clone(),
            "medium".into(),
            Some(context.snapshot_id.0),
            Some(context.runtime_session.clone()),
        )
        .await
        .map_err(|e| e.to_string())?;
        crate::budget::charge(
            &mut mesh_bytes,
            bytes.len(),
            crate::budget::MAX_SOURCE_BYTES,
            "scene source mesh",
        )?;
        if bytes.is_empty() {
            return Err(format!("missing checked mesh for {}", body.name));
        }
        let body_id = body.id.parse().map_err(|e| format!("body id: {e}"))?;
        let previous = backend
            .previous_scene
            .as_ref()
            .and_then(|(scene, _)| scene.bodies().iter().find(|body| body.body_id() == body_id));
        bodies.push(
            SceneBody::new_reusing(
                body_id,
                projection.revision,
                Arc::from(bytes),
                MeshLimits::default(),
                previous,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    let mut profiles = Vec::new();
    let mut profile_budget = ProfileBudget::default();
    for sketch in projection.sketches.values().filter(|sketch| sketch.visible) {
        let id: SketchId = sketch.id.parse().map_err(|e| format!("sketch id: {e}"))?;
        let plane = api::get_sketch_plane(&backend.state, id)
            .await
            .map_err(|e| e.to_string())?;
        let regions = api::get_sketch_regions(&backend.state, sketch.id.clone())
            .await
            .map_err(|e| e.to_string())?;
        profiles.extend(decode_profiles_budgeted(
            id,
            plane,
            regions,
            &mut profile_budget,
        )?);
    }
    let mut records = Vec::new();
    let mut record_budget = crate::record_budget::RecordBudget::default();
    for feature in &projection.features {
        let record = api::get_operation_record(&backend.state, record_id(&feature.id)?)
            .await
            .map_err(|error| error.to_string())?;
        record_budget.charge(&record)?;
        if !onecad_interaction::numeric_parameters(&record.op).is_empty() {
            records.push(record);
        }
    }
    if api::preview_context(&backend.state)
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
    let prepared = onecad_render::PreparedScene::new_cached(
        &scene,
        backend
            .shared
            .preparation_budget()?
            .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES),
        &mut backend.preparation_cache,
    )
    .map_err(|e| e.to_string())?;
    // Preparation can succeed before the final publication fence rejects this capture.
    // Keep its source and CPU charge alive even when no Display reaches the UI.
    backend.preparation_lease = Some(lease.clone());
    if prepared.cpu_bytes() > crate::budget::MAX_PREPARED_BYTES {
        return Err("Native CPU preparation budget exceeded".into());
    }
    if backend.preparation_cache.source_bytes() > crate::budget::MAX_SOURCE_BYTES as u64
        || backend.preparation_cache.cpu_bytes() > crate::budget::MAX_PREPARED_BYTES
    {
        return Err("Native preparation cache budget exceeded".into());
    }
    if api::preview_context(&backend.state)
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
        context,
        scene,
        profiles,
        records,
        render_receipt,
        prepared: std::sync::Mutex::new(Some(prepared)),
    })
}
