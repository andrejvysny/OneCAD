use super::*;

pub(super) async fn countersinks(h: &Harness, recipe: &Value, body: BodyId) {
    let worker = h.state.live_worker().unwrap();
    let mut face_key = None;
    for k in 1.. {
        let key = format!("f:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        if dto
            .frame
            .filter(|_| dto.surface_type == "plane")
            .is_some_and(|f| {
                f.normal == Some([0.0, 0.0, 1.0]) && (f.origin[2] - 3.03784).abs() < 1e-9
            })
        {
            face_key = Some(key);
            break;
        }
    }
    let key = face_key.expect("actual FTC09 top face");
    let promoted = {
        let mut guard = h.state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.promote_selection(
            rt.head_snapshot_id().unwrap(),
            body,
            vec![(TopoKey::new(&key), None)],
        )
        .await
        .unwrap()
    };
    let point =
        |p: &Value| Vec3::new_unchecked(p[0].as_f64().unwrap(), -p[1].as_f64().unwrap(), 3.03784);
    let points: Vec<_> = recipe["countersinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| point(&c["center"]))
        .collect();
    let at = Vec3::new_unchecked(0.0, 0.0, 3.03784);
    let mut params = HoleParams {
        target_body: body,
        face: ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(&promoted[0].element_id),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: at,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        },
        point: points[0],
        points: None,
        hole_type: HoleType::Countersink,
        diameter: Scalar::new(5.9436),
        depth: None,
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: Some(Scalar::new(7.4676)),
        cs_angle_deg: Some(Scalar::new(90.0)),
        thread: None,
        result_policy_version: Some(2),
        extra: Default::default(),
    };
    params
        .set_placements(
            points
                .into_iter()
                .map(|at| HolePlacement { id: None, at })
                .collect(),
        )
        .unwrap();
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                RecordId(Uuid::from_u128(0xf7c09400)),
                1,
                "FTC09 countersinks",
                Operation::Known(KnownOperation::Hole(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(super) async fn pocket(h: &Harness, wires: &Value, body: BodyId, index: u128) {
    let sid = SketchId(Uuid::from_u128(0xf7c09100 + index));
    let datum = DatumPlaneId(Uuid::from_u128(0xf7c09200 + index));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "FTC09 top", "XY", 3.03784),
        },
    )
    .await
    .unwrap();
    let mut sketch = Sketch::on_world_plane(sid, "FTC09 pocket", WorldPlane::XY);
    sketch.attachment = SketchAttachment::Datum { datum };
    api::apply_edit_command(&h.state, h.host.clone(), EditCommand::AddSketch { sketch })
        .await
        .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sid.to_string(), Some(true))
        .await
        .unwrap();
    api::sketch_upsert(&h.state, h.host.clone(), sid.to_string(), entities(wires))
        .await
        .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), sid.to_string())
        .await
        .unwrap();
    assert_eq!(finish.regions.len(), 1);
    h.settled(&h.projection().await).await;
    let mut operation = native_plate_support::profile::extrude(&finish, -1.27);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut operation else {
        unreachable!()
    };
    params.profile.as_mut().unwrap().sketch = sid;
    params.boolean_mode = BooleanMode::Cut;
    params.target_body = Some(body);
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                RecordId(Uuid::from_u128(0xf7c09300 + index)),
                1,
                "FTC09 pocket",
                operation,
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&applied.projection).await;
}

pub(super) fn verify_source() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let provenance: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/reference-histories/nist-ftc09-provenance.json"
    ))
    .unwrap();
    for (path, key) in [
        (
            "fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp",
            "sourceSha256",
        ),
        (
            "fixtures/reference-histories/nist-ftc09-curves.json",
            "recipeSha256",
        ),
        (
            "fixtures/reference-histories/extract_ftc09.py",
            "extractorSha256",
        ),
    ] {
        let digest = Sha256::digest(std::fs::read(root.join(path)).unwrap());
        assert_eq!(format!("{digest:x}"), provenance[key].as_str().unwrap());
    }
}

pub(super) async fn verify_geometry(h: &Harness, body: BodyId) {
    verify_bounds(h, body).await;
    verify_countersink_edges(h, body).await;
}

async fn verify_bounds(h: &Harness, body: BodyId) {
    use onecad_protocol::mesh::{MeshLimits, MeshView};
    let projection = h.projection().await;
    let blob = api::get_mesh(
        &h.state,
        body.to_string(),
        "fine".into(),
        None,
        Some(projection.runtime_session),
    )
    .await
    .unwrap();
    let mesh = MeshView::decode(&blob, MeshLimits::default()).unwrap();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for i in 0..mesh.header().vertex_count as usize {
        let p = mesh.position(i).unwrap();
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let provenance: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/reference-histories/nist-ftc09-provenance.json"
    ))
    .unwrap();
    let low = &provenance["sourceAnalyticBoundsMm"]["min"];
    let high = &provenance["sourceAnalyticBoundsMm"]["max"];
    let n = |v: &Value| v.as_f64().unwrap() as f32;
    let expected_min = [n(&low[0]), -n(&high[2]), n(&low[1])];
    let expected_max = [n(&high[0]), -n(&low[2]), n(&high[1])];
    for axis in 0..3 {
        let tolerance = f32::EPSILON * expected_max[axis].abs().max(1.0);
        assert!(
            (min[axis] - expected_min[axis]).abs() <= tolerance,
            "min axis {axis}: {}",
            min[axis]
        );
        assert!(
            (max[axis] - expected_max[axis]).abs() <= tolerance,
            "max axis {axis}: {}",
            max[axis]
        );
    }
}

async fn verify_countersink_edges(h: &Harness, body: BodyId) {
    let recipe: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/reference-histories/nist-ftc09-curves.json"
    ))
    .unwrap();
    let source = recipe["countersinks"].as_array().unwrap();
    let mut found = [[false; 2]; 4];
    let mut asymmetric_hole = false;
    let worker = h.state.live_worker().unwrap();
    for k in 1.. {
        let Ok(Some(dto)) =
            ElementQuery::classify_element_by_topo_key(&worker, body, &format!("e:{k}")).await
        else {
            break;
        };
        let Some(frame) = dto.frame.filter(|_| dto.curve_type == "circle") else {
            continue;
        };
        let Some(radius) = frame.radius else { continue };
        let close =
            |a: f64, b: f64| (a - b).abs() <= 128.0 * f64::EPSILON * a.abs().max(b.abs()).max(1.0);
        if close(frame.origin[0], -0.150114)
            && close(frame.origin[1], 101.6)
            && close(frame.origin[2], 0.0)
            && close(radius, 3.175)
        {
            asymmetric_hole = true;
        }
        for (i, corner) in source.iter().enumerate() {
            let center = &corner["center"];
            let x = center[0].as_f64().unwrap();
            let y = -center[1].as_f64().unwrap();
            if !close(frame.origin[0], x) || !close(frame.origin[1], y) {
                continue;
            }
            for (j, (z, r)) in [
                (0.0, corner["lowerRadius"].as_f64().unwrap()),
                (3.03784, corner["upperRadius"].as_f64().unwrap()),
            ]
            .into_iter()
            .enumerate()
            {
                if close(frame.origin[2], z) && close(radius, r) {
                    found[i][j] = true;
                }
            }
        }
    }
    assert_eq!(
        found, [[true; 2]; 4],
        "each source corner must retain its through-hole and countersink circle"
    );
    assert!(
        asymmetric_hole,
        "asymmetric source hole must preserve handed position"
    );
}
