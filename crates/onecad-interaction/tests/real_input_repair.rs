//! Explicit repair through shared native reducer/application contracts and OCCT.
#[allow(dead_code)]
#[path = "real_edge_ops/drive.rs"]
mod drive;
#[path = "real_input_repair/gesture.rs"]
mod gesture;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/profile.rs"]
mod profile;
#[path = "real_input_repair/sequence.rs"]
mod sequence;
use host::Harness;
use onecad_app::{api, dto::FeatureStatus};
use onecad_core::{
    document::{record::*, refs::ElementKind, variables::Scalar},
    edit::EditCommand,
    ids::{BodyId, RecordId},
};
use onecad_interaction::*;
use std::{
    f64::consts::FRAC_PI_4,
    path::{Path, PathBuf},
};
use uuid::Uuid;
fn moved() -> RecordId {
    RecordId(Uuid::from_u128(0x8e001))
}
fn fillet() -> RecordId {
    RecordId(Uuid::from_u128(0x8e002))
}
fn body() -> BodyId {
    BodyId(profile::extrude_id().0)
}
fn expected() -> f64 {
    80.0 * 50.0 * 25.0 - (1.0 - FRAC_PI_4) * 9.0 * 25.0
}

#[test]
fn explicit_repick_repairs_move_gate_preview_commit_history_reopen() {
    host::required_worker();
    match std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE")
        .ok()
        .as_deref()
    {
        Some(phase @ ("repair-author" | "repair-reopen")) => {
            let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    if phase == "repair-author" {
                        author(&root).await;
                    } else {
                        reopen(&root).await;
                    }
                });
        }
        _ => {
            let root = tempfile::tempdir().unwrap();
            for phase in ["repair-author", "repair-reopen"] {
                host::run_isolated_phase(
                    root.path(),
                    phase,
                    "explicit_repick_repairs_move_gate_preview_commit_history_reopen",
                );
            }
        }
    }
}
async fn apply(h: &Harness, command: EditCommand) {
    let result = api::apply_operation(&h.state, h.host.clone(), vec![command], None)
        .await
        .unwrap();
    h.settled(&result.projection).await;
}
async fn edge(h: &Harness) -> api::StagePick {
    let edge = drive::elements(h, body(), "e")
        .await
        .into_iter()
        .find(|edge| edge.curve_type == 0 && (edge.magnitude - 25.0).abs() < 1e-6)
        .unwrap();
    api::StagePick {
        body: body(),
        kind: ElementKind::Edge,
        address: api::StageAddress::TopoKey(edge.topo_key),
    }
}
async fn stage(h: &Harness) -> api::InputRepair {
    api::stage_input_repair(
        &h.state,
        api::preview_context(&h.state).await.unwrap(),
        fillet(),
        edge(h).await,
    )
    .await
    .unwrap()
}
async fn begin(
    h: &Harness,
    state: &mut Interaction,
    repair: api::InputRepair,
    gesture: u64,
) -> Vec<Effect> {
    let context = drive::adopt(h, state).await;
    state.dispatch(Action::BeginInputRepair {
        repair: Box::new(repair),
        gesture,
        context,
    })
}
async fn assert_volume(h: &Harness, expected: f64) {
    assert!((drive::volume(h, body()).await - expected).abs() < 1e-6);
}
async fn seed(h: &Harness) {
    let record = api::get_operation_record(&h.state, moved()).await.unwrap();
    let Operation::Known(KnownOperation::TransformBody(mut p)) = record.op else {
        unreachable!()
    };
    p.translate[1] = Scalar::new(40.0);
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: moved(),
            op: Operation::Known(KnownOperation::TransformBody(p)),
        }],
        None,
    )
    .await
    .unwrap();
    let report = h.terminal(&result.projection).await;
    assert_eq!(report.repair_steps, [fillet().to_string()]);
    assert_volume(h, 100_000.0).await;
}
async fn build(h: &Harness) {
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let finished = profile::create(h).await;
    apply(
        h,
        EditCommand::AddOperation {
            record: OperationRecord::new(
                profile::extrude_id(),
                1,
                "Box",
                profile::extrude(&finished, 25.0),
            ),
            at_cursor: true,
        },
    )
    .await;
    let mut record = operand_binding::transform_record(body(), [0.0; 3], moved(), 2);
    let Operation::Known(KnownOperation::TransformBody(p)) = &mut record.op else {
        unreachable!()
    };
    p.translate[0] = Scalar::new(15.0);
    record.inputs = record.op.derive_inputs();
    apply(
        h,
        EditCommand::AddOperation {
            record,
            at_cursor: true,
        },
    )
    .await;
    let scope = api::preview_context(&h.state).await.unwrap();
    let pick = edge(h).await;
    let api::StageAddress::TopoKey(topo_key) = pick.address else {
        unreachable!()
    };
    let refs = api::stage_edge_op(
        &h.state,
        &scope,
        "Fillet",
        vec![api::EdgeOpPickInput {
            body_id: Some(onecad_app::worker::wire::body_id_wire(body())),
            topo_key: Some(topo_key),
            element_id: None,
        }],
        false,
    )
    .await
    .unwrap()
    .refs;
    let operation = Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(3.0),
        edge_ids: refs
            .iter()
            .map(|r| r.primary.as_ref().unwrap().element.clone())
            .collect(),
        edges: refs,
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }));
    apply(
        h,
        EditCommand::AddOperation {
            record: OperationRecord::new(fillet(), 3, "Repairable Fillet", operation),
            at_cursor: true,
        },
    )
    .await;
    assert_volume(h, expected()).await;
    seed(h).await;
}
async fn author(root: &Path) {
    let h = Harness::new(&root.join("author")).await;
    build(&h).await;
    let before = h.projection().await;
    let original = api::get_operation_record(&h.state, fillet()).await.unwrap();
    let scope = api::preview_context(&h.state).await.unwrap();
    let mut wrong = edge(&h).await;
    wrong.kind = ElementKind::Face;
    assert!(
        api::stage_input_repair(&h.state, scope.clone(), fillet(), wrong)
            .await
            .is_err()
    );
    let mut wrong = edge(&h).await;
    wrong.body = BodyId::new();
    assert!(api::stage_input_repair(&h.state, scope, fillet(), wrong)
        .await
        .is_err());
    let repair = stage(&h).await;
    assert!(
        api::commit_input_repair(&h.state, h.host.clone(), &repair)
            .await
            .is_err(),
        "preview required server-side"
    );
    let mut state = Interaction::default();
    let effects = begin(&h, &mut state, repair.clone(), 1).await;
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PreviewInputRepair { .. })));
    assert!(state.dispatch(Action::Confirm).is_empty());
    let tag = state.tool.as_ref().unwrap().tag.clone();
    assert!(state
        .dispatch(Action::ParameterChanged {
            key: NumericParameter::FilletRadius.into(),
            text: "9".into()
        })
        .is_empty());
    assert!(state
        .dispatch(Action::ModelingOptionsChanged {
            tag: tag.clone(),
            options: ModelingOptions::FaceOffsetMode(OffsetDistanceType::Offset)
        })
        .is_empty());
    drive::drive(&h, &mut state, effects).await;
    assert!(state.tool.as_ref().unwrap().ready());
    assert_eq!(
        h.projection().await,
        before,
        "staging and preview cannot publish"
    );
    assert_eq!(
        api::get_operation_record(&h.state, fillet()).await.unwrap(),
        original
    );
    assert_volume(&h, 100_000.0).await;
    state.dispatch(Action::Cancel);
    assert!(state
        .dispatch(Action::EditPreviewFinished {
            tag,
            result: Err(onecad_app::error::ApiError::Unsupported("obsolete".into()))
        })
        .is_empty());
    assert_eq!(h.projection().await, before);
    assert!(
        api::commit_input_repair(&h.state, h.host.clone(), &repair)
            .await
            .is_err(),
        "cancel revokes the staged repair"
    );
    let repair = stage(&h).await;
    let effects = begin(&h, &mut state, repair.clone(), 2).await;
    drive::drive(&h, &mut state, effects).await;
    let after = drive::confirm(&h, &mut state).await;
    assert_eq!(after.undo_depth, before.undo_depth + 1);
    assert_eq!(after.features.len(), before.features.len());
    assert!(after.features.iter().all(|f| f.status == FeatureStatus::Ok));
    assert_volume(&h, expected()).await;
    assert!(state.dispatch(Action::Confirm).is_empty());
    assert!(
        api::commit_input_repair(&h.state, h.host.clone(), &repair)
            .await
            .is_err(),
        "duplicate commit fenced"
    );
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    let report = h.terminal(&undo).await;
    assert_eq!(report.repair_steps, [fillet().to_string()]);
    assert_volume(&h, 100_000.0).await;
    assert!(
        api::preview_input_repair(&h.state, &repair, onecad_core::regen::Lod::Fine)
            .await
            .is_err(),
        "old staged token fenced after undo"
    );
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    assert_volume(&h, expected()).await;
    api::save_document(
        &h.state,
        h.host.clone(),
        Some(root.join("repair.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    h.close().await;
}
async fn reopen(root: &Path) {
    let h = Harness::new(&root.join("reopen")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join("repair.onecad").to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    h.settled(&h.projection().await).await;
    assert_volume(&h, expected()).await;
    assert_eq!(
        api::get_operation_record(&h.state, fillet())
            .await
            .unwrap()
            .record_id,
        fillet()
    );
    h.close().await;
}
