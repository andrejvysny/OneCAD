use super::*;
mod audit;
pub(super) use audit::{audit_saved_floats, authored_float_snapshot};
mod checkpoint;
pub(super) use checkpoint::replay_checkpoint;
mod clip;
pub(super) use clip::{
    clip_profile, clip_section, clip_unfillet_recipe, fillet_clip_edges, fillet_clip_floors,
    rear_clip_sections, rear_gap_sections, rear_upper_section,
};
mod contour;
mod flare;
mod inboard;
mod initial;
pub(super) use inboard::fillet_inboard_tool;
pub(super) use initial::initial_body;
mod operations;
mod rear_clip;
mod wall;
pub(super) use contour::{
    chamfer_rear_ramp, fillet_connector_lip, fillet_drafted_underside, fillet_lower_front,
    fillet_upper_outer,
};
pub(super) use flare::{connector_end_profile, connector_flare_profile};
pub(super) use operations::{
    add_sketch, cut_connector_end, extrusion, loft_clip_cut, loft_clip_section_cut, mesh_signature,
    ruled_ramp,
};
pub(super) use rear_clip::{inboard_clip_cut, rear_clip_cut, rear_gap_cut};
pub(super) use wall::cut_straight_wall;

fn id(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(0xf7c09000 + n))
}
fn point(v: &Value) -> Vec2 {
    Vec2::new_unchecked(-v[1].as_f64().unwrap(), -v[0].as_f64().unwrap())
}
pub(super) fn entities(wires: &Value) -> Vec<SketchEditOp> {
    let mut result = Vec::new();
    let mut n = 0u128;
    for wire in wires.as_array().unwrap() {
        for curve in wire.as_array().unwrap() {
            let kind = curve["kind"].as_str().unwrap();
            let first = id(n);
            n += 1;
            let coordinate = if kind == "line" {
                &curve["start"]
            } else {
                &curve["center"]
            };
            result.push(SketchEditOp::AddEntity {
                entity: SketchEntity::point(first, point(coordinate), false, false),
            });
            let second = id(n);
            n += 1;
            let entity = if kind == "line" {
                result.push(SketchEditOp::AddEntity {
                    entity: SketchEntity::point(second, point(&curve["end"]), false, false),
                });
                let line = id(n);
                n += 1;
                SketchEntity::line(line, first, second, false)
            } else if kind == "arc" {
                SketchEntity::arc(
                    second,
                    first,
                    curve["radius"].as_f64().unwrap(),
                    -std::f64::consts::FRAC_PI_2 - curve["end"].as_f64().unwrap(),
                    -std::f64::consts::FRAC_PI_2 - curve["start"].as_f64().unwrap(),
                    false,
                )
                .unwrap()
            } else {
                assert_eq!(kind, "circle");
                SketchEntity::circle(second, first, curve["radius"].as_f64().unwrap(), false)
                    .unwrap()
            };
            result.push(SketchEditOp::AddEntity { entity });
        }
    }
    result
}

fn inside_triangle(point: [f64; 2], p: [[f64; 2]; 3]) -> bool {
    let cross = |a: [f64; 2], b: [f64; 2]| {
        (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])
    };
    let signs = [cross(p[0], p[1]), cross(p[1], p[2]), cross(p[2], p[0])];
    signs.iter().all(|v| *v >= 0.0) || signs.iter().all(|v| *v <= 0.0)
}

async fn promote_cap(
    h: &Harness,
    body: BodyId,
    at: onecad_core::math::Vec3,
) -> onecad_core::ids::ElementId {
    use onecad_core::ids::{ElementId, TopoKey};
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
    let mut selected = None;
    for face in 0..mesh.header().face_count as usize {
        let span = mesh.face_range(face).unwrap();
        for t in span.first..span.first + span.count {
            let verts = mesh
                .triangle(t as usize)
                .unwrap()
                .map(|v| mesh.position(v as usize).unwrap());
            if verts.iter().all(|p| {
                (f64::from(p[2]) - at.z).abs() <= f64::from(f32::EPSILON) * at.z.abs().max(1.0)
            }) && inside_triangle(
                [at.x, at.y],
                verts.map(|p| [f64::from(p[0]), f64::from(p[1])]),
            ) {
                selected = Some(mesh.face_id(face).unwrap().to_string());
                break;
            }
        }
        if selected.is_some() {
            break;
        }
    }
    let key = selected.expect("actual horizontal face under requested point");
    let key = TopoKey::new(&key);
    if !key.is_valid() {
        return ElementId::new(key.as_str());
    }
    let mut guard = h.state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    let promoted = runtime
        .promote_selection(runtime.head_snapshot_id().unwrap(), body, vec![(key, None)])
        .await
        .unwrap();
    ElementId::new(&promoted[0].element_id)
}

pub(super) async fn drill_boss(h: &Harness, body: BodyId, source_x: f64, n: u128) {
    use onecad_core::{
        document::{
            record::{HoleParams, HoleType},
            refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef},
            variables::Scalar,
        },
        math::Vec3,
    };
    let at = Vec3::new_unchecked(source_x, 10.5, 3.1);
    let face = promote_cap(h, body, at).await;
    let params = HoleParams {
        target_body: body,
        face: ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element: face,
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
        point: at,
        points: None,
        hole_type: HoleType::Countersink,
        diameter: Scalar::new(3.3),
        depth: Some(Scalar::new(2.5)),
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: Some(Scalar::new(3.7)),
        cs_angle_deg: Some(Scalar::new(90.0)),
        thread: None,
        result_policy_version: Some(2),
        extra: Default::default(),
    };
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                record(n),
                1,
                "Framework boss hole",
                Operation::Known(KnownOperation::Hole(params)),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&result.projection).await;
}

pub(super) async fn connector_profile(h: &Harness) -> onecad_app::dto::FinishSketchDto {
    let sid = sketch(7);
    let datum = DatumPlaneId(Uuid::from_u128(0xfac0207));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "Connector exterior", "YZ", 32.0),
        },
    )
    .await
    .unwrap();
    let mut s = Sketch::on_world_plane(sid, "Framework connector core", WorldPlane::YZ);
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
    for (i, y) in [2.2, 4.8].into_iter().enumerate() {
        let p = EntityId(Uuid::from_u128(0xfac0500 + 3 * i as u128));
        let q = EntityId(Uuid::from_u128(0xfac0501 + 3 * i as u128));
        let line = EntityId(Uuid::from_u128(0xfac0502 + 3 * i as u128));
        for (id, x) in [(p, -2.925), (q, 2.925)] {
            ops.push(SketchEditOp::AddEntity {
                entity: SketchEntity::point(id, Vec2::new_unchecked(x, y), false, false),
            });
        }
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::line(line, p, q, false),
        });
    }
    for (i, (x, a, b)) in [
        (
            -2.925,
            std::f64::consts::FRAC_PI_2,
            3.0 * std::f64::consts::FRAC_PI_2,
        ),
        (
            2.925,
            -std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let center = EntityId(Uuid::from_u128(0xfac0510 + 2 * i as u128));
        let arc = EntityId(Uuid::from_u128(0xfac0511 + 2 * i as u128));
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::point(center, Vec2::new_unchecked(x, 3.5), false, false),
        });
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::arc(arc, center, 1.3, a, b, false).unwrap(),
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

pub(super) async fn slot_profile(
    h: &Harness,
    n: u128,
    offset: f64,
    plane: WorldPlane,
    points: &[[f64; 2]],
) -> onecad_app::dto::FinishSketchDto {
    let sid = sketch(n);
    let datum = DatumPlaneId(Uuid::from_u128(0xfac0200 + n));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(
                datum,
                "Cut plane",
                match plane {
                    WorldPlane::XZ => "XZ",
                    WorldPlane::YZ => "YZ",
                    WorldPlane::XY => "XY",
                },
                offset,
            ),
        },
    )
    .await
    .unwrap();
    let mut s = Sketch::on_world_plane(sid, "Framework cut profile", plane);
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
    for i in 0..points.len() {
        let a = EntityId(Uuid::from_u128(0xfac0600 + 3 * i as u128));
        let b = EntityId(Uuid::from_u128(0xfac0601 + 3 * i as u128));
        let line = EntityId(Uuid::from_u128(0xfac0602 + 3 * i as u128));
        for (id, at) in [(a, points[i]), (b, points[(i + 1) % points.len()])] {
            ops.push(SketchEditOp::AddEntity {
                entity: SketchEntity::point(id, Vec2::new_unchecked(at[0], at[1]), false, false),
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

pub(super) fn verify_source() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let provenance: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/reference-histories/framework-enclosure-provenance.json"
    ))
    .unwrap();
    for (path, key) in [
        (
            "fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp",
            "sourceSha256",
        ),
        (
            "fixtures/reference-histories/extract_framework_profiles.py",
            "profileExtractorSha256",
        ),
        (
            "fixtures/reference-histories/framework-enclosure-profiles.json",
            "profilesSha256",
        ),
        (
            "fixtures/reference-histories/compare_framework_intermediate.py",
            "comparisonScriptSha256",
        ),
        (
            "fixtures/reference-histories/inspect_framework_blends.py",
            "blendInspectorSha256",
        ),
        (
            "fixtures/reference-histories/framework-blend-adjacency.json",
            "blendAdjacencySha256",
        ),
        (
            "fixtures/reference-histories/inspect_framework_surfaces.py",
            "surfaceInspectorSha256",
        ),
        (
            "fixtures/reference-histories/framework-surface-fits.json",
            "surfaceFitsSha256",
        ),
    ] {
        let digest = Sha256::digest(std::fs::read(root.join(path)).unwrap());
        assert_eq!(format!("{digest:x}"), provenance[key].as_str().unwrap());
    }
}
