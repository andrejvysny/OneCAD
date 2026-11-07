use super::*;

fn mesh_bounds_and_volume(view: &MeshView<'_>) -> ([[f64; 3]; 2], f64) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut six_volume = 0.0;
    for i in 0..view.header().triangle_count as usize {
        let [a, b, c] = view
            .triangle(i)
            .unwrap()
            .map(|v| view.position(v as usize).unwrap().map(f64::from));
        for point in [a, b, c] {
            for axis in 0..3 {
                lo[axis] = lo[axis].min(point[axis]);
                hi[axis] = hi[axis].max(point[axis]);
            }
        }
        six_volume += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    ([lo, hi], six_volume.abs() / 6.0)
}
pub(super) fn source_mesh(index: usize, meshes: &[Arc<Vec<u8>>]) {
    if index > 6 {
        return;
    }
    let view = MeshView::decode(&meshes[0], MeshLimits::default()).unwrap();
    let (bounds, volume) = mesh_bounds_and_volume(&view);
    let (expected, reference_volume) = if (1..=4).contains(&index) {
        let data: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../fixtures/reference-derivatives/robonine/analytic/observations.json"
        ))
        .unwrap();
        let faces = [181, 168, 108, 96];
        assert_eq!(view.header().face_count, faces[index - 1]);
        (
            data["parts"][index - 1]["before"]["bounds_mm"].clone(),
            None,
        )
    } else {
        let data: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../fixtures/reference-oracles/prusa-sensor.json"
        ))
        .unwrap();
        let name = match index {
            0 => "box",
            5 => "cover",
            6 => "lever",
            _ => unreachable!(),
        };
        (
            data["parts"][name]["boundsMm"].clone(),
            data["parts"][name]["volumeMm3"].as_f64(),
        )
    };
    for side in 0..2 {
        for axis in 0..3 {
            let target = expected[side][axis].as_f64().unwrap();
            assert!(
                (bounds[side][axis] - target).abs() <= 0.15,
                "definition {index} source bounds"
            );
        }
    }
    if let Some(expected) = reference_volume {
        assert!(
            (volume - expected).abs() <= expected * 0.02,
            "definition {index} source volume trial"
        );
    }
}

async fn edit_cut(h: &Harness, session: &str, lever: &Path) {
    let saved = ContainerReader::open(lever).unwrap();
    let mut op = saved
        .document()
        .timeline
        .records()
        .iter()
        .find(|r| r.record_id == RecordId(Uuid::from_u128(0x531020)))
        .unwrap()
        .op
        .clone();
    let Operation::Known(KnownOperation::Extrude(ref mut params)) = op else {
        panic!("lever cut kind")
    };
    params.distance = Scalar::new(4.0);
    edit(
        &h.state,
        session,
        AssemblyEdit::EditDefinition {
            id: fixture::def(6),
            command: Box::new(EditCommand::UpdateOperationParams {
                record: RecordId(Uuid::from_u128(0x531020)),
                op,
            }),
        },
    )
    .await;
    replay(&h.state, session, &[6]).await;
}
pub(super) async fn translate(state: &AppState, session: &str, record: u128, distance: f64) {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let part = projection
        .definitions
        .iter()
        .find(|p| p.id == fixture::def(6).to_string())
        .unwrap();
    let targets = part
        .bodies
        .iter()
        .map(|body| body.id.parse().unwrap())
        .collect();
    let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets,
        translate: [Scalar::new(distance), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: Default::default(),
        copy: false,
        extra: Default::default(),
    }));
    edit(
        state,
        session,
        AssemblyEdit::EditDefinition {
            id: fixture::def(6),
            command: Box::new(EditCommand::AddOperation {
                record: OperationRecord::new(
                    RecordId(Uuid::from_u128(record)),
                    0,
                    "Translate native lever",
                    op,
                ),
                at_cursor: true,
            }),
        },
    )
    .await;
    replay(state, session, &[6]).await;
}
async fn exercise_placement(h: &Harness, session: &str) {
    let before = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    let first = before
        .occurrences
        .iter()
        .find(|o| o.id == fixture::occ(12).to_string())
        .unwrap()
        .local;
    let second = before
        .occurrences
        .iter()
        .find(|o| o.id == fixture::occ(13).to_string())
        .unwrap()
        .local;
    let hidden = edit(
        &h.state,
        session,
        AssemblyEdit::SetVisibility {
            occurrence: fixture::occ(13),
            visible: false,
        },
    )
    .await;
    assert!(
        !hidden
            .occurrences
            .iter()
            .find(|o| o.id == fixture::occ(13).to_string())
            .unwrap()
            .visible
    );
    assert!(
        hidden
            .occurrences
            .iter()
            .find(|o| o.id == fixture::occ(12).to_string())
            .unwrap()
            .visible
    );
    edit(
        &h.state,
        session,
        AssemblyEdit::SetVisibility {
            occurrence: fixture::occ(13),
            visible: true,
        },
    )
    .await;
    let moved = edit(
        &h.state,
        session,
        AssemblyEdit::SetPose {
            occurrence: fixture::occ(13),
            pose: RigidTransform::new(
                second.rotation,
                second.translation + Vec3d::new(2.0, 0.0, 0.0),
            ),
        },
    )
    .await;
    assert_eq!(
        moved
            .occurrences
            .iter()
            .find(|o| o.id == fixture::occ(12).to_string())
            .unwrap()
            .local,
        first
    );
}
pub(super) async fn exercise(h: &Harness, session: &str, lever: &Path) {
    exercise_placement(h, session).await;
    let before = meshes(&h.state, session, 6, 12).await;
    edit_cut(h, session, lever).await;
    let changed = meshes(&h.state, session, 6, 12).await;
    mesh_compare::different(
        "shared native cut",
        before[0].as_slice(),
        changed[0].as_slice(),
    );
    assert!(Arc::ptr_eq(
        &changed[0],
        &meshes(&h.state, session, 6, 13).await[0]
    ));
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[6]).await;
    mesh_compare::equal(
        "undo native cut",
        before[0].as_slice(),
        meshes(&h.state, session, 6, 12).await[0].as_slice(),
    );
    api::redo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[6]).await;
    mesh_compare::equal(
        "redo native cut",
        changed[0].as_slice(),
        meshes(&h.state, session, 6, 12).await[0].as_slice(),
    );
    edit(
        &h.state,
        session,
        AssemblyEdit::MakeIndependent {
            occurrence: fixture::occ(13),
            new_definition: fixture::def(10),
            name: "Independent native lever".into(),
        },
    )
    .await;
    replay(&h.state, session, &[10]).await;
    let independent = meshes(&h.state, session, 10, 13).await;
    mesh_compare::equal(
        "native independent copy",
        changed[0].as_slice(),
        independent[0].as_slice(),
    );
    translate(&h.state, session, 0xD602, 1.0).await;
    mesh_compare::different(
        "translation changes original",
        independent[0].as_slice(),
        meshes(&h.state, session, 6, 12).await[0].as_slice(),
    );
    mesh_compare::equal(
        "translation preserves independent copy",
        independent[0].as_slice(),
        meshes(&h.state, session, 10, 13).await[0].as_slice(),
    );
}

pub(super) async fn verify_nist(root: &Path) {
    let oracle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/nist-ftc09.json"
    ))
    .unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp");
    let h = Harness::new(root).await;
    api::import_step(
        &h.state,
        h.host.clone(),
        source.to_string_lossy().into_owned(),
    )
    .await
    .unwrap();
    let projection = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let p = h.projection().await;
            if p.bodies.len() == 1
                && p.features.len() == 1
                && p.features
                    .iter()
                    .all(|f| f.status == onecad_app::dto::FeatureStatus::Ok)
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("NIST source import did not publish a valid solid");
    let body: BodyId = projection.bodies.keys().next().unwrap().parse().unwrap();
    let context = api::preview_context(&h.state).await.unwrap();
    let mass = api::query_mass_properties(
        &h.state,
        onecad_app::worker::wire::body_id_wire(body),
        Some(onecad_app::dto::GeometryReadFenceDto {
            document_id: context.document_id,
            runtime_session: context.runtime_session,
            snapshot_id: context.snapshot_id.0,
        }),
    )
    .await
    .unwrap();
    let tolerance = oracle["trialRelativeTolerance"].as_f64().unwrap();
    assert_eq!(tolerance, 1e-6, "preserve the preregistered NIST trial");
    eprintln!("NIST 1e-6 relative trial is not a qualified accuracy bound; source/transfer uncertainty and second-reader crosscheck remain unresolved");
    for (actual, expected) in [
        (mass.volume, oracle["volumeMm3"].as_f64().unwrap()),
        (
            mass.surface_area,
            oracle["surfaceAreaMm2"].as_f64().unwrap(),
        ),
    ] {
        assert!((actual-expected).abs() <= expected.abs()*tolerance, "NIST upstream property: actual={actual}, expected={expected}, relative trial={tolerance}");
    }
    h.close().await;
}
