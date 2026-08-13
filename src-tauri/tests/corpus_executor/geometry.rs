fn rectangle_sketch(label: &str, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let sid = SketchId(stable_uuid(label));
    let mut sketch = Sketch::on_world_plane(sid, label, WorldPlane::XY);
    let point = |n: u128| EntityId(stable_uuid(&format!("{label}:p{n}")));
    let line = |n: u128| EntityId(stable_uuid(&format!("{label}:l{n}")));
    let at = [
        Vec2::new_unchecked(x0, y0),
        Vec2::new_unchecked(x0 + w, y0),
        Vec2::new_unchecked(x0 + w, y0 + h),
        Vec2::new_unchecked(x0, y0 + h),
    ];
    for (index, position) in at.into_iter().enumerate() {
        sketch
            .add_entity(SketchEntity::point(
                point(index as u128),
                position,
                false,
                false,
            ))
            .expect("unique rectangle point");
    }
    for index in 0..4 {
        sketch
            .add_entity(SketchEntity::line(
                line(index),
                point(index),
                point((index + 1) % 4),
                false,
            ))
            .expect("valid rectangle line");
    }
    sketch
}

fn hosted_rectangle_sketch(
    label: &str,
    bounds: [f64; 4],
    body: BodyId,
    face: ElementId,
    anchor: Vec3,
) -> Sketch {
    let [x0, y0, w, h] = bounds;
    let mut sketch = rectangle_sketch(label, x0, y0, w, h);
    sketch.plane = onecad_core::sketch::plane_from_point_normal(
        Vec3::new_unchecked(0.0, 0.0, anchor.z),
        Vec3::new_unchecked(0.0, 0.0, 1.0),
    );
    sketch.attachment = SketchAttachment::HostFace {
        face: ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: face,
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: anchor,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        },
        projected_boundary_version: 1,
    };
    sketch
}

fn sketch_record_for(record: u128, sketch: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sketch);
    let params = SketchOpParams {
        sketch: sketch.id,
        plane: SketchPlaneRef {
            kind: PlaneKind::Custom,
            origin: sketch.plane.origin,
            x_axis: sketch.plane.x_axis,
            y_axis: sketch.plane.y_axis,
            normal: sketch.plane.normal,
            extra: Default::default(),
        },
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: sketch.host_face().cloned(),
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(record)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record_for(
    record: u128,
    sketch: SketchId,
    distance: f64,
    boolean_mode: BooleanMode,
    target: Option<BodyId>,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(record)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(distance),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode,
            target_body: target,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        })),
    )
}

fn fillet_record_for(record: u128, body: BodyId, edge: ElementId, anchor: Vec3) -> OperationRecord {
    let reference = ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: edge.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: anchor,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(record)),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(1.0),
            edge_ids: vec![edge],
            edges: vec![reference],
            chain_tangent_edges: true,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

fn timeline(records: impl IntoIterator<Item = OperationRecord>) -> Timeline {
    let mut timeline = Timeline::new();
    for record in records {
        timeline.insert_at_cursor(record);
    }
    timeline
}

const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;

fn mesh_ids(
    view: &MeshHeaderView,
    blob: &[u8],
    offsets: u32,
    chars: u32,
    count: usize,
) -> Vec<String> {
    let offsets = view.section(offsets).expect("mesh id offsets");
    let chars = view.section(chars).expect("mesh id chars");
    (0..count)
        .map(|index| {
            let lo = u32_le(blob, offsets.offset as usize + index * 4) as usize;
            let hi = u32_le(blob, offsets.offset as usize + (index + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[chars.offset as usize + lo..chars.offset as usize + hi])
                .into_owned()
        })
        .collect()
}

fn vertical_edge(view: &MeshHeaderView, blob: &[u8]) -> Result<(ElementId, Vec3), String> {
    let ranges = view
        .section(SEC_EDGE_RANGES)
        .ok_or("MESH1 has no edge ranges")?;
    let positions = view
        .section(SEC_EDGE_POSITIONS)
        .ok_or("MESH1 has no edge positions")?;
    let ids = mesh_ids(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let mut best: Option<(usize, f64, Vec3)> = None;
    for index in 0..view.edge_count as usize {
        let base = ranges.offset as usize + index * 8;
        let first = u32_le(blob, base) as usize;
        let count = u32_le(blob, base + 4) as usize;
        if count == 0 {
            continue;
        }
        let mut min_z = f64::INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        let mut sum = [0.0; 3];
        for point in 0..count {
            let base = positions.offset as usize + (first + point) * 12;
            let xyz = [
                f32_le(blob, base) as f64,
                f32_le(blob, base + 4) as f64,
                f32_le(blob, base + 8) as f64,
            ];
            min_z = min_z.min(xyz[2]);
            max_z = max_z.max(xyz[2]);
            for axis in 0..3 {
                sum[axis] += xyz[axis];
            }
        }
        let centre = Vec3::new_unchecked(
            sum[0] / count as f64,
            sum[1] / count as f64,
            sum[2] / count as f64,
        );
        let span = max_z - min_z;
        if best.as_ref().is_none_or(|(_, old, _)| span > *old) {
            best = Some((index, span, centre));
        }
    }
    let (index, _, centre) = best.ok_or("MESH1 has no pickable edge")?;
    Ok((ElementId::new(ids[index].clone()), centre))
}

fn top_face(view: &MeshHeaderView, blob: &[u8]) -> Result<(ElementId, Vec3), String> {
    let positions = view
        .section(SEC_POSITIONS)
        .ok_or("MESH1 has no positions")?;
    let indices = view.section(SEC_INDICES).ok_or("MESH1 has no indices")?;
    let ranges = view
        .section(SEC_FACE_RANGES)
        .ok_or("MESH1 has no face ranges")?;
    let ids = mesh_ids(
        view,
        blob,
        SEC_FACE_ID_OFFS,
        SEC_FACE_ID_CHARS,
        view.face_count as usize,
    );
    let mut best: Option<(usize, f64, Vec3)> = None;
    for face in 0..view.face_count as usize {
        let base = ranges.offset as usize + face * 8;
        let first_triangle = u32_le(blob, base) as usize;
        let triangle_count = u32_le(blob, base + 4) as usize;
        let mut sum = [0.0; 3];
        let mut samples = 0usize;
        for triangle in first_triangle..first_triangle + triangle_count {
            for corner in 0..3 {
                let index =
                    u32_le(blob, indices.offset as usize + (triangle * 3 + corner) * 4) as usize;
                let offset = positions.offset as usize + index * 12;
                for (axis, total) in sum.iter_mut().enumerate() {
                    *total += f32_le(blob, offset + axis * 4) as f64;
                }
                samples += 1;
            }
        }
        if samples == 0 {
            continue;
        }
        let centre = Vec3::new_unchecked(
            sum[0] / samples as f64,
            sum[1] / samples as f64,
            sum[2] / samples as f64,
        );
        if best.as_ref().is_none_or(|(_, z, _)| centre.z > *z) {
            best = Some((face, centre.z, centre));
        }
    }
    let (face, _, centre) = best.ok_or("MESH1 has no face")?;
    Ok((ElementId::new(ids[face].clone()), centre))
}

async fn mesh_for(
    wm: &WorkerManager,
    body: BodyId,
    snapshot: SnapshotId,
) -> Result<Vec<u8>, String> {
    wm.fetch_mesh(body, Lod::Coarse, snapshot)
        .await
        .map_err(|e| format!("tessellation failed: {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Assertions against `expected`
// ─────────────────────────────────────────────────────────────────────────────

fn check_measured_volume(want: f64, tol: f64, actual: f64, what: &str) -> Result<(), String> {
    if (actual - want).abs() > tol {
        return Err(format!(
            "{what}: volume {actual} not within {tol} of {want}"
        ));
    }
    Ok(())
}

fn check_volume_claim(claim: &VolumeClaim, actual: f64, what: &str) -> Result<(), String> {
    check_measured_volume(claim.value()?, claim.tol.unwrap_or(1e-6), actual, what)
}

async fn volume_of(wm: &WorkerManager, body: BodyId) -> Result<f64, String> {
    let label = format!("body_{}", body.0);
    wm.query_mass_properties(body, label)
        .await
        .map(|mass| mass.volume)
        .map_err(|e| format!("mass query failed: {e}"))
}

async fn topology_of(
    wm: &WorkerManager,
    body: BodyId,
) -> Result<onecad_lib::dto::BodyTopologyDto, String> {
    let label = format!("body_{}", body.0);
    wm.query_body_topology(body, label)
        .await
        .map_err(|e| format!("topology query failed: {e}"))
}
