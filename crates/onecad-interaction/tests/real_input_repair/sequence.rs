use super::*;
fn chamfer() -> RecordId {
    RecordId(Uuid::from_u128(0x8e003))
}

#[test]
fn repairs_are_incremental_and_multireference_steps_are_refused() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("repair-sequence") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(sequence(&root));
    } else {
        let root = tempfile::tempdir().unwrap();
        host::run_isolated_phase(
            root.path(),
            "repair-sequence",
            "sequence::repairs_are_incremental_and_multireference_steps_are_refused",
        );
    }
}
async fn replace_move(h: &Harness) -> onecad_app::dto::DocumentProjection {
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
    h.terminal(&result.projection).await;
    h.projection().await
}
async fn other_edges(h: &Harness) -> Vec<api::StagePick> {
    let record = api::get_operation_record(&h.state, fillet()).await.unwrap();
    let Operation::Known(KnownOperation::Fillet(p)) = record.op else {
        unreachable!()
    };
    let anchor = p.edges[0].anchor.as_ref().unwrap().world_point;
    let mut edges: Vec<_> = drive::elements(h, body(), "e")
        .await
        .into_iter()
        .filter(|e| e.curve_type == 0 && (e.magnitude - 25.0).abs() < 1e-6)
        .collect();
    edges.sort_by(|a, b| {
        let distance = |e: &onecad_app::dto::ElementInfoDto| {
            (e.center[0] - anchor.x).powi(2) + (e.center[1] - anchor.y).powi(2)
        };
        distance(b).total_cmp(&distance(a))
    });
    edges
        .into_iter()
        .take(2)
        .map(|e| api::StagePick {
            body: body(),
            kind: ElementKind::Edge,
            address: api::StageAddress::TopoKey(e.topo_key),
        })
        .collect()
}
async fn sequence(root: &Path) {
    let h = Harness::new(root).await;
    build(&h).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    let picks = other_edges(&h).await;
    let refs = stage_chamfer_edges(&h, vec![picks[0].clone()]).await;
    let op = Operation::Known(KnownOperation::Chamfer(ChamferParams {
        radius: Scalar::new(2.0),
        distance2: None,
        angle_deg: None,
        edge_ids: refs
            .iter()
            .map(|r| r.primary.as_ref().unwrap().element.clone())
            .collect(),
        edges: refs,
        reference_faces: vec![],
        reference_face_refs: vec![],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }));
    apply(
        &h,
        EditCommand::AddOperation {
            record: OperationRecord::new(chamfer(), 4, "Second gated feature", op),
            at_cursor: true,
        },
    )
    .await;
    assert_volume(&h, expected() - 50.0).await;
    replace_move(&h).await;
    let first = stage(&h).await;
    let mut state = Interaction::default();
    let effects = begin(&h, &mut state, first.clone(), 1).await;
    drive::drive(&h, &mut state, effects).await;
    assert!(matches!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::RepairWarning(_)
    ));
    assert!(state.tool.as_ref().unwrap().ready());
    let effects = state.dispatch(Action::Confirm);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ApplyInputRepair { .. })));
    let result = api::commit_input_repair(&h.state, h.host.clone(), &first)
        .await
        .unwrap();
    assert!(result.rollback_token.is_some());
    let report = h.terminal(&result.projection).await;
    assert_eq!(report.repair_steps, [chamfer().to_string()]);
    assert_volume(&h, expected()).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    let report = h.terminal(&undo).await;
    assert_eq!(
        report.repair_steps,
        [fillet().to_string(), chamfer().to_string()]
    );
    assert_volume(&h, 100_000.0).await;
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    let report = h.terminal(&redo).await;
    assert_eq!(report.repair_steps, [chamfer().to_string()]);
    assert_volume(&h, expected()).await;
    let pick = other_edges(&h).await.remove(0);
    let second = api::stage_input_repair(
        &h.state,
        api::preview_context(&h.state).await.unwrap(),
        chamfer(),
        pick,
    )
    .await
    .unwrap();
    let preview = api::preview_input_repair(&h.state, &second, onecad_core::regen::Lod::Fine)
        .await
        .unwrap();
    assert_eq!(preview.outcome, api::EditPreviewOutcome::Ready);
    let result = api::commit_input_repair(&h.state, h.host.clone(), &second)
        .await
        .unwrap();
    h.settled(&result.projection).await;
    assert_volume(&h, expected() - 50.0).await;
    multi_ref_refusal(&h).await;
    h.close().await;
}
async fn multi_ref_refusal(h: &Harness) {
    let refs = stage_chamfer_edges(h, other_edges(h).await).await;
    let record = api::get_operation_record(&h.state, chamfer())
        .await
        .unwrap();
    let Operation::Known(KnownOperation::Chamfer(mut p)) = record.op else {
        unreachable!()
    };
    p.edge_ids = refs
        .iter()
        .map(|r| r.primary.as_ref().unwrap().element.clone())
        .collect();
    p.edges = refs;
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: chamfer(),
            op: Operation::Known(KnownOperation::Chamfer(p)),
        }],
        None,
    )
    .await
    .unwrap();
    let report = h.terminal(&result.projection).await;
    assert_eq!(
        report.repair_steps,
        [chamfer().to_string()],
        "multi-edge re-edit truthfully preserves the worker's identity refusal"
    );
    let before = h.projection().await;
    assert!(api::input_repair_target(
        &api::get_operation_record(&h.state, chamfer())
            .await
            .unwrap()
    )
    .is_err());
    assert!(api::stage_input_repair(
        &h.state,
        api::preview_context(&h.state).await.unwrap(),
        chamfer(),
        edge(h).await
    )
    .await
    .is_err());
    assert_eq!(
        h.projection().await,
        before,
        "refusal cannot clear any step gate"
    );
}

async fn stage_chamfer_edges(
    h: &Harness,
    picks: Vec<api::StagePick>,
) -> Vec<onecad_core::document::refs::ElementRef> {
    let picks = picks
        .into_iter()
        .map(|pick| {
            let api::StageAddress::TopoKey(key) = pick.address else {
                unreachable!()
            };
            api::EdgeOpPickInput {
                body_id: Some(onecad_app::worker::wire::body_id_wire(pick.body)),
                topo_key: Some(key),
                element_id: None,
            }
        })
        .collect();
    api::stage_edge_op(
        &h.state,
        &api::preview_context(&h.state).await.unwrap(),
        "Chamfer",
        picks,
        false,
    )
    .await
    .unwrap()
    .refs
}
