use super::*;
use std::path::Path;

pub(crate) async fn replay_checkpoint(h: &Harness, body: BodyId, root: &Path, name: &str) -> bool {
    if std::env::var("ONECAD_FRAMEWORK_REPLAY_CHECKPOINT").as_deref() != Ok(name) {
        return false;
    }
    let before = super::mesh_signature(h, body).await;
    export(h, "ONECAD_FRAMEWORK_DIAGNOSTIC_OUT").await;
    let path = root.join("framework-checkpoint.onecad");
    let saved = api::save_document(
        &h.state,
        h.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
    h.close().await;
    let reopened = Harness::new(&root.join("checkpoint-reopen")).await;
    api::open_document(
        &reopened.state,
        reopened.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    reopened.settled(&reopened.projection().await).await;
    export(&reopened, "ONECAD_FRAMEWORK_REOPEN_DIAGNOSTIC_OUT").await;
    let after = super::mesh_signature(&reopened, body).await;
    eprintln!("Framework checkpoint {name}: before={before:?}, after={after:?}");
    assert_eq!(before, after);
    reopened.close().await;
    true
}

async fn export(h: &Harness, variable: &str) {
    if let Ok(path) = std::env::var(variable) {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        api::export_step_file(&h.state, h.host.clone(), Some(path))
            .await
            .unwrap();
    }
}
