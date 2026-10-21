use super::*;

#[test]
fn numeric_partial_authorship_real_solver_and_atomic_history() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("authorship") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(workflow(&root));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-authorship-")
        .tempdir()
        .unwrap();
    host::run_isolated_phase(
        root.path(),
        "authorship",
        "authorship::numeric_partial_authorship_real_solver_and_atomic_history",
    );
}

async fn workflow(root: &Path) {
    let harness = new(root).await;
    for (tool, values, authored, anchors, cursor) in cases() {
        api::new_document(&harness.state, harness.host.clone())
            .await
            .unwrap();
        assert!(
            harness
                .state
                .live_worker()
                .unwrap()
                .wait_ready(std::time::Duration::from_secs(15))
                .await
        );
        let mut state = begin(&harness, true).await;
        state.dispatch(SketchAction::SetTool(tool));
        for anchor in anchors {
            let point = captured(&mut state, anchor);
            assert!(state
                .dispatch(SketchAction::Click {
                    point,
                    ids: ids(500)
                })
                .is_empty());
        }
        let point = captured(&mut state, cursor);
        state.dispatch(SketchAction::Move(point));
        let session = state.session.as_ref().unwrap();
        let effects = state.dispatch(SketchAction::CreateNumericAuthored {
            draft_generation: session.draft_generation,
            key: session.key.clone(),
            capture: session.accepted_capture.unwrap(),
            ids: ids(1000),
            dimensions: Box::new(values),
            authored,
        });
        let solve = submit(&harness, &mut state, effects).await;
        assert!(
            matches!(
                solve.status,
                SketchSolveStatus::UnderConstrained | SketchSolveStatus::FullyConstrained
            ),
            "{tool:?}: {solve:?}"
        );
        if matches!(tool, SketchTool::Rectangle | SketchTool::CenterRectangle) {
            assert_eq!(solve.dof, 3, "position and untouched height remain free");
        }
        let accepted = state.session.as_ref().unwrap().authoritative.clone();
        assert!(!accepted.entities().is_empty());
        assert!(
            !accepted
                .constraints()
                .iter()
                .any(|constraint| constraint.id() == ids(1000).constraints[9]),
            "untouched second field must remain free: {tool:?}"
        );
        state.dispatch(SketchAction::SetTool(SketchTool::Select));
        history(&harness, &mut state, HistoryDirection::Undo).await;
        assert!(state
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .is_empty());
        history(&harness, &mut state, HistoryDirection::Redo).await;
        let restored = &state.session.as_ref().unwrap().authoritative;
        assert_eq!(restored.entities().len(), accepted.entities().len());
        assert_eq!(restored.constraints(), accepted.constraints());
        close_visit(&harness).await;
    }
    tangent_arc(&harness).await;
    harness.close().await;
}

type NumericCase = (
    SketchTool,
    CreationDimensions,
    Vec<CreationField>,
    Vec<Vec2>,
    Vec2,
);

fn cases() -> Vec<NumericCase> {
    let at = |x, y| Vec2::new_unchecked(x, y);
    vec![
        (
            SketchTool::Point,
            CreationDimensions::Point {
                x: Scalar::new(0.0),
                y: Scalar::new(0.0),
            },
            vec![CreationField::X],
            vec![],
            at(0.0, 0.0),
        ),
        (
            SketchTool::Line,
            CreationDimensions::Line {
                length: Scalar::new(10.0),
            },
            vec![CreationField::Length],
            vec![at(0.0, 0.0)],
            at(6.0, 8.0),
        ),
        (
            SketchTool::Rectangle,
            CreationDimensions::Rectangle {
                width: Scalar::new(80.0),
                height: Scalar::new(50.0),
            },
            vec![CreationField::Width],
            vec![at(0.0, 0.0)],
            at(10.0, 50.0),
        ),
        (
            SketchTool::CenterRectangle,
            CreationDimensions::CenterRectangle {
                width: Scalar::new(80.0),
                height: Scalar::new(50.0),
            },
            vec![CreationField::Width],
            vec![at(0.0, 0.0)],
            at(10.0, 25.0),
        ),
        (
            SketchTool::Circle,
            CreationDimensions::Circle {
                diameter: Scalar::new(6.0),
            },
            vec![CreationField::Diameter],
            vec![at(0.0, 0.0)],
            at(3.0, 0.0),
        ),
        (
            SketchTool::Arc,
            CreationDimensions::Arc {
                radius: Scalar::new(10.0),
                sweep: std::f64::consts::FRAC_PI_2,
            },
            vec![CreationField::Sweep],
            vec![at(0.0, 0.0), at(10.0, 0.0)],
            at(0.0, 10.0),
        ),
        (
            SketchTool::ThreePointArc,
            CreationDimensions::ThreePointArc {
                radius: Scalar::new(10.0),
            },
            vec![CreationField::Radius],
            vec![at(-5.0, 0.0), at(5.0, 0.0)],
            at(0.0, 3.0),
        ),
        (
            SketchTool::Slot,
            CreationDimensions::Slot {
                width: Scalar::new(6.0),
            },
            vec![],
            vec![at(0.0, 0.0), at(20.0, 0.0)],
            at(10.0, 3.0),
        ),
    ]
}

async fn tangent_arc(harness: &Harness) {
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        harness
            .state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let mut state = begin(harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    let point = captured(&mut state, Vec2::new_unchecked(0.0, 0.0));
    state.dispatch(SketchAction::Click {
        point,
        ids: ids(500),
    });
    let point = captured(&mut state, Vec2::new_unchecked(10.0, 0.0));
    let effects = state.dispatch(SketchAction::Click {
        point,
        ids: ids(1000),
    });
    submit(harness, &mut state, effects).await;
    let line = state.session.as_ref().unwrap().authoritative.clone();
    let key = state.session.as_ref().unwrap().key.clone();
    state.dispatch(SketchAction::SetTangentArc { key, enabled: true });
    let point = captured(&mut state, Vec2::new_unchecked(20.0, 10.0));
    state.dispatch(SketchAction::Move(point));
    let session = state.session.as_ref().unwrap();
    let effects = state.dispatch(SketchAction::CreateNumericAuthored {
        draft_generation: session.draft_generation,
        key: session.key.clone(),
        capture: session.accepted_capture.unwrap(),
        ids: ids(2000),
        dimensions: Box::new(CreationDimensions::TangentArc {
            radius: Scalar::new(5.0),
        }),
        authored: vec![CreationField::Radius],
    });
    submit(harness, &mut state, effects).await;
    let accepted = state.session.as_ref().unwrap().authoritative.clone();
    assert!(
        matches!(accepted.get_entity(ids(2000).entities[1]), Some(SketchEntity::Arc { radius, .. }) if (*radius - 5.0).abs() < 1e-8)
    );
    assert!(accepted
        .constraints()
        .iter()
        .any(|constraint| matches!(constraint, Constraint::TangentAtPoint { .. })));
    state.dispatch(SketchAction::SetTool(SketchTool::Select));
    history(harness, &mut state, HistoryDirection::Undo).await;
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .len(),
        line.entities().len()
    );
    history(harness, &mut state, HistoryDirection::Redo).await;
    assert_eq!(
        state.session.as_ref().unwrap().authoritative.constraints(),
        accepted.constraints()
    );
    close_visit(harness).await;
}
