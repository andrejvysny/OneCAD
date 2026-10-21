use super::*;
use onecad_app::{api, sketch_candidate::SketchCandidateControl};

const OTHER_SKETCH: u128 = 0xFA10;
const OTHER_EXTRUDE: u128 = 0xFA11;

async fn base() -> (WorkerManager, AppState, BodyId, String) {
    let wm = spawn_worker(real_worker().expect("required worker")).await;
    let state = app_state_over(&wm);
    let (engine, meshes, solver) = state.make_backend();
    let mut runtime = DocumentRuntime::new_blank(engine, meshes, solver);
    for record in box_records(SketchId::new(), 10.0) {
        add_op(&mut runtime, record);
    }
    let other = rect_sketch(SketchId::new(), 0xB000, 500.0, 500.0, 10.0, 10.0);
    add_op(&mut runtime, sketch_record(OTHER_SKETCH, &other));
    add_op(&mut runtime, extrude_record(OTHER_EXTRUDE, other.id, 8.0));
    published(&regen_all(&mut runtime).await, "drafted entry host");
    let body = body_of(EXTRUDE_A);
    let mesh = body_mesh(&mut runtime, body).await;
    let view = validate_mesh_blob(&mesh).unwrap();
    let key = face_probes(&view, &mesh)
        .into_iter()
        .find(|face| face.normal[2].abs() > 0.01 && face.normal[2].abs() < 0.99)
        .expect("oriented planar side")
        .topo_key;
    let dir = tempfile::tempdir().unwrap();
    runtime
        .save(&dir.path().join("entry.onecad"), save_meta())
        .unwrap();
    *state.runtime.lock().await = Some(runtime);
    (wm, state, body, key)
}

async fn prepare(
    state: &AppState,
    body: BodyId,
    key: &str,
    id: SketchId,
) -> api::PreparedFaceSketch {
    api::prepare_sketch_on_face(
        state,
        api::preview_context(state).await.unwrap(),
        body_id_wire(body),
        String::new(),
        Some(key.into()),
        id.to_string(),
        "Face".into(),
    )
    .await
    .unwrap()
}

async fn unchanged(state: &AppState) -> (serde_json::Value, usize, usize) {
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    (
        serde_json::to_value(runtime.projection()).unwrap(),
        runtime.undo_depth(),
        runtime.redo_depth(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_drop_and_cancel_leave_no_sketch_binding_history_or_dirty_state() {
    if real_worker().is_none() {
        return;
    }
    let (wm, state, body, key) = base().await;
    let before = unchanged(&state).await;
    assert!(api::prepare_sketch_on_face(
        &state,
        api::preview_context(&state).await.unwrap(),
        body_id_wire(body),
        "el_wrong_identity".into(),
        Some(key.clone()),
        SketchId::new().to_string(),
        "Mismatch".into()
    )
    .await
    .is_err());
    assert_eq!(unchanged(&state).await, before);
    let prepared = prepare(&state, body, &key, SketchId::new()).await;
    assert_eq!(unchanged(&state).await, before);
    drop(prepared);
    let prepared = prepare(&state, body, &key, SketchId::new()).await;
    let control = SketchCandidateControl::default();
    assert!(control.cancel());
    assert!(
        api::commit_sketch_on_face_controlled(&state, prepared, control)
            .await
            .is_err()
    );
    assert_eq!(unchanged(&state).await, before);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        saved_document(&state, &dir.path().join("cancel.onecad"))
            .await
            .elements
            .len(),
        0
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entry_commit_rejects_revision_and_worker_epoch() {
    if real_worker().is_none() {
        return;
    }
    let (wm, state, body, key) = base().await;
    let old = prepare(&state, body, &key, SketchId::new()).await;
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .unwrap()
            .apply(EditCommand::AddSketch {
                sketch: Sketch::on_world_plane(SketchId::new(), "Other", WorldPlane::XY),
            })
            .unwrap();
    }
    let changed = unchanged(&state).await;
    assert!(api::commit_sketch_on_face(&state, old).await.is_err());
    assert_eq!(unchanged(&state).await, changed);
    let old = prepare(&state, body, &key, SketchId::new()).await;
    let epoch = api::preview_context(&state).await.unwrap().worker_epoch;
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .on_worker_restart(onecad_core::ids::WorkerEpoch(epoch.0 + 1));
    let restarted = unchanged(&state).await;
    assert!(api::commit_sketch_on_face(&state, old).await.is_err());
    assert_eq!(unchanged(&state).await, restarted);
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_entry_cannot_attach_to_a_reopened_copy_of_the_same_document() {
    if real_worker().is_none() {
        return;
    }
    let (wm, state, body, key) = base().await;
    let displaced = prepare(&state, body, &key, SketchId::new()).await;
    let _replacement_facets = state.make_backend();
    let unchanged_provider = unchanged(&state).await;
    assert!(api::commit_sketch_on_face(&state, displaced).await.is_err());
    assert_eq!(unchanged(&state).await, unchanged_provider);
    let prepared = prepare(&state, body, &key, SketchId::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same.onecad");
    let document = saved_document(&state, &path).await;
    let (engine, meshes, solver) = state.make_backend();
    let reopened = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
    assert_eq!(reopened.document_uuid(), document.id);
    *state.runtime.lock().await = Some(reopened);
    let before = unchanged(&state).await;
    assert!(api::commit_sketch_on_face(&state, prepared).await.is_err());
    assert_eq!(unchanged(&state).await, before);
    wm.retire();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oriented_face_admits_identity_and_locked_boundary_once_and_reopens() {
    if real_worker().is_none() {
        return;
    }
    let (wm, state, body, key) = base().await;
    let id = SketchId::new();
    let prepared = prepare(&state, body, &key, id).await;
    let before = unchanged(&state).await;
    let control = SketchCandidateControl::default();
    api::commit_sketch_on_face_controlled(&state, prepared, control.clone())
        .await
        .unwrap();
    assert!(!control.cancel());
    let after = unchanged(&state).await;
    assert_eq!(after.1, before.1 + 1);
    let sketch = api::get_sketch_snapshot(&state, id).await.unwrap();
    assert_ne!(sketch.plane, WorldPlane::XY.plane());
    assert!(sketch
        .entities()
        .iter()
        .all(|entity| entity.is_reference_locked()));
    assert!(sketch.host_face().unwrap().primary.is_some());
    assert!(sketch.host_face().unwrap().intent.is_some());
    state_enter(&state, id).await;
    assert_eq!(state_finish(&state, id).await.regions.len(), 1);
    let report = state_regen(&state).await;
    published(&report, "oriented support consumed through regeneration");
    assert_eq!(
        state
            .runtime
            .lock()
            .await
            .as_mut()
            .unwrap()
            .rebind_persisted_elements()
            .await,
        (1, 0)
    );
    let primary = sketch.host_face().unwrap().primary.as_ref().unwrap();
    assert!(ElementQuery::query_element(
        &wm,
        SnapshotId(report.snapshot_id),
        body,
        primary.element.as_str()
    )
    .await
    .unwrap()
    .is_some());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oriented.onecad");
    let document = saved_document(&state, &path).await;
    assert_eq!(document.elements.len(), 1);
    wm.retire();
    let fresh = spawn_worker(real_worker().unwrap()).await;
    let fresh_state = app_state_over(&fresh);
    let (engine, meshes, solver) = fresh_state.make_backend();
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
    published(
        &regen_all(&mut reopened).await,
        "fresh worker oriented support reopen",
    );
    assert_eq!(reopened.rebind_persisted_elements().await, (1, 0));
    let restored = reopened.sketch_snapshot(id, "reopen").unwrap();
    assert_eq!(restored.attachment, sketch.attachment);
    for (actual, expected) in [
        (restored.plane.origin, sketch.plane.origin),
        (restored.plane.x_axis, sketch.plane.x_axis),
        (restored.plane.y_axis, sketch.plane.y_axis),
        (restored.plane.normal, sketch.plane.normal),
    ] {
        assert!((actual.x - expected.x).abs() < 1e-12);
        assert!((actual.y - expected.y).abs() < 1e-12);
        assert!((actual.z - expected.z).abs() < 1e-12);
    }
    fresh.retire();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oriented_face_add_and_cut_preserve_the_other_body() {
    if real_worker().is_none() {
        return;
    }
    for (mode, distance) in [(BooleanMode::Add, 3.0), (BooleanMode::Cut, -3.0)] {
        let (wm, state, body, key) = base().await;
        let id = SketchId::new();
        let prepared = prepare(&state, body, &key, id).await;
        api::commit_sketch_on_face(&state, prepared).await.unwrap();
        state_enter(&state, id).await;
        let region = state_finish(&state, id).await.regions.remove(0);
        let before = state_mesh(&state, body).await;
        let other = body_of(OTHER_EXTRUDE);
        let other_before = state_mesh(&state, other).await;
        state_add_op(
            &state,
            host_extrude_record(OP_TAIL, id, &region.region_id, distance, mode, body),
        )
        .await;
        published(
            &state_regen(&state).await,
            "oriented face explicit boolean target",
        );
        let after = state_mesh(&state, body).await;
        let volume = |mesh: &[u8]| mesh_volume(&validate_mesh_blob(mesh).unwrap(), mesh);
        assert!((volume(&after) - volume(&before)) * distance > 0.0);
        let other_after = state_mesh(&state, other).await;
        assert_eq!(positions(&other_before), positions(&other_after));
        assert_eq!(volume(&other_before), volume(&other_after));
        wm.shutdown().await;
    }
}

fn positions(mesh: &[u8]) -> &[u8] {
    let view = validate_mesh_blob(mesh).unwrap();
    let section = view.section(SEC_POSITIONS).unwrap();
    &mesh[section.offset as usize..(section.offset + section.byte_len) as usize]
}
