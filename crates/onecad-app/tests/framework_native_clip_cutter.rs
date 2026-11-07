//! Editable numeric cutter derived from pinned Framework source geometry.
//! Imported geometry, interactive authoring, and source equivalence are not claimed.
#[path = "framework_native_clip_support/mod.rs"]
mod clip;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{api, worker::wire::body_id_wire};
use onecad_core::{
    document::record::{KnownOperation, Operation},
    edit::EditCommand,
    ids::{BodyId, RecordId},
};
use std::path::Path;

async fn volume(h: &Harness, body: BodyId) -> f64 {
    api::query_mass_properties(&h.state, body_id_wire(body), None)
        .await
        .unwrap()
        .volume
}
async fn no_imports(h: &Harness) {
    assert!(h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .import_blob_shas()
        .is_empty());
    let projection = h.projection().await;
    let kinds: Vec<_> = projection
        .features
        .iter()
        .map(|f| f.op_type.as_str())
        .collect();
    assert!(kinds
        .iter()
        .all(|k| ["Sketch", "Loft", "Extrude", "Revolve", "Boolean", "Sweep"].contains(k)));
    assert_eq!(kinds.iter().filter(|k| **k == "Sweep").count(), 2);
    for required in ["Sketch", "Extrude", "Revolve", "Boolean"] {
        assert!(kinds.contains(&required));
    }
}
async fn author(root: &Path) {
    let h = Harness::new(&root.join("author")).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let cutter = clip::build_cutter(&h).await;
    no_imports(&h).await;
    let before = volume(&h, cutter).await;
    assert!(before.is_finite() && before > 0.0);
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    assert_eq!(
        h.projection().await.bodies.len(),
        10,
        "undo restores sharp and all nine protectors"
    );
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    assert_eq!(h.projection().await.bodies.len(), 1);
    assert_eq!(volume(&h, cutter).await.to_bits(), before.to_bits());
    let path = root.join("clip-cutter.onecad");
    let saved = api::save_document(
        &h.state,
        h.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    std::fs::write(
        root.join("cutter-state.json"),
        serde_json::to_vec(
            &serde_json::json!({"body":cutter.to_string(),"volumeBits":before.to_bits()}),
        )
        .unwrap(),
    )
    .unwrap();
    if let Some(path) = std::env::var_os("ONECAD_FRAMEWORK_CLIP_CUTTER_OUT") {
        let path = std::path::PathBuf::from(path);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        api::export_step_file(
            &h.state,
            h.host.clone(),
            Some(path.to_string_lossy().into_owned()),
        )
        .await
        .unwrap()
        .unwrap();
    }
    h.close().await;
}
async fn reopen(root: &Path) {
    let h = Harness::new(&root.join("reopen")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join("clip-cutter.onecad")
            .to_string_lossy()
            .into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await;
    no_imports(&h).await;
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("cutter-state.json")).unwrap()).unwrap();
    let body: BodyId = state["body"].as_str().unwrap().parse().unwrap();
    assert_eq!(h.projection().await.bodies.len(), 1);
    assert_eq!(
        volume(&h, body).await.to_bits(),
        state["volumeBits"].as_u64().unwrap()
    );
    let projection = h.projection().await;
    let id: RecordId = projection
        .features
        .iter()
        .find(|f| f.op_type == "Sweep")
        .unwrap()
        .id
        .parse()
        .unwrap();
    let record = api::get_operation_record(&h.state, id).await.unwrap();
    let Operation::Known(KnownOperation::Sweep(mut params)) = record.op else {
        panic!("typed Sweep")
    };
    let range = params.path_range.as_mut().unwrap();
    range.end_radians -= (range.end_radians - range.start_radians) * 0.01;
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: id,
            op: Operation::Known(KnownOperation::Sweep(params)),
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&result.projection).await;
    assert_eq!(h.projection().await.bodies.len(), 1);
    no_imports(&h).await;
    h.close().await;
}
#[test]
fn native_framework_clip_cutter_history_save_reopen() {
    host::required_worker();
    match std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE")
        .ok()
        .as_deref()
    {
        Some(phase @ ("clip-author" | "clip-reopen")) => {
            let root = std::path::PathBuf::from(
                std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap(),
            );
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    if phase == "clip-author" {
                        author(&root).await;
                    } else {
                        reopen(&root).await;
                    }
                });
        }
        _ => {
            let root = tempfile::tempdir().unwrap();
            for phase in ["clip-author", "clip-reopen"] {
                host::run_isolated_phase(
                    root.path(),
                    phase,
                    "native_framework_clip_cutter_history_save_reopen",
                );
            }
        }
    }
}
