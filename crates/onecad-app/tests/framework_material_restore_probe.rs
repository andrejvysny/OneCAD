//! Private native-history experiment; not full reference qualification.
#[path = "framework_native_clip_support/mod.rs"]
mod clip;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{api, worker::wire::body_id_wire};
use sha2::{Digest, Sha256};

#[tokio::test(flavor = "multi_thread")]
async fn original_material_restoration_uses_native_history() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            "onecad_app::document_runtime=debug,onecad_lib::document_runtime=debug,onecad_app::worker=debug,onecad_lib::worker=debug,onecad_protocol=debug",
        )
        .with_test_writer()
        .try_init();
    host::required_worker();
    let source =
        std::path::PathBuf::from(std::env::var_os("ONECAD_FRAMEWORK_RESTORE_SOURCE").unwrap());
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&source).unwrap()));
    assert_eq!(
        digest,
        "acf206fff9aebb61284d3939201b53947a70f7d07e3fe22dc72698421f316aa8"
    );
    let area = tempfile::tempdir().unwrap();
    let h = Harness::new(area.path()).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        source.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await;
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let body = projection.bodies.keys().next().unwrap().parse().unwrap();
    assert!(h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .import_blob_shas()
        .is_empty());
    let before = api::query_mass_properties(&h.state, body_id_wire(body), None)
        .await
        .unwrap();
    eprintln!("original native volume {}", before.volume);
    let output =
        std::env::var_os("ONECAD_FRAMEWORK_RESTORE_OUTPUT_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir(path).unwrap();
    }
    let output = output.as_deref().unwrap_or(area.path());
    let checkpoint = output.join("working.onecad");
    assert!(
        api::save_document(
            &h.state,
            h.host.clone(),
            Some(checkpoint.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    clip::cut_and_restore_original_material(&h, body, &checkpoint).await;
    let after = api::query_mass_properties(&h.state, body_id_wire(body), None)
        .await
        .unwrap();
    eprintln!(
        "restored native volume {}, area {}",
        after.volume, after.surface_area
    );
    assert!(after.volume > 0.0 && after.volume < before.volume);
    assert!(h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .import_blob_shas()
        .is_empty());
    let path = output.join("restored.onecad");
    assert!(
        api::save_document(
            &h.state,
            h.host.clone(),
            Some(path.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    h.close().await;
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(source).unwrap())),
        digest
    );
}
