use super::*;
pub async fn context(harness: &Harness) -> DocumentContext {
    let head = api::preview_context(&harness.state).await.unwrap();
    DocumentContext {
        document_id: head.document_id,
        runtime_session: head.runtime_session,
        revision: head.revision.0,
        snapshot_id: head.snapshot_id,
        scene: SceneIdentity {
            session: 1,
            scene: head.snapshot_id.0,
        },
    }
}

pub async fn snapshot(harness: &Harness) -> SketchSnapshot {
    SketchSnapshot {
        history: None,
        document: context(harness).await,
        sketch: Box::new(
            api::get_sketch_snapshot(&harness.state, sketch_id())
                .await
                .unwrap(),
        ),
    }
}

pub async fn begin(harness: &Harness, create: bool) -> SketchInteraction {
    let mut state = SketchInteraction::default();
    let document = context(harness).await;
    state.dispatch(SketchAction::ContextChanged(Some(document.clone())));
    let sketch = if create {
        Sketch::on_world_plane(sketch_id(), "Persistent center rectangle", WorldPlane::XY)
    } else {
        api::get_sketch_snapshot(&harness.state, sketch_id())
            .await
            .unwrap()
    };
    let mut effects = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        create,
        context: document,
    });
    assert_eq!(effects.len(), 1);
    if create {
        let SketchEffect::CreateSketch { tag, command } = effects.remove(0) else {
            panic!("expected authoring effect")
        };
        api::apply_edit_command(&harness.state, harness.host.clone(), *command)
            .await
            .unwrap();
        state.dispatch(SketchAction::ContextChanged(Some(context(harness).await)));
        effects = state.dispatch(SketchAction::Created { tag });
    }
    let SketchEffect::EnterSketch { tag, created_here } = effects.remove(0) else {
        panic!("expected enter effect")
    };
    let solve = api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        tag.key.sketch_id.to_string(),
        Some(created_here),
    )
    .await
    .unwrap();
    let snapshot = snapshot(harness).await;
    let document = snapshot.document.clone();
    state.dispatch(SketchAction::ContextChanged(Some(document)));
    state.dispatch(SketchAction::Entered {
        tag,
        snapshot,
        solve: Box::new(solve),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    state
}

pub fn ids(seed: u128) -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|i| EntityId(Uuid::from_u128(seed + i as u128))),
        constraints: std::array::from_fn(|i| ConstraintId(Uuid::from_u128(seed + 100 + i as u128))),
    }
}

pub fn captured(state: &mut SketchInteraction, point: Vec2) -> CapturedSketchPoint {
    let session = state.session.as_ref().unwrap();
    let view = ViewSnapshot::new(
        session.document.scene,
        1,
        Camera {
            target: DVec3::ZERO,
            offset: DVec3::new(0.0, 0.0, 100.0),
            projection: Projection::Orthographic,
            ..Camera::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap();
    let world = session.plane.to_world(point);
    let logical = view
        .project(DVec3::new(world.x, world.y, world.z))
        .unwrap()
        .logical;
    let point = capture_sketch_pointer(session, &view, logical).unwrap();
    state.dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
    });
    point
}

pub async fn submit(
    harness: &Harness,
    state: &mut SketchInteraction,
    mut effects: Vec<SketchEffect>,
) -> onecad_app::dto::SketchUpsertDto {
    let (tag, receipt) = drive(harness, &mut effects).await;
    if receipt.regen_expected {
        eprintln!(
            "center/point: waiting command completion revision {}",
            receipt.projection.revision
        );
        harness.settled(&receipt.projection).await;
    }
    let admission = receipt.result.unwrap();
    assert!(admission.accepted, "{:?}", admission.solve);
    state.dispatch(SketchAction::BatchAccepted { tag: tag.clone() });
    let snapshot = snapshot(harness).await;
    state.dispatch(SketchAction::ContextChanged(Some(
        snapshot.document.clone(),
    )));
    state.dispatch(SketchAction::Solved {
        tag,
        snapshot,
        solve: Box::new(admission.solve.clone()),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    admission.solve
}

async fn drive(
    harness: &Harness,
    effects: &mut Vec<SketchEffect>,
) -> (
    SketchTag,
    onecad_app::api::CommandCompletion<onecad_app::dto::SketchConstraintAdmission>,
) {
    assert_eq!(effects.len(), 1);
    match effects.remove(0) {
        SketchEffect::Upsert { tag, ops } => (
            tag,
            api::author_sketch_primitive_with_completion(
                &harness.state,
                harness.host.clone(),
                sketch_id().to_string(),
                ops,
            )
            .await
            .unwrap(),
        ),
        SketchEffect::AddConstraints { tag, constraints } => (
            tag,
            api::add_sketch_constraints_with_completion(
                &harness.state,
                harness.host.clone(),
                sketch_id().to_string(),
                constraints,
            )
            .await
            .unwrap(),
        ),
        SketchEffect::SetDimension {
            tag,
            constraint,
            value,
        } => (
            tag,
            api::set_sketch_dimension_with_completion(
                &harness.state,
                harness.host.clone(),
                sketch_id().to_string(),
                constraint,
                value,
            )
            .await
            .unwrap(),
        ),
        other => panic!("unexpected {other:?}"),
    }
}

pub async fn history(
    harness: &Harness,
    state: &mut SketchInteraction,
    direction: onecad_core::edit::HistoryDirection,
) {
    let mut effects = state.dispatch(SketchAction::History(direction));
    let SketchEffect::History {
        tag, edit_session, ..
    } = effects.remove(0)
    else {
        panic!("history effect")
    };
    let scope = onecad_app::dto::SketchHistoryScope {
        sketch_id: sketch_id(),
        edit_session,
        runtime_session: tag.document.runtime_session.clone(),
        revision: tag.document.revision,
        snapshot_id: tag.document.snapshot_id,
    };
    let receipt =
        api::sketch_history_with_completion(&harness.state, harness.host.clone(), scope, direction)
            .await
            .unwrap();
    if receipt.regen_expected {
        eprintln!(
            "center/point: waiting command completion revision {}",
            receipt.projection.revision
        );
        harness.settled(&receipt.projection).await;
    }
    let history = receipt.result.unwrap();
    assert!(history.moved);
    let snapshot = snapshot(harness).await;
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    state.dispatch(SketchAction::ContextChanged(Some(
        snapshot.document.clone(),
    )));
    state.dispatch(SketchAction::HistoryResolved {
        tag,
        snapshot,
        solve: Box::new(solve),
        moved: true,
        direction,
    });
}

pub async fn close_visit(harness: &Harness) {
    api::cancel_sketch(
        &harness.state,
        harness.host.clone(),
        sketch_id().to_string(),
        Some(false),
    )
    .await
    .unwrap();
}

pub async fn regions(harness: &Harness, count: usize) {
    let regions = api::get_sketch_regions(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    assert_eq!(regions.regions.len(), count);
    if count == 1 {
        assert!(regions.regions[0].holes.is_empty());
    }
}

pub async fn sketch(harness: &Harness) -> Sketch {
    api::get_sketch_snapshot(&harness.state, sketch_id())
        .await
        .unwrap()
}

pub fn at(sketch: &Sketch, id: EntityId) -> Vec2 {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => *at,
        _ => panic!("persistent point"),
    }
}

pub fn near(actual: Vec2, x: f64, y: f64) {
    assert!(
        (actual.x - x).abs() < 1e-6 && (actual.y - y).abs() < 1e-6,
        "{actual:?} != ({x}, {y})"
    );
}

pub fn symmetric(sketch: &Sketch, ids: &PrimitiveIds, center: Vec2) {
    near(at(sketch, ids.entities[8]), center.x, center.y);
    for (a, b) in [(0, 2), (1, 3)] {
        let a = at(sketch, ids.entities[a]);
        let b = at(sketch, ids.entities[b]);
        near(
            Vec2::new_unchecked((a.x + b.x) * 0.5, (a.y + b.y) * 0.5),
            center.x,
            center.y,
        );
    }
}
