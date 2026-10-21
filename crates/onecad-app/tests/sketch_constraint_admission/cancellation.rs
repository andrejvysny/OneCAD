use super::*;
use onecad_app::sketch_candidate::SketchCandidateControl;

#[tokio::test]
async fn cancellation_after_worker_solve_preserves_accepted_state() {
    let Some((worker, mut runtime, sketch)) = setup(true).await else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    assert!(runtime.mark_saved(
        &directory.path().join("accepted.onecad"),
        runtime.revision()
    ));
    assert!(!runtime.is_dirty());
    let before = runtime.sketch_snapshot(sketch, "before").unwrap();
    let state = (
        runtime.revision(),
        runtime.undo_depth(),
        runtime.redo_depth(),
        runtime.is_dirty(),
    );
    let control = SketchCandidateControl::default();
    let driven = runtime
        .prepare_sketch_dimension(sketch, constraint(3), Scalar::new(20.0))
        .await
        .unwrap()
        .with_control(control.clone())
        .drive()
        .await
        .unwrap();
    assert!(control.cancel());
    let error = runtime.commit_sketch_constraints(driven).unwrap_err();
    assert!(
        error.to_string().contains("cancelled before admission"),
        "{error}"
    );
    assert_eq!(runtime.sketch_snapshot(sketch, "after").unwrap(), before);
    assert_eq!(
        (
            runtime.revision(),
            runtime.undo_depth(),
            runtime.redo_depth(),
            runtime.is_dirty()
        ),
        state
    );
    let driven = runtime
        .prepare_sketch_dimension(sketch, constraint(3), Scalar::new(15.0))
        .await
        .unwrap()
        .drive()
        .await
        .unwrap();
    assert!(
        runtime
            .commit_sketch_constraints(driven)
            .unwrap()
            .0
            .accepted
    );
    worker.shutdown().await;
}

#[tokio::test]
async fn cancellation_before_execution_releases_candidate_lane() {
    let Some((worker, runtime, sketch)) = setup(true).await else {
        return;
    };
    let control = SketchCandidateControl::default();
    assert!(control.cancel());
    let prepared = runtime
        .prepare_sketch_dimension(sketch, constraint(3), Scalar::new(20.0))
        .await
        .unwrap()
        .with_control(control);
    match prepared.drive().await {
        Ok(_) => panic!("cancelled candidate executed"),
        Err(error) => assert!(error.to_string().contains("cancelled before admission")),
    }
    assert!(runtime
        .prepare_sketch_dimension(sketch, constraint(3), Scalar::new(15.0))
        .await
        .is_ok());
    worker.shutdown().await;
}
