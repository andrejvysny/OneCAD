//! Data-integrity probe: does a user-authored FACE COLOR survive save → reopen?
//!
//! A face color is authored from the Inspector's face section
//! (`src/features/inspector/sections.tsx` → `treeActions::setFaceColor` →
//! `EditCommand::SetFaceColor`) and is stored in the body registry keyed by the
//! Rust-minted `ElementId` of the face (`document/body.rs` `face_colors`). The
//! frontend paints it through `meshSync.resolveAuthoredFaceColors`, which has two
//! paths and NO third:
//!
//! 1. the mesh's id table already carries ElementIds (`IDS_HAVE_ELEMENTIDS`), so the
//!    stored key maps straight onto a face; or
//! 2. it asks `elementInfo(bodyId, elementId)` for that element's current TopoKey.
//!
//! Both bottom out in the worker's element-map partition, which is minted ON DEMAND
//! and lives in the worker process. `BindElementIds` has exactly one production call
//! site (`document_runtime.rs`, inside `promote_selection`), and nothing re-binds a
//! PERSISTED ElementId when a document is reopened. So the question this file
//! answers with evidence rather than inspection: after a reopen, can either path
//! still find the coloured face?
//!
//! The document data itself is not in doubt — `face_colors` is serialized in the
//! body registry and comes back — so this is a rendering-fidelity probe, not a
//! data-destruction one. It is written as a CHARACTERIZATION test: it records what
//! the stack actually does today, with the in-session control right beside the
//! post-reopen measurement so neither number can be blamed on the fixture.
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
use onecad_core::document::refs::SketchRegionRef;
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
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors offset_face.rs)
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
    let (engine, meshes, solver) = seams(wm);
    DocumentRuntime::new_blank(engine, meshes, solver)
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
        created: "2026-08-14T00:00:00Z".into(),
        modified: "2026-08-14T00:00:00Z".into(),
    }
}

const SKETCH_REC: u128 = 0xC00;
const EXTRUDE_REC: u128 = 0xC01;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op builders (rect_sketch verbatim from offset_face.rs / breadth_ops.rs)
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
// MESH1 face picks — the TopoKey a viewport click would produce
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;

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

/// `(id-table entry, centroid z)` per tessellated face, in face order.
fn face_ids_and_heights(view: &MeshHeaderView, blob: &[u8]) -> Vec<(String, f64)> {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let ids = id_table(view, blob, view.face_count as usize);
    let mut out = Vec::with_capacity(ids.len());
    for (f, id) in ids.iter().enumerate() {
        let first = u32_le(blob, frbase + f * 8) as usize;
        let count = u32_le(blob, frbase + f * 8 + 4) as usize;
        let (mut sz, mut n) = (0.0f64, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                sz += vertex(blob, pbase, u32_le(blob, io + k * 4) as usize)[2];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        out.push((id.clone(), sz / n));
    }
    out
}

/// The extrude's far cap (greatest average world Z) — the face this probe colours.
async fn top_face_id(rt: &mut DocumentRuntime, body: BodyId) -> (String, bool) {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh");
    let view = validate_mesh_blob(&mesh).expect("body MESH1 validates");
    let ids = face_ids_and_heights(&view, &mesh);
    let top = ids
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("at least one face");
    (top.0, view.ids_have_element_ids())
}

// ─────────────────────────────────────────────────────────────────────────────
// The probe
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_authored_face_colour_reopens_as_data_but_its_element_no_longer_resolves() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0xC10));
    let body = body_of(EXTRUDE_REC);

    add_op(
        &mut rt,
        sketch_record(SKETCH_REC, &rect_sketch(sid, 0x1000, 0.0, 0.0, 20.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_REC, sid, 25.0));
    let report = regen_all(&mut rt).await;
    let snapshot = SnapshotId(published(&report, "stock box").id.0);

    // ── The user picks the top face and colours it. ───────────────────────────
    let (top_key, ids_were_element_ids) = top_face_id(&mut rt, body).await;
    assert!(
        !ids_were_element_ids,
        "before any promotion the mesh id table carries TopoKeys, not ElementIds \
         (this is the fixture's own precondition)"
    );
    let promoted = rt
        .promote_selection(snapshot, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("AcquireElementIds promotes the face pick");
    let element = ElementId::new(&promoted[0].element_id);
    assert!(
        element.as_str().starts_with("el_"),
        "ElementIds are Rust-minted `el_…` (Invariant 2), got {element}"
    );
    let colour = [200u8, 30, 30, 255];
    rt.apply(EditCommand::SetFaceColor {
        body,
        element_id: element.clone(),
        color: Some(colour),
    })
    .expect("SetFaceColor");

    // ── CONTROL: in-session, the id resolves, so the colour can be painted. ───
    let live = ElementQuery::query_element(&wm, snapshot, body, element.as_str())
        .await
        .expect("QueryElement must not error for a just-promoted id");
    assert!(
        live.is_some(),
        "control: a promoted ElementId resolves while the session that minted it is \
         alive — if this is None the probe below proves nothing"
    );
    assert_eq!(
        rt.projection().bodies[&body.to_string()]
            .face_colors
            .get(element.as_str()),
        Some(&colour),
        "control: the authored colour is in the projection the viewport paints from"
    );

    // ── Save, then reopen in a genuinely FRESH worker process. ───────────────
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Coloured.onecad");
    rt.save(&path, save_meta()).expect("save");
    drop(rt);
    wm.shutdown().await;

    let bin2 = real_worker().expect("worker binary");
    let wm2 = spawn_worker(bin2).await;
    let (engine, meshes, solver) = seams(&wm2);
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    let report2 = regen_all(&mut reopened).await;
    let snapshot2 = SnapshotId(published(&report2, "reopened document").id.0);

    // 1. The DATA survives. This is the half that is not in doubt, asserted so a
    //    future regression cannot quietly turn a rendering gap into real loss.
    assert_eq!(
        reopened.projection().bodies[&body.to_string()]
            .face_colors
            .get(element.as_str()),
        Some(&colour),
        "the authored face colour is still in the reopened document, keyed by the \
         same ElementId"
    );

    // 2. Neither painting path can find the face that colour belongs to.
    let (_key_after, ids_are_element_ids) = top_face_id(&mut reopened, body).await;
    let resolved = ElementQuery::query_element(&wm2, snapshot2, body, element.as_str())
        .await
        .expect("QueryElement must not error for an unknown id — it answers None");

    assert!(
        !ids_are_element_ids && resolved.is_none(),
        "CHARACTERIZATION: after a reopen the worker's element-map partition is empty \
         for this element (BindElementIds has one production call site, inside \
         promote_selection, and nothing re-binds a persisted id at open), so BOTH \
         frontend paths in meshSync.resolveAuthoredFaceColors come up empty — the \
         mesh id table carries TopoKeys (idsHaveElementIds={ids_are_element_ids}) and \
         elementInfo answers {resolved:?}. If this assertion fails the seam has been \
         closed: delete the probe's failure expectation and pin the fix instead."
    );

    wm2.shutdown().await;
}
