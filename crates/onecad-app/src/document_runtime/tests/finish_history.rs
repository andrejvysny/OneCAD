use super::*;

#[tokio::test]
async fn native_finish_handoff_is_one_shot_and_invalidated_by_interleaving() {
    let mut runtime = runtime_with(Arc::new(FakeBackend::new()));
    let (sketch, point) = sketch_with_point();
    let id = sketch.id;
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime.enter_sketch(id).await.unwrap();
    runtime
        .sketch_upsert(
            id,
            vec![SketchEditOp::SetEntityPositions {
                positions: vec![(point, Vec2::new(4.0, 5.0).unwrap())],
            }],
        )
        .await
        .unwrap();
    runtime.cancel_sketch(id, false).await.unwrap();
    assert_eq!(
        runtime.pending_sketch_finish,
        Some((id, runtime.undo_position()))
    );
    runtime
        .apply(EditCommand::RenameSketch {
            sketch: id,
            name: "Interleaved".into(),
        })
        .unwrap();
    assert!(runtime.pending_sketch_finish.is_none());
    runtime.enter_sketch(id).await.unwrap();
    runtime
        .sketch_upsert(
            id,
            vec![SketchEditOp::SetEntityPositions {
                positions: vec![(point, Vec2::new(6.0, 7.0).unwrap())],
            }],
        )
        .await
        .unwrap();
    runtime.cancel_sketch(id, false).await.unwrap();
    assert!(runtime.pending_sketch_finish.is_some());
    runtime.finish_sketch(id).await.unwrap();
    assert!(runtime.pending_sketch_finish.is_none());
}
