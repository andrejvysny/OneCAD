//! DI-5 / W3 — does a STEP export actually CARRY the document's names and colours,
//! and does it still do so after a reopen?
//!
//! `face_color_reopen.rs` pins the half before this one: a user-authored face colour
//! survives save → reopen and re-binds through the DI-4 ladder. This file asks the
//! question that follows — the user now hits *Export STEP*. Nothing in the old
//! writer could express an appearance (`STEPControl_Writer` writes geometry and only
//! geometry), so both the body name and every face colour were dropped on the way
//! out, silently, with a perfectly valid STEP file to show for it.
//!
//! The measurement is a full round trip on VALUES, never "a file appeared":
//!
//!   author name + body colour + face colour
//!     → save → REOPEN in a genuinely fresh worker process → regen → DI-4 re-bind
//!     → `pending_step_attributes` + `resolve_face_colors` (the exact pair
//!       `api::export_step_file` composes, so this is production code under test)
//!     → `ExportStep`
//!     → re-IMPORT the produced file in a THIRD worker
//!     → the product name, the painted face's colour, and the body colour on every
//!       other face are all read back and compared.
//!
//! The painted face is identified GEOMETRICALLY on the way back in (highest
//! centroid == the top cap the pick was made on), because "one face is red" is a
//! much weaker claim than "the face the user painted is red": a colour on the wrong
//! face is the H5-B mis-bind this migration exists to eliminate, and it looks
//! entirely successful.
//!
//! The second test pins the refusal side: an authored colour whose element the
//! ladder honestly declines to re-bind must be OMITTED from the export, not mapped
//! onto a plausible ordinal.
//!
//! REQUIRE_WORKER-guarded (CI hard-fails without a worker; local dev skips cleanly).

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
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::export::{pending_step_attributes, resolve_face_colors, StepExportAttributes};
use onecad_lib::imports::prepare_import;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors face_color_reopen.rs)
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

fn seams(
    wm: &WorkerManager,
) -> (
    Arc<dyn GeometryEngine>,
    Arc<dyn MeshProvider>,
    Arc<dyn SolverEngine>,
) {
    (
        Arc::new(wm.clone()),
        Arc::new(wm.clone()),
        Arc::new(wm.clone()),
    )
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let (engine, meshes, solver) = seams(wm);
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

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "test".into(),
        occt_fingerprint: None,
        created: "2026-08-17T00:00:00Z".into(),
        modified: "2026-08-17T00:00:00Z".into(),
    }
}

const SKETCH_REC: u128 = 0xE00;
const EXTRUDE_REC: u128 = 0xE01;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op builders (rect_sketch verbatim from face_color_reopen.rs)
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

/// The XY frame lifted to `z` — a boss sketched ON the top of a 25 mm box.
fn plane_ref_at_z(z: f64) -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Custom,
        origin: Vec3::new_unchecked(0.0, 0.0, z),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// `sketch_record` on an explicit plane.
fn sketch_record_on(rec: u128, sk: &Sketch, plane: SketchPlaneRef) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sk.id,
            plane,
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            host_face: None,
            extra: Default::default(),
        })),
    )
}

/// `extrude_record` in `Add` mode against `target` (a boss joined onto a body).
fn extrude_add_record(rec: u128, sketch: SketchId, dist: f64, target: BodyId) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(dist),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::Add,
            target_body: Some(target),
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
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
// MESH1 readers (face_color_reopen.rs + the FACE_COLORS section)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
const SEC_FACE_COLORS: u32 = 12;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

fn id_table(view: &MeshHeaderView, blob: &[u8], count: usize) -> Vec<String> {
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

/// Mean vertex Z of every tessellated face, in face order.
fn face_heights(view: &MeshHeaderView, blob: &[u8]) -> Vec<f64> {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    (0..view.face_count as usize)
        .map(|f| {
            let first = u32_le(blob, frbase + f * 8) as usize;
            let count = u32_le(blob, frbase + f * 8 + 4) as usize;
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for t in first..first + count {
                let io = ibase + t * 12;
                for k in 0..3 {
                    sum += vertex(blob, pbase, u32_le(blob, io + k * 4) as usize)[2];
                    n += 1.0;
                }
            }
            if n == 0.0 {
                f64::NEG_INFINITY
            } else {
                sum / n
            }
        })
        .collect()
}

/// The FACE_COLORS section as `[r,g,b,a]` per face (mesh_format.md §4).
fn face_colors(view: &MeshHeaderView, blob: &[u8]) -> Vec<[u8; 4]> {
    let sec = view
        .section(SEC_FACE_COLORS)
        .expect("the re-imported body must carry a FACE_COLORS section");
    let base = sec.offset as usize;
    (0..view.face_count as usize)
        .map(|f| {
            let o = base + f * 4;
            [blob[o], blob[o + 1], blob[o + 2], blob[o + 3]]
        })
        .collect()
}

/// The face id + world centroid of the highest face of `body`'s mesh — the top cap
/// a user would click to paint.
async fn top_face(rt: &mut DocumentRuntime, body: BodyId) -> (String, Vec3) {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh");
    let view = validate_mesh_blob(&mesh).expect("body MESH1 validates");
    let ids = id_table(&view, &mesh, view.face_count as usize);
    let heights = face_heights(&view, &mesh);
    let top = heights
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .expect("at least one face");
    (ids[top].clone(), face_centroid(&view, &mesh, top))
}

fn face_centroid(view: &MeshHeaderView, blob: &[u8], face: usize) -> Vec3 {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let first = u32_le(blob, frbase + face * 8) as usize;
    let count = u32_le(blob, frbase + face * 8 + 4) as usize;
    let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
    for t in first..first + count {
        let io = ibase + t * 12;
        for k in 0..3 {
            let v = vertex(blob, pbase, u32_le(blob, io + k * 4) as usize);
            for (acc, coord) in sum.iter_mut().zip(v) {
                *acc += coord;
            }
            n += 1.0;
        }
    }
    assert!(n > 0.0, "the chosen face has triangles");
    Vec3::new(sum[0] / n, sum[1] / n, sum[2] / n).expect("finite centroid")
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared flow
// ─────────────────────────────────────────────────────────────────────────────

const NAME: &str = "Bracket";
const FACE_RED: [u8; 4] = [200, 30, 30, 255];
const BODY_GREY: [u8; 4] = [20, 40, 60, 255];

/// Author the fixture document (box + name + body colour + a face colour on the top
/// cap) and save it. `with_anchor` decides whether the pick carries the evidence the
/// DI-4 ladder needs to re-bind it after a reopen.
async fn author_and_save(
    wm: &WorkerManager,
    path: &std::path::Path,
    with_anchor: bool,
) -> ElementId {
    let mut rt = runtime_over(wm);
    let sid = SketchId(Uuid::from_u128(0xE10));
    let body = body_of(EXTRUDE_REC);

    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x2000, 0.0, 0.0, 20.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_REC, sid, 25.0));
    if !with_anchor {
        // The anchor-less case needs a GENUINE tie: a 20×10×10 boss on half of the
        // top gives the boss top and the uncovered base top identical descriptors
        // (planar, 200 mm², +Z, same sorted-edge hash). resolverVersion 4 tells the
        // pre-v4 top/bottom pair apart by `outward`, so that pair no longer ties.
        let boss_sid = SketchId(Uuid::from_u128(0xE11));
        add_op(
            &mut rt,
            sketch_record_on(
                0xE12,
                &rect_sketch(boss_sid, 0x3000, 0.0, 0.0, 20.0, 10.0),
                plane_ref_at_z(25.0),
            ),
        );
        add_op(&mut rt, extrude_add_record(0xE13, boss_sid, 10.0, body));
    }
    let report = regen_all(&mut rt).await;
    let snapshot = SnapshotId(published(&report, "stock box").id.0);

    let (top_key, centroid) = top_face(&mut rt, body).await;
    let anchor = with_anchor.then(|| AnchorIntent {
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
    let element = ElementId::new(&promoted[0].element_id);

    rt.apply(EditCommand::SetFaceColor {
        body,
        element_id: element.clone(),
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

    rt.save(path, save_meta()).expect("save");
    element
}

/// Re-import `step` in `wm` and return `(product names, per-face [colour, height])`
/// for the single body it recovers.
async fn reimport(
    wm: &WorkerManager,
    step: &std::path::Path,
) -> (Vec<String>, Vec<([u8; 4], f64)>) {
    let mut rt = runtime_over(wm);
    let prepared = prepare_import(wm, step).await.expect("prepare_import");
    assert_eq!(
        prepared.solid_count, 1,
        "the exported file must carry exactly the one body"
    );
    rt.add_import_record(&prepared, false)
        .expect("author the ImportStep record");
    let report = regen_all(&mut rt).await;
    published(&report, "re-import");

    let body = rt.head_body_ids()[0];
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("re-imported body mesh");
    let view = validate_mesh_blob(&mesh).expect("re-imported MESH1 validates");
    let colors = face_colors(&view, &mesh);
    let heights = face_heights(&view, &mesh);
    (
        prepared.product_names.clone(),
        colors.into_iter().zip(heights).collect(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// The probes
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_authored_name_and_face_colour_survive_reopen_and_step_export() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let doc = dir.path().join("Painted.onecad");

    let wm = spawn_worker(bin.clone()).await;
    let element = author_and_save(&wm, &doc, /*with_anchor=*/ true).await;
    wm.shutdown().await;

    // ── Reopen in a genuinely fresh worker process, then re-bind (DI-4). ──────
    let wm2 = spawn_worker(bin.clone()).await;
    let (engine, meshes, solver) = seams(&wm2);
    let mut reopened = DocumentRuntime::open(&doc, engine, meshes, solver).expect("reopen");
    let report = regen_all(&mut reopened).await;
    let snapshot = SnapshotId(published(&report, "reopened document").id.0);
    assert_eq!(
        reopened.rebind_persisted_elements().await,
        (1, 0),
        "the authored face's persisted id must re-bind before the export can address it"
    );

    // ── The exact pair `api::export_step_file` composes. ─────────────────────
    let (mut attributes, pending) = pending_step_attributes(&reopened);
    assert_eq!(
        attributes.body_names.values().collect::<Vec<_>>(),
        vec![NAME],
        "the tree name is what goes out as the STEP product name"
    );
    assert_eq!(
        attributes.body_colors.values().collect::<Vec<_>>(),
        vec![&BODY_GREY],
        "the authored whole-body colour is carried"
    );
    assert_eq!(pending.len(), 1, "exactly one authored face colour is owed");
    assert_eq!(
        (&pending[0].element, pending[0].body, pending[0].color),
        (&element, body_of(EXTRUDE_REC), FACE_RED),
        "…keyed by the id the pick minted, on the body it was picked from"
    );
    let omitted = resolve_face_colors(&mut attributes, pending, Some(snapshot), &wm2).await;
    assert_eq!(
        omitted, 0,
        "a re-bound element must resolve to a TopoKey — if it does not, the colour \
         would be silently dropped from every export after a reopen"
    );
    let resolved_keys: Vec<&String> = attributes
        .face_colors
        .values()
        .flat_map(|m| m.keys())
        .collect();
    assert_eq!(resolved_keys.len(), 1);
    assert!(
        resolved_keys[0].starts_with("f:"),
        "a face colour must be keyed by a FACE TopoKey, got {:?}",
        resolved_keys[0]
    );

    let step = dir.path().join("Painted.step");
    let bodies = reopened.head_body_ids();
    let written = WorkerManager::export_step(
        &wm2,
        &step.to_string_lossy(),
        &bodies,
        "AP242DIS",
        &attributes,
    )
    .await
    .expect("ExportStep");
    assert!(written > 0, "the exporter reports bytes written");
    let text = std::fs::read_to_string(&step).expect("the STEP file is readable");
    assert!(
        text.contains("AP242"),
        "the FILE_SCHEMA header must name AP242 — the schema the command now asks for"
    );
    assert!(
        text.contains(NAME),
        "the body name must appear in the STEP text at all (a plain STEPControl_Writer \
         export never contained it)"
    );
    // The face colour must be somewhere in the file too; the strong claim is asserted
    // by the re-import below, this only localises a failure to write-vs-read.
    assert!(
        text.contains("STYLED_ITEM") || text.contains("SURFACE_STYLE_USAGE"),
        "the STEP file must carry presentation styles (colours)"
    );
    drop(reopened);
    wm2.shutdown().await;

    // ── Read the produced file back with the real import lane. ───────────────
    let wm3 = spawn_worker(bin).await;
    let (names, faces) = reimport(&wm3, &step).await;
    assert_eq!(
        names,
        vec![NAME.to_string()],
        "the product name round-trips through STEP"
    );
    assert_eq!(faces.len(), 6, "the box comes back with six faces");

    let top = faces
        .iter()
        .enumerate()
        .max_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
        .map(|(i, _)| i)
        .expect("a highest face");
    assert_eq!(
        faces[top].0, FACE_RED,
        "the painted TOP face must come back with the AUTHORED colour — a colour on any \
         other face would be the silent mis-bind this stack refuses to make"
    );
    for (i, (color, _)) in faces.iter().enumerate() {
        if i == top {
            continue;
        }
        assert_eq!(
            *color, BODY_GREY,
            "face {i} must inherit the whole-body colour the document authored"
        );
    }
    wm3.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unresolvable_authored_face_colour_is_omitted_from_the_export() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let doc = dir.path().join("Ambiguous.onecad");

    // No anchor ⇒ the two caps of this box are a genuine tie (both planar, both
    // 400 mm², both score 1.0 with margin 0), so the DI-4 ladder REFUSES to re-bind
    // — the case `face_color_reopen.rs` pins. The export must inherit that refusal.
    let wm = spawn_worker(bin.clone()).await;
    let element = author_and_save(&wm, &doc, /*with_anchor=*/ false).await;
    wm.shutdown().await;

    let wm2 = spawn_worker(bin.clone()).await;
    let (engine, meshes, solver) = seams(&wm2);
    let mut reopened = DocumentRuntime::open(&doc, engine, meshes, solver).expect("reopen");
    let report = regen_all(&mut reopened).await;
    let snapshot = SnapshotId(published(&report, "reopened document").id.0);
    assert_eq!(
        reopened.rebind_persisted_elements().await,
        (0, 1),
        "an anchor-less pick is a real tie the ladder must refuse (the fixture's own \
         precondition)"
    );

    let (mut attributes, pending) = pending_step_attributes(&reopened);
    assert_eq!(pending.len(), 1, "the colour is still in the document");
    assert_eq!(
        pending[0].element, element,
        "…keyed by the id it was authored against"
    );
    let omitted = resolve_face_colors(&mut attributes, pending, Some(snapshot), &wm2).await;
    assert_eq!(
        (omitted, attributes.face_colors.len()),
        (1, 0),
        "an unresolvable element is OMITTED, never mapped onto whichever face holds a \
         plausible ordinal"
    );
    // The rest of the appearance is unaffected — a refusal costs one colour, not the export.
    assert_eq!(
        attributes.body_names.values().collect::<Vec<_>>(),
        vec![NAME]
    );
    assert_eq!(
        attributes.body_colors.values().collect::<Vec<_>>(),
        vec![&BODY_GREY]
    );

    let step = dir.path().join("Ambiguous.step");
    let bodies = reopened.head_body_ids();
    WorkerManager::export_step(
        &wm2,
        &step.to_string_lossy(),
        &bodies,
        "AP242DIS",
        &attributes,
    )
    .await
    .expect("ExportStep still succeeds");
    drop(reopened);
    wm2.shutdown().await;

    let wm3 = spawn_worker(bin).await;
    let (names, faces) = reimport(&wm3, &step).await;
    assert_eq!(names, vec![NAME.to_string()], "the name still travels");
    assert!(
        faces.iter().all(|(color, _)| *color == BODY_GREY),
        "no face may carry the authored colour when its element did not resolve; got {faces:?}"
    );
    wm3.shutdown().await;
}

/// A document with nothing authored produces the pre-DI-5 request unchanged, so the
/// new fields are genuinely additive rather than "always present, sometimes empty".
#[test]
fn an_attribute_less_export_emits_the_pre_di5_args() {
    let args = onecad_lib::worker::wire::export_step_args(
        "/tmp/x.step",
        &[BodyId(Uuid::from_u128(1))],
        "AP242DIS",
        &StepExportAttributes::default(),
    );
    let obj = args.as_object().expect("args is an object");
    assert_eq!(
        obj.keys().collect::<Vec<_>>(),
        vec!["bodyIds", "path", "schema"],
        "no empty bodyNames/bodyColors/faceColors may be emitted"
    );
}
