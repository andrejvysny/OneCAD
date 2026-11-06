use super::*;

pub(crate) async fn clip_profile(
    h: &Harness,
    n: u128,
    x: f64,
    wires: &Value,
) -> onecad_app::dto::FinishSketchDto {
    let sid = sketch(n);
    let datum = DatumPlaneId(Uuid::from_u128(0xfac0300 + n));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "Clip floor", "XZ", x),
        },
    )
    .await
    .unwrap();
    let mut s = Sketch::on_world_plane(sid, "Source clip floor", WorldPlane::XZ);
    s.attachment = SketchAttachment::Datum { datum };
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddSketch { sketch: s },
    )
    .await
    .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sid.to_string(), Some(true))
        .await
        .unwrap();
    let mut ops = Vec::new();
    let curves = wires[0].as_array().unwrap();
    for (i, curve) in curves.iter().enumerate() {
        assert_eq!(curve["kind"], "line");
        let a = EntityId(Uuid::from_u128(0xfac2000 + 3 * i as u128));
        let b = EntityId(Uuid::from_u128(0xfac2001 + 3 * i as u128));
        let line = EntityId(Uuid::from_u128(0xfac2002 + 3 * i as u128));
        for (id, coord) in [(a, &curve["start"]), (b, &curve["end"])] {
            let x = coord[0].as_f64().unwrap();
            let y = coord[1].as_f64().unwrap();
            ops.push(SketchEditOp::AddEntity {
                entity: SketchEntity::point(id, Vec2::new_unchecked(x, y), false, false),
            });
        }
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::line(line, a, b, false),
        });
    }
    api::sketch_upsert(&h.state, h.host.clone(), sid.to_string(), ops)
        .await
        .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), sid.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    finish
}

pub(crate) async fn fillet_clip_floors(h: &Harness, body: BodyId, recipe: &Value, side: usize) {
    fillet_clip_edges(h, body, recipe, side, 120 + side as u128, 6).await;
}

pub(crate) async fn fillet_clip_edges(
    h: &Harness,
    body: BodyId,
    recipe: &Value,
    side: usize,
    n: u128,
    expected_edges: usize,
) {
    use onecad_app::worker::ElementQuery;
    use onecad_core::{
        document::{
            record::FilletParams,
            refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
            variables::Scalar,
        },
        ids::{ElementId, TopoKey},
        math::Vec3,
    };
    let worker = h.state.live_worker().unwrap();
    let snapshot = h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .head_snapshot_id()
        .unwrap();
    let mut lines = Vec::new();
    let mut splines = Vec::new();
    for k in 1.. {
        let key = format!("e:{k}");
        let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(&worker, body, &key).await
        else {
            break;
        };
        if dto.curve_type == "line" {
            if let Some(frame) = dto.frame {
                let descriptor = worker
                    .query_element_by_topo_key(snapshot, body, &key)
                    .await
                    .unwrap()
                    .unwrap();
                lines.push((key, frame, descriptor));
            }
        } else if dto.curve_type == "other" && expected_edges == 3 {
            let descriptor = worker
                .query_element_by_topo_key(snapshot, body, &key)
                .await
                .unwrap()
                .unwrap();
            splines.push((key, descriptor));
        }
    }
    let mut matches = std::collections::BTreeMap::new();
    for face in std::iter::once(&recipe["clipFloorFaces"][side]) {
        let x = face["sourceXmm"].as_f64().unwrap();
        for (edge_index, curve) in face["wires"][0].as_array().unwrap().iter().enumerate() {
            if std::env::var_os("ONECAD_FRAMEWORK_CLIP_OMIT_UNSTABLE").is_some()
                && [0, 4].contains(&edge_index)
            {
                continue;
            }
            let a = &curve["start"];
            let b = &curve["end"];
            let p = [
                x,
                (a[0].as_f64().unwrap() + b[0].as_f64().unwrap()) / 2.0,
                (a[1].as_f64().unwrap() + b[1].as_f64().unwrap()) / 2.0,
            ];
            let dy = b[0].as_f64().unwrap() - a[0].as_f64().unwrap();
            let dz = b[1].as_f64().unwrap() - a[1].as_f64().unwrap();
            let length = dy.hypot(dz);
            let found: Vec<_> = lines
                .iter()
                .filter(|(_, f, desc)| {
                    let Some(v) = f.axis else { return false };
                    let q = [p[0] - f.origin[0], p[1] - f.origin[1], p[2] - f.origin[2]];
                    let dot = q[0] * v[0] + q[1] * v[1] + q[2] * v[2];
                    let dist2 = q.iter().map(|v| v * v).sum::<f64>() - dot * dot;
                    let parallel = (dy * v[1] + dz * v[2]).abs() / length;
                    let along =
                        ((desc.center[1] - p[1]) * dy + (desc.center[2] - p[2]) * dz) / length;
                    let contained = along.abs() + desc.magnitude / 2.0 <= length / 2.0 + 1e-5;
                    dist2.abs() < 1e-10 && parallel > 1.0 - 1e-10 && contained
                })
                .collect();
            if found.is_empty() && expected_edges < 6 {
                continue;
            }
            assert!(
                !found.is_empty(),
                "source clip edge has no bounded match: {p:?}"
            );
            for (key, _, descriptor) in found {
                let c = descriptor.center;
                matches.insert(key.clone(), Vec3::new_unchecked(c[0], c[1], c[2]));
            }
        }
    }
    let round_corners =
        expected_edges == 3 && std::env::var_os("ONECAD_FRAMEWORK_ROUND_CLIP_CORNERS").is_some();
    if round_corners {
        let inner_x = recipe["clipFloorFaces"][side]["sourceXmm"]
            .as_f64()
            .unwrap();
        let outer_x = if side == 0 { -16.0 } else { 16.0 };
        let inner = clip_lines(recipe, side, inner_x);
        let outer = clip_lines(recipe, side, outer_x);
        for index in [2, 3] {
            let a = intersect(inner[(index + 5) % 6], inner[index]);
            let b = intersect(outer[(index + 5) % 6], outer[index]);
            let midpoint = [
                (inner_x + outer_x) / 2.0,
                (a[0] + b[0]) / 2.0,
                (a[1] + b[1]) / 2.0,
            ];
            let length =
                ((outer_x - inner_x).powi(2) + (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2))
                    .sqrt();
            for (key, desc) in &splines {
                let error = (0..3)
                    .map(|i| (desc.center[i] - midpoint[i]).powi(2))
                    .sum::<f64>();
                if error < 1e-10 && (desc.magnitude - length).abs() < 1e-5 {
                    let c = desc.center;
                    matches.insert(key.clone(), Vec3::new_unchecked(c[0], c[1], c[2]));
                }
            }
        }
    }
    let expected = if std::env::var_os("ONECAD_FRAMEWORK_CLIP_OMIT_UNSTABLE").is_some() {
        4
    } else {
        expected_edges
    };
    assert!(
        matches.len() >= expected,
        "selected contour must remain covered"
    );
    if expected_edges == 3 {
        assert_eq!(
            matches.len(),
            if round_corners { 4 } else { 3 },
            "half-cutter source edge count"
        );
    }
    let mut edges = Vec::new();
    for (key, at) in matches {
        let key = TopoKey::new(&key);
        let id = if key.is_valid() {
            let mut guard = h.state.runtime.lock().await;
            let runtime = guard.as_mut().unwrap();
            let result = runtime
                .promote_selection(
                    runtime.head_snapshot_id().unwrap(),
                    body,
                    vec![(key.clone(), None)],
                )
                .await
                .unwrap();
            ElementId::new(&result[0].element_id)
        } else {
            ElementId::new(key.as_str())
        };
        eprintln!(
            "clip edge side={side} topo={} element={} point={at:?}",
            key.as_str(),
            id.as_str()
        );
        edges.push(ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: id,
                kind: ElementKind::Edge,
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
        });
    }
    let ids = edges
        .iter()
        .map(|edge| edge.primary.as_ref().unwrap().element.clone())
        .collect();
    let params = FilletParams {
        radius: Scalar::new(0.3),
        edge_ids: ids,
        edges,
        chain_tangent_edges: false,
        tangent_closure_version: None,
        overflow: None,
        overflow_version: None,
        extra: Default::default(),
    };
    let applied = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(n),
                1,
                "Framework clip floor blends",
                Operation::Known(KnownOperation::Fillet(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    let terminal = h.terminal(&applied.projection).await;
    if !terminal.repair_steps.is_empty() {
        let guard = h.state.runtime.lock().await;
        eprintln!(
            "clip blend repair: {:?}",
            guard.as_ref().unwrap().repair_items()
        );
    }
    assert!(terminal.repair_steps.is_empty(), "clip blend: {terminal:?}");
    assert!(terminal.failed_steps.is_empty(), "clip blend: {terminal:?}");
}

fn clip_lines(recipe: &Value, side: usize, x: f64) -> Vec<[f64; 3]> {
    let indices = if side == 0 {
        [65, 34, 38, 40, 41, 72]
    } else {
        [111, 90, 86, 84, 83, 81]
    };
    let planes = recipe["clipCutPlanes"].as_array().unwrap();
    indices
        .into_iter()
        .map(|index| {
            let p = planes.iter().find(|p| p["faceIndex"] == index).unwrap();
            let o = &p["origin"];
            let n = &p["normal"];
            let get = |v: &Value, i| v[i].as_f64().unwrap();
            let nx = get(n, 0);
            let ny = get(n, 1);
            let nz = get(n, 2);
            [
                -nz,
                ny,
                nx * (x - get(o, 0)) - ny * get(o, 1) - nz * get(o, 2),
            ]
        })
        .collect()
}

fn intersect(a: [f64; 3], b: [f64; 3]) -> [f64; 2] {
    let det = a[0] * b[1] - a[1] * b[0];
    assert!(det.abs() > 1e-8);
    [
        (a[1] * b[2] - a[2] * b[1]) / det,
        (a[2] * b[0] - a[0] * b[2]) / det,
    ]
}

pub(crate) fn clip_section(recipe: &Value, side: usize, x: f64, upper: bool) -> [[f64; 2]; 4] {
    let l = clip_lines(recipe, side, x);
    if upper {
        [
            intersect(l[2], l[5]),
            intersect(l[0], l[5]),
            intersect(l[0], l[1]),
            intersect(l[2], l[1]),
        ]
    } else {
        [
            intersect(l[2], l[3]),
            intersect(l[4], l[3]),
            intersect(l[4], l[5]),
            intersect(l[2], l[5]),
        ]
    }
}

pub(crate) fn clip_unfillet_recipe(recipe: &Value) -> Value {
    let mut result = recipe.clone();
    for (side, x) in [-13.2, 13.2].into_iter().enumerate() {
        let lines = clip_lines(recipe, side, x);
        let vertices: Vec<_> = (0..6)
            .map(|i| intersect(lines[(i + 5) % 6], lines[i]))
            .collect();
        let wire: Vec<_> = (0..6)
            .map(|i| json!({"kind":"line","start":vertices[i],"end":vertices[(i+1)%6]}))
            .collect();
        result["clipFloorFaces"][side]["wires"] = json!([wire]);
    }
    result
}

pub(crate) fn rear_clip_sections(
    recipe: &Value,
    side: usize,
    x: f64,
) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let indices = if side == 0 {
        [32, 45, 65]
    } else {
        [93, 102, 111]
    };
    let planes = recipe["clipCutPlanes"].as_array().unwrap();
    let line = |index| {
        let p = planes.iter().find(|p| p["faceIndex"] == index).unwrap();
        let get = |v: &Value, i| v[i].as_f64().unwrap();
        let (o, n) = (&p["origin"], &p["normal"]);
        let (nx, ny, nz) = (get(n, 0), get(n, 1), get(n, 2));
        [
            -nz,
            ny,
            nx * (x - get(o, 0)) - ny * get(o, 1) - nz * get(o, 2),
        ]
    };
    let [top, bottom, end] = indices.map(line);
    assert!(end[0].abs() > 1e-8 && end[1].abs() < 1e-8);
    let start = -end[2] / end[0];
    let tip = intersect(top, bottom);
    let height = |plane: [f64; 3], y: f64| -(plane[0] * y + plane[2]) / plane[1];
    (
        vec![tip, [33.0, height(bottom, 33.0)], [33.0, height(top, 33.0)]],
        vec![[start, 0.0], [33.0, 0.0], [33.0, 7.0], [start, 7.0]],
    )
}

pub(crate) fn rear_gap_sections(recipe: &Value, side: usize, x: f64) -> [Vec<[f64; 2]>; 3] {
    let indices = if side == 0 {
        [32, 45, 65, 34, 40]
    } else {
        [93, 102, 111, 90, 84]
    };
    let planes = recipe["clipCutPlanes"].as_array().unwrap();
    let line = |index| {
        let p = planes.iter().find(|p| p["faceIndex"] == index).unwrap();
        let get = |v: &Value, i| v[i].as_f64().unwrap();
        let (o, n) = (&p["origin"], &p["normal"]);
        let (nx, ny, nz) = (get(n, 0), get(n, 1), get(n, 2));
        [
            -nz,
            ny,
            nx * (x - get(o, 0)) - ny * get(o, 1) - nz * get(o, 2),
        ]
    };
    let [top, bottom, end, roof, floor] = indices.map(line);
    let tip = intersect(top, bottom)[0];
    let start = -end[2] / end[0];
    let height = |plane: [f64; 3], y: f64| -(plane[0] * y + plane[2]) / plane[1];
    [
        vec![
            [25.0, height(floor, 25.0)],
            [33.0, height(floor, 33.0)],
            [33.0, height(roof, 33.0)],
            [25.0, height(roof, 25.0)],
        ],
        vec![[start, 0.0], [33.0, 0.0], [33.0, 7.0], [start, 7.0]],
        vec![[25.0, 0.0], [tip, 0.0], [tip, 7.0], [25.0, 7.0]],
    ]
}

pub(crate) fn rear_upper_section(recipe: &Value, side: usize, x: f64) -> [[f64; 2]; 4] {
    let lines = clip_lines(recipe, side, x);
    let start = -lines[0][2] / lines[0][0];
    let height = |plane: [f64; 3], y: f64| -(plane[0] * y + plane[2]) / plane[1];
    [
        [start, height(lines[5], start)],
        [33.0, height(lines[5], 33.0)],
        [33.0, height(lines[1], 33.0)],
        [start, height(lines[1], start)],
    ]
}
