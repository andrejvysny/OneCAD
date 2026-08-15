async fn run_symmetric_ambiguity(wm: &WorkerManager, epoch: WorkerEpoch) -> Result<(), String> {
    let sketch = rectangle_sketch("corpus-f-box", -20.0, -10.0, 40.0, 20.0);
    let body_record = 0xF011;
    let body = body_of_record(RecordId(Uuid::from_u128(body_record)));
    let timeline = timeline([
        sketch_record_for(0xF010, &sketch),
        extrude_record_for(body_record, sketch.id, 25.0, BooleanMode::NewBody, None),
    ]);
    let (prepared, _) = run_plan(wm, plan_request(&timeline, 0xF012, epoch)).await?;
    if prepared.stopped_reason != StoppedReason::Completed {
        return Err(format!(
            "symmetric box stopped: {:?}",
            prepared.stopped_reason
        ));
    }
    accept(wm, 0xF012, epoch).await?;
    let mesh = mesh_for(wm, body, prepared.prepared_snapshot_id).await?;
    let view = validate_mesh_blob(&mesh).map_err(|e| format!("symmetric MESH1: {e}"))?;
    let centre = Vec3::new_unchecked(
        f64::from(view.bbox_min[0] + view.bbox_max[0]) / 2.0,
        f64::from(view.bbox_min[1] + view.bbox_max[1]) / 2.0,
        f64::from(view.bbox_min[2] + view.bbox_max[2]) / 2.0,
    );
    let result = wm
        .resolve_refs(symmetric_request(
            prepared.prepared_snapshot_id,
            body,
            centre,
        ))
        .await
        .map_err(|e| format!("ResolveRefs failed: {e}"))?;
    match result.as_slice() {
        [resolution] if matches!(resolution.outcome, ResolveOutcome::NeedsRepair(_)) => Ok(()),
        [resolution] => Err(format!(
            "symmetric tie must NeedsRepair, got {:?}",
            resolution.outcome
        )),
        other => Err(format!("symmetric tie returned {} rows", other.len())),
    }
}

fn symmetric_request(snapshot_id: SnapshotId, body: BodyId, centre: Vec3) -> ResolveRequest {
    ResolveRequest {
        snapshot_id,
        refs: vec![ResolveRef {
            ref_id: "resolve_symmetric_ref".into(),
            element: ElementRef {
                primary: Some(PrimaryRef {
                    body,
                    element: ElementId::new("el_center"),
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: Some(AnchorIntent {
                    world_point: centre,
                    surface_uv: Some(Vec2::new_unchecked(0.5, 0.5)),
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
                extra: Default::default(),
            },
        }],
    }
}

fn point_id(label: &str) -> EntityId {
    EntityId(stable_uuid(label))
}

fn constraint_id(label: &str) -> ConstraintId {
    ConstraintId(stable_uuid(label))
}

fn line_sketch(label: &str, a: Vec2, b: Vec2) -> (Sketch, EntityId, EntityId, EntityId) {
    let mut sketch = Sketch::on_world_plane(SketchId(stable_uuid(label)), label, WorldPlane::XY);
    let p0 = point_id(&format!("{label}:p0"));
    let p1 = point_id(&format!("{label}:p1"));
    let line = point_id(&format!("{label}:line"));
    sketch
        .add_entity(SketchEntity::point(p0, a, false, false))
        .expect("line p0");
    sketch
        .add_entity(SketchEntity::point(p1, b, false, false))
        .expect("line p1");
    sketch
        .add_entity(SketchEntity::line(line, p0, p1, false))
        .expect("line");
    (sketch, p0, p1, line)
}

async fn drag_constrained_line(
    wm: &WorkerManager,
    id: &str,
    vertical: bool,
    target: [f64; 2],
    expected: [f64; 2],
    tolerance: f64,
    gesture_id: u64,
) -> Result<(), String> {
    let (mut sketch, dragged, _other, line) = if vertical {
        line_sketch(
            id,
            Vec2::new_unchecked(0.0, 0.0),
            Vec2::new_unchecked(0.0, 6.0),
        )
    } else {
        line_sketch(
            id,
            Vec2::new_unchecked(0.0, 0.0),
            Vec2::new_unchecked(12.0, 0.0),
        )
    };
    let constraint = if vertical {
        Constraint::Vertical {
            id: constraint_id(&format!("{id}:constraint")),
            line,
        }
    } else {
        Constraint::Horizontal {
            id: constraint_id(&format!("{id}:constraint")),
            line,
        }
    };
    sketch
        .add_constraint(constraint)
        .map_err(|e| format!("{id}: {e:?}"))?;
    let (solve, end) = solve_point_drag(wm, &sketch, dragged, target, gesture_id, id).await?;
    let position = end
        .solved_positions
        .get(&dragged.to_string())
        .or_else(|| solve.positions.get(&dragged.to_string()))
        .ok_or_else(|| format!("{id}: dragged point absent from solve result"))?;
    if (position[0] - expected[0]).abs() > tolerance
        || (position[1] - expected[1]).abs() > tolerance
    {
        return Err(format!(
            "{id}: position {position:?}, expected {expected:?}"
        ));
    }
    Ok(())
}

async fn solve_point_drag(
    wm: &WorkerManager,
    sketch: &Sketch,
    dragged: EntityId,
    target: [f64; 2],
    gesture_id: u64,
    label: &str,
) -> Result<
    (
        onecad_lib::dto::DragSolveDto,
        onecad_lib::dto::SketchUpsertDto,
    ),
    String,
> {
    let upsert = wm
        .sketch_upsert(sketch)
        .await
        .map_err(|e| format!("{label}: SketchUpsert: {e}"))?;
    wm.begin_gesture(
        &sketch.id.to_string(),
        upsert.sketch_revision,
        gesture_id,
        dragged,
        GestureTarget::point(dragged),
        "",
    )
    .await
    .map_err(|e| format!("{label}: BeginGesture: {e}"))?;
    let solve = wm
        .solve_drag(gesture_id, 1, dragged, target)
        .await
        .map_err(|e| format!("{label}: SolveDrag: {e}"))?;
    if solve.status != "success" {
        return Err(format!("{label}: expected success, got {}", solve.status));
    }
    let end = wm
        .end_gesture(&sketch.id.to_string(), gesture_id, Some(target))
        .await
        .map_err(|e| format!("{label}: EndGesture: {e}"))?;
    Ok((solve, end))
}

fn rectangle_drag_fixture() -> Result<(Sketch, [EntityId; 4]), String> {
    let label = "drag_rectangle_corner";
    let mut sketch = Sketch::on_world_plane(SketchId(stable_uuid(label)), label, WorldPlane::XY);
    let points = [
        point_id("drag-rect:p0"),
        point_id("drag-rect:p1"),
        point_id("drag-rect:p2"),
        point_id("drag-rect:p3"),
    ];
    let positions = [
        Vec2::new_unchecked(0.0, 0.0),
        Vec2::new_unchecked(10.0, 0.0),
        Vec2::new_unchecked(10.0, 6.0),
        Vec2::new_unchecked(0.0, 6.0),
    ];
    let lines = [
        point_id("drag-rect:l0"),
        point_id("drag-rect:l1"),
        point_id("drag-rect:l2"),
        point_id("drag-rect:l3"),
    ];
    add_rectangle_entities(&mut sketch, points, positions, lines)?;
    add_rectangle_constraints(&mut sketch, points, positions, lines)?;
    Ok((sketch, points))
}

fn add_rectangle_entities(
    sketch: &mut Sketch,
    points: [EntityId; 4],
    positions: [Vec2; 4],
    lines: [EntityId; 4],
) -> Result<(), String> {
    for (id, position) in points.into_iter().zip(positions) {
        sketch
            .add_entity(SketchEntity::point(id, position, false, false))
            .map_err(|e| format!("rectangle point: {e:?}"))?;
    }
    for index in 0..4 {
        sketch
            .add_entity(SketchEntity::line(
                lines[index],
                points[index],
                points[(index + 1) % 4],
                false,
            ))
            .map_err(|e| format!("rectangle line: {e:?}"))?;
    }
    Ok(())
}

fn add_rectangle_constraints(
    sketch: &mut Sketch,
    points: [EntityId; 4],
    positions: [Vec2; 4],
    lines: [EntityId; 4],
) -> Result<(), String> {
    for (index, line) in lines.into_iter().enumerate() {
        let constraint = if index % 2 == 0 {
            Constraint::Horizontal {
                id: constraint_id(&format!("drag-rect:c{index}")),
                line,
            }
        } else {
            Constraint::Vertical {
                id: constraint_id(&format!("drag-rect:c{index}")),
                line,
            }
        };
        sketch
            .add_constraint(constraint)
            .map_err(|e| format!("rectangle constraint: {e:?}"))?;
    }
    sketch
        .add_constraint(Constraint::Fixed {
            id: constraint_id("drag-rect:fixed-opposite"),
            point: points[2],
            point_position: CurvePosition::Arbitrary,
            at: positions[2],
        })
        .map_err(|e| format!("rectangle fixed point: {e:?}"))?;
    Ok(())
}

async fn drag_rectangle_corner(wm: &WorkerManager, tolerance: f64) -> Result<(), String> {
    let (sketch, points) = rectangle_drag_fixture()?;
    let (solve, end) =
        solve_point_drag(wm, &sketch, points[0], [-2.0, -1.0], 0x7103, "rectangle").await?;
    let read = |point: EntityId, original: [f64; 2]| {
        end.solved_positions
            .get(&point.to_string())
            .copied()
            .or_else(|| solve.positions.get(&point.to_string()).copied())
            .unwrap_or(original)
    };
    let dragged = read(points[0], [0.0, 0.0]);
    let opposite = read(points[2], [10.0, 6.0]);
    if (dragged[0] + 2.0).abs() > tolerance
        || (dragged[1] + 1.0).abs() > tolerance
        || (opposite[0] - 10.0).abs() > tolerance
        || (opposite[1] - 6.0).abs() > tolerance
    {
        return Err(format!(
            "rectangle drag mismatch: dragged={dragged:?}, opposite={opposite:?}"
        ));
    }
    Ok(())
}

fn dof_claim<'a>(expected: &'a SolverExpected, state: &str) -> Result<&'a DofClaim, String> {
    expected
        .dof
        .iter()
        .find(|claim| claim.state == state)
        .ok_or_else(|| format!("missing typed DOF claim {state}"))
}

async fn verify_dof(wm: &WorkerManager, expected: &SolverExpected) -> Result<(), String> {
    let (mut dof_sketch, _p0, _p1, line) = line_sketch(
        "corpus-g-dof",
        Vec2::new_unchecked(0.0, 0.0),
        Vec2::new_unchecked(10.0, 0.0),
    );
    let two_free = wm
        .sketch_upsert(&dof_sketch)
        .await
        .map_err(|e| format!("DOF initial: {e}"))?;
    if two_free.dof != dof_claim(expected, "twoFreePoints")?.dof {
        return Err(format!("two-free DOF {}, expected 4", two_free.dof));
    }
    dof_sketch
        .add_constraint(Constraint::Horizontal {
            id: constraint_id("corpus-g-horizontal-1"),
            line,
        })
        .map_err(|e| format!("horizontal: {e:?}"))?;
    let horizontal = wm
        .sketch_upsert(&dof_sketch)
        .await
        .map_err(|e| format!("DOF horizontal: {e}"))?;
    if horizontal.dof != dof_claim(expected, "afterHorizontal")?.dof {
        return Err(format!("horizontal DOF {}, expected 3", horizontal.dof));
    }
    // The document authoring boundary canonicalizes an identical duplicate before
    // syncing PlaneGCS. The frozen oracle's claim is idempotence, not two solver
    // equations with different ids (which PlaneGCS correctly diagnoses redundant).
    let redundant = wm
        .sketch_upsert(&dof_sketch)
        .await
        .map_err(|e| format!("DOF redundant: {e}"))?;
    let redundant_claim = dof_claim(expected, "afterRedundantDuplicateHorizontal")?;
    if redundant.dof != redundant_claim.dof
        || redundant_claim.over_constrained == Some(false)
            && matches!(
                redundant.status,
                onecad_lib::dto::SketchSolveStatus::OverConstrained
                    | onecad_lib::dto::SketchSolveStatus::Conflicting
            )
    {
        return Err(format!("redundant constraint changed solve: {redundant:?}"));
    }
    Ok(())
}

fn drag_claim<'a>(expected: &'a SolverExpected, id: &str) -> Result<&'a DragClaim, String> {
    expected
        .drags
        .iter()
        .find(|claim| claim.id == id)
        .ok_or_else(|| format!("missing typed drag claim {id}"))
}

async fn verify_drags(
    wm: &WorkerManager,
    case: &CorpusCase,
    expected: &SolverExpected,
) -> Result<(), String> {
    let vertical = drag_claim(expected, "drag_vertical")?;
    if vertical.status != "success" {
        return Err("drag_vertical frozen status is not success".into());
    }
    drag_constrained_line(
        wm,
        "drag_vertical",
        true,
        [0.0, 9.0],
        [0.0, 9.0],
        vertical.tol.unwrap_or(1e-4),
        0x7101,
    )
    .await?;
    let horizontal = drag_claim(expected, "drag_horizontal")?;
    if horizontal.status != "success" {
        return Err("drag_horizontal frozen status is not success".into());
    }
    drag_constrained_line(
        wm,
        "drag_horizontal",
        false,
        [7.25, 0.0],
        [7.25, 0.0],
        horizontal.tol.unwrap_or(1e-4),
        0x7102,
    )
    .await?;
    let rectangle = drag_claim(expected, "drag_rectangle_corner")?;
    if rectangle.status != "success" {
        return Err("drag_rectangle_corner frozen status is not success".into());
    }
    drag_rectangle_corner(wm, rectangle.tol.unwrap_or(1e-4)).await?;

    let scripted_drags: BTreeSet<&str> = case
        .op_script
        .iter()
        .filter(|op| op.op_type == "SolveDrag")
        .map(|op| op.op_id.as_str())
        .collect();
    for id in ["drag_vertical", "drag_horizontal", "drag_rectangle_corner"] {
        if !scripted_drags.contains(id) || drag_claim(expected, id)?.status != "success" {
            return Err(format!(
                "scripted drag {id} lacks an executable success claim"
            ));
        }
    }
    Ok(())
}

async fn run_sketch_solver(wm: &WorkerManager, case: &CorpusCase) -> Result<(), String> {
    let expected: SolverExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    verify_dof(wm, &expected).await?;
    verify_drags(wm, case, &expected).await
}
