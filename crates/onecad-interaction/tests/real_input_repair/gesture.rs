use super::*;

#[test]
fn repair_commit_refuses_an_active_gesture_without_revision_change() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("repair-gesture") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(refusal(&root));
    } else {
        let root = tempfile::tempdir().unwrap();
        host::run_isolated_phase(
            root.path(),
            "repair-gesture",
            "gesture::repair_commit_refuses_an_active_gesture_without_revision_change",
        );
    }
}
async fn refusal(root: &Path) {
    let h = Harness::new(root).await;
    build(&h).await;
    let repair = stage(&h).await;
    let preview = api::preview_input_repair(&h.state, &repair, onecad_core::regen::Lod::Fine)
        .await
        .unwrap();
    assert!(api::repair_preview_ready(&preview));
    let before = h.projection().await;
    api::begin_gesture(
        &h.state,
        profile::sketch_id().to_string(),
        Uuid::from_u128(0x273100).to_string(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        h.projection().await.revision,
        before.revision,
        "BeginGesture cannot move the authoring fence"
    );
    let result = api::commit_input_repair(&h.state, h.host.clone(), &repair).await;
    assert!(
        result.is_err(),
        "Repair commit must respect active gesture ownership: {result:?}"
    );
    assert_eq!(h.projection().await, before);
    api::cancel_gesture(&h.state).await.unwrap();
    let cancelled = h.projection().await;
    assert_eq!(cancelled.revision, before.revision);
    assert_eq!(cancelled.undo_depth, before.undo_depth);
    assert_eq!(cancelled.features, before.features);
    assert_eq!(cancelled.bodies, before.bodies);
    receipt_rollback(&h).await;
    h.close().await;
}

async fn receipt_rollback(h: &Harness) {
    let original = api::get_operation_record(&h.state, fillet()).await.unwrap();
    let before = h.projection().await;
    let repair = stage(h).await;
    let preview = api::preview_input_repair(&h.state, &repair, onecad_core::regen::Lod::Fine)
        .await
        .unwrap();
    assert!(api::repair_preview_ready(&preview));
    let result = api::commit_input_repair(&h.state, h.host.clone(), &repair)
        .await
        .unwrap();
    h.settled(&result.projection).await;
    assert_volume(h, expected()).await;
    let receipt = result
        .rollback_token
        .expect("repair must issue guarded receipt");
    let rollback = api::rollback_failed_operation(&h.state, h.host.clone(), receipt.clone())
        .await
        .unwrap();
    assert!(rollback.rolled_back && rollback.regen_expected);
    let report = h.terminal(&rollback.projection).await;
    assert_eq!(report.repair_steps, [fillet().to_string()]);
    assert_eq!(
        api::get_operation_record(&h.state, fillet()).await.unwrap(),
        original
    );
    assert_eq!(h.projection().await.undo_depth, before.undo_depth);
    assert_volume(h, 100_000.0).await;
    let duplicate = api::rollback_failed_operation(&h.state, h.host.clone(), receipt)
        .await
        .unwrap();
    assert!(!duplicate.rolled_back, "a receipt is consumed once");
}
