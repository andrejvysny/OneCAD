use super::*;

#[tokio::test]
#[ignore = "private diagnostic requires production worker and Metal"]
async fn public_mixed_capture_completes() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let source = Path::new(
        "/private/tmp/onecad-public-fixtures-786438fb/public-mixed-10x20-input/assembly.onecad",
    );
    assert_eq!(
        onecad_library::blob::sha256_hex(&fs::read(source).unwrap()),
        "b570ac7efbb71d4c576b22813ea90693534cc9a255f7ce4cc5091b2bd9c5db59"
    );
    let worker = std::env::var_os("ONECAD_WORKER_PATH").unwrap();
    assert_eq!(
        onecad_library::blob::sha256_hex(&fs::read(worker).unwrap()),
        "4272493274b789539a0e731afe0d45084e344ccf646f46bec35af6561a3fcfb0"
    );
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let opened = intent(&mut backend, Request::Open(Some(source.to_owned()))).await?;
        eprintln!("open outcome: {opened:?}");
        if !matches!(opened, Outcome::Replaced { .. }) {
            return Err(format!("unexpected open outcome: {opened:?}"));
        }
        crate::services::load_ready_display(&mut backend).await
    })
    .await;
    eprintln!(
        "capture budget: {:?}",
        backend.shared.host_budget.snapshot()
    );
    match &result {
        Ok(Ok(display)) => eprintln!(
            "display: definitions={} occurrences={} bodies={}",
            display.assembly.definitions.len(),
            display.assembly.occurrences.len(),
            display.scene.bodies().len()
        ),
        _ => eprintln!(
            "capture did not complete; projection: {:?}",
            api::get_projection(&backend.state).await
        ),
    }
    let _ = api::close_document(&backend.state, backend.host.clone()).await;
    backend.state.retire_all();
    let display = result
        .expect("open and capture exceeded 60 seconds")
        .expect("open and capture failed");
    assert_eq!(display.assembly.definitions.len(), 11);
    assert_eq!(display.assembly.occurrences.len(), 20);
    assert_eq!(display.scene.bodies().len(), 20);
}

#[tokio::test]
#[ignore = "private diagnostic requires production worker and Metal"]
async fn compact_public_mixed_capture_completes() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let source = Path::new(
        "/private/tmp/onecad-public-fixtures-compact-candidate/public-mixed-10x20-input/assembly.onecad",
    );
    assert_eq!(
        onecad_library::blob::sha256_hex(&fs::read(source).unwrap()),
        "7c39c64fa89618fcb7cf40dcb2a230b0f3f68acd2d09f4fd7c8af58be006ae9d"
    );
    let worker = std::env::var_os("ONECAD_WORKER_PATH").unwrap();
    assert_eq!(
        onecad_library::blob::sha256_hex(&fs::read(worker).unwrap()),
        "4272493274b789539a0e731afe0d45084e344ccf646f46bec35af6561a3fcfb0"
    );
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let opened = intent(&mut backend, Request::Open(Some(source.to_owned()))).await?;
        eprintln!("open outcome: {opened:?}");
        if !matches!(opened, Outcome::Replaced { .. }) {
            return Err(format!("unexpected open outcome: {opened:?}"));
        }
        crate::services::load_ready_display(&mut backend).await
    })
    .await;
    eprintln!(
        "capture budget: {:?}",
        backend.shared.host_budget.snapshot()
    );
    match &result {
        Ok(Ok(display)) => eprintln!(
            "display: definitions={} occurrences={} bodies={}",
            display.assembly.definitions.len(),
            display.assembly.occurrences.len(),
            display.scene.bodies().len()
        ),
        _ => eprintln!(
            "capture did not complete; projection: {:?}",
            api::get_projection(&backend.state).await
        ),
    }
    let _ = api::close_document(&backend.state, backend.host.clone()).await;
    backend.state.retire_all();
    let display = result
        .expect("open and capture exceeded 60 seconds")
        .expect("open and capture failed");
    assert_eq!(display.assembly.definitions.len(), 11);
    assert_eq!(display.assembly.occurrences.len(), 20);
    assert_eq!(display.scene.bodies().len(), 20);
}
