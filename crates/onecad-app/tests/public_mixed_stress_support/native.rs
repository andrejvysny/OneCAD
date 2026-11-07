use super::*;

pub(super) async fn build_hole_plate(path: &Path, root: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qa_plate_hole.onecad");
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&source).unwrap())),
        "a83d1bbac05941030fc97703eea69e9775b2c29326d8c6a91782383c1183d9ec"
    );
    let h = Harness::new(root).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        source.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(Duration::from_secs(15))
            .await
    );
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
}
pub(super) async fn build_constrained(path: &Path, root: &Path) {
    let h = Harness::new(root).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(Duration::from_secs(15))
            .await
    );
    let region = profile::create(&h).await;
    let p = api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddOperation {
            record: OperationRecord::new(
                profile::extrude_id(),
                1,
                "Synthetic constrained plate",
                profile::extrude(&region, 6.0),
            ),
            at_cursor: true,
        },
    )
    .await
    .unwrap();
    let settled = h.settled(&p).await;
    assert_eq!(settled.bodies.len(), 1);
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
}
