//! W4 — does `export_3mf_file`'s Rust-side pipeline actually carry the
//! document's name + colours into a valid 3MF file?
//!
//! Mirrors `step_export_attributes.rs`'s harness (same box fixture, same
//! `pending_step_attributes`/`resolve_face_colors` pair the export command
//! composes) but drives `export_threemf::build_bodies` +
//! `onecad_core::io::threemf::write_3mf` directly — the app-crate pieces
//! `export_3mf_file` calls once past the runtime lock — rather than the Tauri
//! command wrapper itself (which needs an `AppState` + dialog harness this
//! test file does not have; `step_export_attributes.rs` makes the same call
//! for `api::export_step_file` vs `WorkerManager::export_step`).
//!
//! REQUIRE_WORKER-guarded (CI hard-fails without a worker; local dev skips
//! cleanly).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{AnchorIntent, SketchRegionRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::io::threemf::write_3mf;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::export::{pending_step_attributes, resolve_face_colors};
use onecad_lib::export_threemf::{build_bodies, raw_face_colors_by_body};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::validate_mesh_blob;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors step_export_attributes.rs / face_color_reopen.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
    );
    None
}

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    wm
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

const SKETCH_REC: u128 = 0xF00;
const EXTRUDE_REC: u128 = 0xF01;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op builders (verbatim from step_export_attributes.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, p0s, x0, y0);
    pt(&mut sk, p0e, x0 + w, y0);
    pt(&mut sk, p1s, x0 + w, y0);
    pt(&mut sk, p1e, x0 + w, y0 + h);
    pt(&mut sk, p2s, x0 + w, y0 + h);
    pt(&mut sk, p2e, x0, y0 + h);
    pt(&mut sk, p3s, x0, y0 + h);
    pt(&mut sk, p3e, x0, y0);
    sk.add_entity(SketchEntity::line(l0, p0s, p0e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l1, p1s, p1e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l2, p2s, p2e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l3, p3s, p3e, false))
        .unwrap();

    let coincident = |sk: &mut Sketch, id, a, b| {
        sk.add_constraint(Constraint::Coincident {
            id,
            point1: a,
            point2: b,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    };
    coincident(&mut sk, c(1), p0e, p1s);
    coincident(&mut sk, c(2), p1e, p2s);
    coincident(&mut sk, c(3), p2e, p3s);
    coincident(&mut sk, c(4), p3e, p0s);
    sk.add_constraint(Constraint::Horizontal { id: c(5), line: l0 })
        .unwrap();
    sk.add_constraint(Constraint::Horizontal { id: c(6), line: l2 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(7), line: l1 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(8), line: l3 })
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: p0s,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point1_position: CurvePosition::Arbitrary,
        point2: p0e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point1_position: CurvePosition::Arbitrary,
        point2: p1e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sk.id,
            plane: xy_plane_ref(),
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            host_face: None,
            extra: Default::default(),
        })),
    )
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""), // first-region fallback
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(dist),
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
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 readers — just enough to find the top face's TopoKey + centroid, the
// pick evidence `promote_selection` needs (mirrors face_color_reopen.rs).
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    use onecad_protocol::mesh::f32_le;
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

fn id_table(
    view: &onecad_protocol::mesh::MeshHeaderView,
    blob: &[u8],
    count: usize,
) -> Vec<String> {
    use onecad_protocol::mesh::u32_le;
    let offs = view.section(SEC_FACE_ID_OFFS).expect("id-offs");
    let chars = view.section(SEC_FACE_ID_CHARS).expect("id-chars");
    let (obase, cbase) = (offs.offset as usize, chars.offset as usize);
    (0..count)
        .map(|i| {
            let lo = u32_le(blob, obase + i * 4) as usize;
            let hi = u32_le(blob, obase + (i + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[cbase + lo..cbase + hi]).into_owned()
        })
        .collect()
}

/// The face id + world centroid of the highest face of `body`'s mesh — the top
/// cap a user would click to paint.
async fn top_face(rt: &mut DocumentRuntime, body: BodyId) -> (String, Vec3) {
    use onecad_protocol::mesh::u32_le;
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh");
    let view = validate_mesh_blob(&mesh).expect("body MESH1 validates");
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let ids = id_table(&view, &mesh, view.face_count as usize);

    let mut best: Option<(usize, f64)> = None;
    for f in 0..view.face_count as usize {
        let first = u32_le(&mesh, frbase + f * 8) as usize;
        let count = u32_le(&mesh, frbase + f * 8 + 4) as usize;
        let (mut sum, mut n) = (0.0f64, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                sum += vertex(&mesh, pbase, u32_le(&mesh, io + k * 4) as usize)[2];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let height = sum / n;
        if best.is_none_or(|(_, h)| height > h) {
            best = Some((f, height));
        }
    }
    let top = best.expect("at least one face").0;

    let first = u32_le(&mesh, frbase + top * 8) as usize;
    let count = u32_le(&mesh, frbase + top * 8 + 4) as usize;
    let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
    for t in first..first + count {
        let io = ibase + t * 12;
        for k in 0..3 {
            let v = vertex(&mesh, pbase, u32_le(&mesh, io + k * 4) as usize);
            for (acc, coord) in sum.iter_mut().zip(v) {
                *acc += coord;
            }
            n += 1.0;
        }
    }
    let centroid = Vec3::new(sum[0] / n, sum[1] / n, sum[2] / n).expect("finite centroid");
    (ids[top].clone(), centroid)
}

// ─────────────────────────────────────────────────────────────────────────────
// The probe
// ─────────────────────────────────────────────────────────────────────────────

const NAME: &str = "Bracket3MF";
const FACE_RED: [u8; 4] = [200, 30, 30, 255];
const BODY_GREY: [u8; 4] = [20, 40, 60, 255];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_authored_name_and_both_colours_survive_the_3mf_export() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xF10));
    let body = body_of(EXTRUDE_REC);

    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x3000, 0.0, 0.0, 20.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_REC, sid, 25.0));
    let report = regen_all(&mut rt).await;
    let snapshot = SnapshotId(published(&report, "stock box").id.0);

    let (top_key, centroid) = top_face(&mut rt, body).await;
    let anchor = Some(AnchorIntent {
        world_point: centroid,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    });
    let promoted = rt
        .promote_selection(snapshot, body, vec![(TopoKey::new(&top_key), anchor)])
        .await
        .expect("AcquireElementIds promotes the face pick");
    let element = onecad_core::ids::ElementId::new(&promoted[0].element_id);

    rt.apply(EditCommand::SetFaceColor {
        body,
        element_id: element,
        color: Some(FACE_RED),
    })
    .expect("SetFaceColor");
    rt.apply(EditCommand::SetBodyColor {
        body,
        color: Some(BODY_GREY),
    })
    .expect("SetBodyColor");
    rt.apply(EditCommand::RenameBody {
        body,
        name: NAME.into(),
    })
    .expect("RenameBody");

    // ── The exact pair `export_3mf_file` composes, same as `export_step_file`. ──
    let (mut attributes, pending) = pending_step_attributes(&rt);
    assert_eq!(pending.len(), 1, "exactly one authored face colour is owed");
    let raw_face_colors = raw_face_colors_by_body(&pending);
    let omitted = resolve_face_colors(&mut attributes, pending, Some(snapshot), &wm).await;
    assert_eq!(omitted, 0, "the just-minted element must resolve");

    let bodies = rt.head_body_ids();
    assert_eq!(bodies, vec![body]);
    let meshes = rt.meshes_arc();
    let threemf_bodies = build_bodies(
        &bodies,
        meshes.as_ref(),
        Some(snapshot),
        &attributes,
        &raw_face_colors,
    )
    .await
    .expect("build_bodies");
    assert_eq!(threemf_bodies.len(), 1);
    let tb = &threemf_bodies[0];
    assert_eq!(tb.name.as_deref(), Some(NAME));
    assert!(
        tb.palette.contains(&BODY_GREY) && tb.palette.contains(&FACE_RED),
        "both authored colours must reach the writer's palette: {:?}",
        tb.palette
    );
    assert!(
        tb.default_color_index.is_some(),
        "the whole-body colour becomes the object's default"
    );
    assert!(
        tb.triangle_colors.iter().any(|c| c.is_some()),
        "the painted face's triangles must carry an explicit colour index"
    );

    // The vertex count the writer emits must match the MESH1 blob's own count.
    let blob = meshes
        .fetch_mesh(body, Lod::Fine, snapshot)
        .await
        .expect("fetch_mesh");
    let mesh_view = validate_mesh_blob(&blob).expect("mesh validates");
    assert_eq!(tb.vertices.len() as u32, mesh_view.vertex_count);

    let bytes = write_3mf(&threemf_bodies).expect("write_3mf");

    // ── Unzip the produced file and assert on the actual XML. ───────────────
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("3mf is a valid zip");
    assert!(archive.by_name("[Content_Types].xml").is_ok());
    assert!(archive.by_name("_rels/.rels").is_ok());
    let mut model = String::new();
    {
        use std::io::Read;
        archive
            .by_name("3D/3dmodel.model")
            .expect("model part present")
            .read_to_string(&mut model)
            .expect("model part is valid utf8");
    }

    assert!(model.contains(NAME), "the body name must appear");
    assert!(
        model.contains("#14283CFF"),
        "the authored whole-body colour (BODY_GREY) must appear as a base entry: {model}"
    );
    assert!(
        model.contains("#C81E1EFF"),
        "the authored face colour (FACE_RED) must appear as a base entry: {model}"
    );
    assert!(
        model.contains(r#"pindex="0""#),
        "the object must declare a default colour (the whole-body fallback)"
    );
    let vertex_tags = model.matches("<vertex ").count();
    assert_eq!(
        vertex_tags as u32, mesh_view.vertex_count,
        "every MESH1 vertex must appear exactly once in the 3MF part"
    );

    wm.shutdown().await;
}
