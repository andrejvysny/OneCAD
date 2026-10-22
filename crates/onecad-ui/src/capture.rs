//! Fenced application reads and CPU preparation happen off the native event loop.
use crate::bridge::{Backend, Display, record_id};
use onecad_app::api;
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
            "fine".into(),
            // Worker snapshot IDs restart after a session reset; they are not
            // model publication generations. The surrounding context checks fence
            // this current-mesh read before and after the complete capture.
            None,
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
    let (profiles, profile_failures) = crate::profile_capture::load(backend, &projection).await?;
    let mut records = Vec::new();
    let mut record_budget = crate::record_budget::RecordBudget::default();
    for feature in &projection.features {
        let record = api::get_operation_record(&backend.state, record_id(&feature.id)?)
            .await
            .map_err(|error| error.to_string())?;
        record_budget.charge(&record)?;
        if matches!(
            record.op,
            onecad_core::document::record::Operation::Known(
                onecad_core::document::record::KnownOperation::Sketch(_)
            )
        ) || !onecad_interaction::numeric_parameters(&record.op).is_empty()
        {
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
    let budget = backend
        .shared
        .preparation_budget()?
        .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES);
    let mut cache = backend.shared.take_preparation_cache();
    let prepared = onecad_render::PreparedScene::new_cached(&scene, budget, &mut cache)
        .map_err(|e| e.to_string());
    let cache_bytes = (cache.source_bytes(), cache.cpu_bytes());
    backend.shared.restore_preparation_cache(cache);
    let prepared = prepared?;
    // Preparation can succeed before the final publication fence rejects this capture.
    // Keep its source and CPU charge alive even when no Display reaches the UI.
    backend.preparation_lease = Some(lease.clone());
    if prepared.cpu_bytes() > crate::budget::MAX_PREPARED_BYTES {
        return Err("Native CPU preparation budget exceeded".into());
    }
    if cache_bytes.0 > crate::budget::MAX_SOURCE_BYTES as u64
        || cache_bytes.1 > crate::budget::MAX_PREPARED_BYTES
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
        profile_failures,
        records,
        render_receipt,
        prepared: std::sync::Mutex::new(Some(prepared)),
    })
}
