use super::*;

pub(super) fn worker_path() -> Option<PathBuf> {
    let path = std::env::var("ONECAD_WORKER_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    assert!(
        path.as_ref().is_none_or(|p| p.is_file()),
        "configured worker must exist"
    );
    assert!(
        path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 requires a real worker"
    );
    path
}

pub(super) fn entity(id: u128) -> EntityId {
    EntityId(Uuid::from_u128(id))
}

pub(super) fn skew_rectangle(id: SketchId) -> Sketch {
    let corners = [[0.0, 0.0], [80.0, 1.1], [80.7, 61.0], [0.5, 60.5]];
    let mut sketch = Sketch::on_world_plane(id, "Skew rectangle", WorldPlane::XY);
    for side in 0..4 {
        for end in 0..2 {
            let at = corners[(side + end) % 4];
            sketch
                .add_entity(SketchEntity::point(
                    entity((100 + side * 2 + end) as u128),
                    Vec2::new(at[0], at[1]).unwrap(),
                    false,
                    false,
                ))
                .unwrap();
        }
    }
    for side in 0..4 {
        let line = entity(200 + side);
        sketch
            .add_entity(SketchEntity::line(
                line,
                entity(100 + side * 2),
                entity(101 + side * 2),
                false,
            ))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident {
                id: ConstraintId(Uuid::from_u128(300 + side)),
                point1: entity(101 + side * 2),
                point2: entity(100 + ((side + 1) % 4) * 2),
                point1_position: CurvePosition::Arbitrary,
                point2_position: CurvePosition::Arbitrary,
            })
            .unwrap();
        let id = ConstraintId(Uuid::from_u128(304 + side));
        sketch
            .add_constraint(if side % 2 == 0 {
                Constraint::Horizontal { id, line }
            } else {
                Constraint::Vertical { id, line }
            })
            .unwrap();
    }
    sketch
}

pub(super) fn ops(sketch: &Sketch) -> Vec<SketchEditOp> {
    sketch
        .entities()
        .iter()
        .cloned()
        .map(|entity| SketchEditOp::AddEntity { entity })
        .chain(
            sketch
                .constraints()
                .iter()
                .cloned()
                .map(|constraint| SketchEditOp::AddConstraint { constraint }),
        )
        .collect()
}

pub(super) fn point(session: &SketchSessionDto, id: u128) -> [f64; 2] {
    let point = session
        .entities
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == entity(id).to_string())
        .unwrap();
    [
        point["at"][0].as_f64().unwrap(),
        point["at"][1].as_f64().unwrap(),
    ]
}

pub(super) fn assert_adopted(session: &SketchSessionDto, profile: &[f64]) {
    for side in 0..4 {
        let a = point(session, 100 + side * 2);
        let b = point(session, 101 + side * 2);
        assert_eq!(
            a[(side % 2 == 0) as usize],
            b[(side % 2 == 0) as usize],
            "H/V side {side} must be solved"
        );
    }
    for point_id in 100..108 {
        let at = point(session, point_id);
        assert!(
            profile
                .as_chunks::<2>()
                .0
                .iter()
                .any(|p| (p[0] - at[0]).abs() < 1e-8 && (p[1] - at[1]).abs() < 1e-8),
            "document point {point_id} must match solved profile vertex"
        );
    }
}

pub(super) fn extrude(sketch: SketchId, region: &str, version: u32) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(region),
            region_identity_version: Some(version),
            region_anchor: Some([40.0, 30.0]),
            extra: Default::default(),
        }),
        distance: Scalar::new(10.0),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(0xe013)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

pub(super) async fn mesh_matches_profile(runtime: &mut DocumentRuntime, profile: &[f64]) {
    let report = runtime
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    let mesh = runtime
        .get_mesh(BodyId(Uuid::from_u128(0xe013)), Lod::Coarse, None)
        .await
        .unwrap();
    let view = validate_mesh_blob(&mesh).unwrap();
    let positions = view.section(1).unwrap().offset as usize;
    eprintln!(
        "Extrude mesh bounds={:?}/{:?}; expected profile={profile:?}",
        view.bbox_min, view.bbox_max
    );
    for index in 0..view.vertex_count as usize {
        let offset = positions + index * 12;
        let xyz = [
            f32_le(&mesh, offset) as f64,
            f32_le(&mesh, offset + 4) as f64,
            f32_le(&mesh, offset + 8) as f64,
        ];
        assert!(
            profile.as_chunks::<2>().0.iter().any(|p| {
                let world = SketchPlane::xy().to_world(Vec2::new(p[0], p[1]).unwrap());
                (xyz[0] - world.x).abs() < 1e-5 && (xyz[1] - world.y).abs() < 1e-5
            }),
            "Extrude vertex {xyz:?} must use the solved profile, not authored skew geometry"
        );
        assert!(xyz[2].abs() < 1e-5 || (xyz[2] - 10.0).abs() < 1e-5);
    }
}
