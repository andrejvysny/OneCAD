use super::*;
use onecad_app::dto::FeatureStatus::{self, Dirty, Error, Ok};
use onecad_core::document::variables::Variable;
use onecad_core::edit::VisibilityTarget;
use onecad_core::ids::VariableId;

fn statuses(rt: &DocumentRuntime) -> Vec<FeatureStatus> {
    rt.projection().features.iter().map(|f| f.status).collect()
}

async fn valid_runtime() -> Option<(DocumentRuntime, WorkerManager)> {
    let wm = spawn_worker(real_worker()?).await;
    let mut rt = runtime_over(&wm);
    build_prefix(&mut rt);
    build_suffix(&mut rt);
    published(&regen(&mut rt, 0).await, "status baseline");
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    Some((rt, wm))
}

fn update_extrude(rt: &mut DocumentRuntime, record: u128, sketch: u128, distance: f64) {
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(record)),
        op: extrude_record(record, SketchId(Uuid::from_u128(sketch)), distance).op,
    })
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_keeps_verdicts_but_conservative_undo_invalidates() {
    let Some((mut rt, wm)) = valid_runtime().await else {
        return;
    };
    rt.apply(EditCommand::RenameBody {
        body: rt.head_body_ids()[0],
        name: "Renamed".into(),
    })
    .unwrap();
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    rt.undo().unwrap();
    assert_eq!(statuses(&rt), vec![Dirty; 4]);
    rt.redo().unwrap().unwrap();
    assert_eq!(statuses(&rt), vec![Dirty; 4]);
    published(&regen(&mut rt, 0).await, "metadata undo replay");
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sketch_visibility_history_preserves_verdicts_without_regeneration() {
    let Some((mut rt, wm)) = valid_runtime().await else {
        return;
    };
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(SketchId(Uuid::from_u128(0xA)), 0x1000, 0.0, 0.0, 40.0, 20.0),
    })
    .unwrap();
    published(&regen(&mut rt, 0).await, "stored sketch baseline");
    let outcome = rt
        .apply(EditCommand::SetVisibility {
            target: VisibilityTarget::Sketch(SketchId(Uuid::from_u128(0xA))),
            visible: false,
        })
        .unwrap();
    assert!(outcome.dirty.is_none());
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    assert!(rt.undo().unwrap().regen.is_none());
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    assert!(rt.redo().unwrap().unwrap().regen.is_none());
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parameter_edits_and_history_retain_only_the_unchanged_prefix() {
    let Some((mut rt, wm)) = valid_runtime().await else {
        return;
    };
    update_extrude(&mut rt, EX_B, 0xB, 30.0);
    assert_eq!(statuses(&rt), vec![Ok, Ok, Ok, Dirty]);
    published(&regen(&mut rt, 3).await, "suffix edit");
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    rt.undo().unwrap();
    assert_eq!(statuses(&rt), vec![Ok, Ok, Ok, Dirty]);
    published(&regen(&mut rt, 3).await, "suffix undo");
    rt.redo().unwrap().unwrap();
    assert_eq!(statuses(&rt), vec![Ok, Ok, Ok, Dirty]);
    update_extrude(&mut rt, EX_A, 0xA, 30.0);
    assert_eq!(statuses(&rt), vec![Ok, Dirty, Dirty, Dirty]);
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppression_and_rollback_do_not_certify_newly_applied_steps() {
    let Some((mut rt, wm)) = valid_runtime().await else {
        return;
    };
    rt.apply(EditCommand::SetOperationSuppression {
        record: RecordId(Uuid::from_u128(EX_B)),
        suppressed: true,
        cascade: false,
    })
    .unwrap();
    assert_eq!(&statuses(&rt)[..3], &[Ok; 3]);
    assert!(rt.projection().features[3].suppressed);
    rt.undo().unwrap();
    assert!(!rt.projection().features[3].suppressed);
    assert_eq!(statuses(&rt), vec![Ok, Ok, Ok, Dirty]);
    published(&regen(&mut rt, 3).await, "unsuppress replay");
    rt.apply(EditCommand::SetRollback { cursor: 2 }).unwrap();
    assert_eq!(&statuses(&rt)[..2], &[Ok; 2]);
    rt.apply(EditCommand::SetRollback { cursor: 4 }).unwrap();
    assert_eq!(statuses(&rt), vec![Ok, Ok, Dirty, Dirty]);
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_variables_invalidate_all_and_new_expression_errors_win() {
    let Some((mut rt, wm)) = valid_runtime().await else {
        return;
    };
    let id = VariableId::new();
    rt.apply(EditCommand::AddVariable {
        variable: Variable {
            id,
            name: "depth".into(),
            value: Scalar::new(25.0),
            unit: Default::default(),
        },
    })
    .unwrap();
    let mut record = rt
        .operation_record(RecordId(Uuid::from_u128(EX_A)))
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op else {
        panic!()
    };
    params.distance = Scalar::with_expr(25.0, "depth");
    rt.apply(EditCommand::UpdateOperationParams {
        record: record.record_id,
        op: record.op,
    })
    .unwrap();
    published(&regen(&mut rt, 0).await, "bound variable");
    assert_eq!(statuses(&rt), vec![Ok; 4]);
    rt.apply(EditCommand::SetVariable {
        variable: id,
        value: Scalar::new(30.0),
    })
    .unwrap();
    assert_eq!(statuses(&rt), vec![Dirty; 4]);
    published(&regen(&mut rt, 0).await, "variable edit");
    rt.apply(EditCommand::RemoveVariable { variable: id })
        .unwrap();
    assert_eq!(statuses(&rt), vec![Dirty, Error, Dirty, Dirty]);
    rt.undo().unwrap();
    assert_eq!(statuses(&rt), vec![Dirty; 4]);
    rt.redo().unwrap().unwrap();
    assert_eq!(statuses(&rt), vec![Dirty, Error, Dirty, Dirty]);
    wm.shutdown().await;
}
