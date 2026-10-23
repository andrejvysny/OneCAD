//! Prefix reads share the real application runtime without requiring renderer admission.
use super::super::*;
use onecad_core::document::record::MirrorBodyParams;

const MIRROR: u128 = 0x5E1;

async fn read(state: &Arc<AppState>, record: u128) -> api::EditOperandContext {
    let fence = api::preview_context(state).await.unwrap();
    let mut read = api::start_edit_operand_context(state.clone(), fence, rec(record))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), &mut read.receiver)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

fn records() -> Vec<OperationRecord> {
    let mut records = part_with_dependent();
    records.insert(
        2,
        OperationRecord::new(
            rec(MIRROR),
            0,
            "Mirror",
            Operation::Known(KnownOperation::MirrorBody(MirrorBodyParams {
                source_body: Some(body(EXTRUDE_A)),
                plane_point: Vec3::new_unchecked(40.0, 0.0, 0.0),
                plane_normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
                fuse_with_original: false,
                extra: Default::default(),
            })),
        ),
    );
    records
}

async fn assert_prefixes(state: &Arc<AppState>) {
    let before = head_state(state).await;
    let first = read(state, SKETCH_A).await;
    assert!(first.bodies.is_empty() && first.operands.is_empty() && first.body_bounds.is_empty());
    let extrude = read(state, EXTRUDE_A).await;
    assert!(
        extrude.bodies.is_empty(),
        "the edited Extrude output is not a target"
    );
    let mirror = read(state, MIRROR).await;
    assert_eq!(mirror.bodies, vec![body(EXTRUDE_A)]);
    assert!(mirror.refusals.is_empty(), "{:?}", mirror.refusals);
    assert_eq!(mirror.body_bounds.len(), 1);
    let bounds = &mirror.body_bounds[0];
    assert_eq!(bounds.body, body(EXTRUDE_A));
    assert!(
        (bounds.max[2] - 10.0).abs() < 1e-5,
        "prefix excludes downstream boss height25"
    );
    assert_eq!(head_state(state).await, before);
}

#[tokio::test]
async fn real_runtime_prefix_operands_exclude_outputs_and_survive_cold_reopen() {
    let Some((wm, state)) = document(records()).await else {
        return;
    };
    let state = Arc::new(state);
    assert_prefixes(&state).await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("prefix.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(
            &path,
            SaveMeta {
                app_version: "edit-operand-test".into(),
                occt_fingerprint: None,
                created: "2026-10-22T00:00:00Z".into(),
                modified: "2026-10-22T00:00:00Z".into(),
            },
        )
        .unwrap();
    wm.shutdown().await;
    drop(state);
    let wm = spawn_worker(real_worker().unwrap()).await;
    let state = Arc::new(app_state_over(&wm));
    let (engine, meshes, solver) = state.make_backend();
    let mut runtime = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
    let report = runtime
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    *state.runtime.lock().await = Some(runtime);
    assert_prefixes(&state).await;
    wm.shutdown().await;
}
