//! T0 suppression gate against the REAL C++ OCCT worker, driven through the app's
//! [`DocumentRuntime`] like `checkpoints.rs` / `revolve_ops.rs`.
//!
//! `EditCommand::SetOperationSuppression` used to write `OperationRecord::suppressed`
//! and stop there: nothing mapped the flag onto `StepState::Suppressed`, the planner
//! filtered on that never-set state, and a "suppressed" op executed anyway. These
//! tests pin suppression as a **geometric** fact:
//!
//! * `suppress_cascades_to_downstream` — sketch → extrude → fillet on the extruded
//!   body; suppressing the extrude with `cascade: true` also suppresses the fillet
//!   (the dependency graph's body edge, which only exists because a committed regen
//!   writes the produced body back onto the record's `outputs`).
//! * `suppressed_op_is_skipped_by_regen` — two independent extrudes, one suppressed;
//!   regen publishes exactly ONE body whose volume is the survivor's alone. This is
//!   the test that catches the inertness (pre-fix it published both bodies).
//! * `unsuppress_restores_geometry` — toggling back restores the second body.
//! * `suppress_survives_undo_redo` — undo restores both bodies, redo re-suppresses.
//! * `suppress_invalidates_checkpoint` — a checkpoint minted before the suppression is
//!   NOT reused (its stored history-prefix hash no longer matches the suppressed-free
//!   prefix), the geometry is still right, and a checkpoint taken AFTER the
//!   suppression is usable again — the load-bearing half of the same-commit
//!   planner/hash coupling.
//!
//! The TRUST wave added three more, each pinning a way suppression used to lie:
//!
//! * `unsuppress_does_not_resurrect_deliberately_suppressed_downstream` (F3) — the
//!   cascade is **suppress-only**. Toggling an upstream op off and back on must not
//!   silently re-run a feature the user had deliberately suppressed.
//! * `incremental_regen_preserves_earlier_record_outputs` (F1) — a
//!   checkpoint-accelerated regen must not ERASE the `outputs` provenance of the
//!   records it did not execute; that wipe killed the cascade for the whole
//!   pre-checkpoint prefix and persisted through save/reopen.
//! * `suppressing_every_op_clears_the_geometry` (F2) — an op list that is empty
//!   *because everything is suppressed* must publish an EMPTY result, not report
//!   `NoOp` and leave the last bodies on screen (and in the saved container).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary (CI). A missing worker is a quiet local-dev skip.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
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
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors checkpoints.rs / revolve_ops.rs)
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

fn suppress(rt: &mut DocumentRuntime, rec: u128, suppressed: bool, cascade: bool) {
    rt.apply(EditCommand::SetOperationSuppression {
        record: rid(rec),
        suppressed,
        cascade,
    })
    .expect("SetOperationSuppression");
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

fn rid(rec: u128) -> RecordId {
    RecordId(Uuid::from_u128(rec))
}

/// The deterministic worker-minted body of a NewBody op (D1: `body_<opId>`).
fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

/// Whether the record with this id currently carries the suppression flag.
fn is_suppressed(rt: &DocumentRuntime, rec: u128) -> bool {
    rt.projection()
        .features
        .iter()
        .find(|f| f.id == rid(rec).to_string())
        .unwrap_or_else(|| panic!("record {rec:#x} in the projection"))
        .suppressed
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-07-31T00:00:00Z".into(),
        modified: "2026-07-31T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (a fully-constrained rectangle, as checkpoints.rs)
// ─────────────────────────────────────────────────────────────────────────────

const SK_A: u128 = 0xA00;
const EX_A: u128 = 0xA01;
const SK_B: u128 = 0xB00;
const EX_B: u128 = 0xB01;
const SK_C: u128 = 0xC00;
const EX_C: u128 = 0xC01;
const SK_D: u128 = 0xD00;
const EX_D: u128 = 0xD01;
const FILLET: u128 = 0xF01;

// Box A is 40 × 20 × 25 = 20000; box B is 30 × 15 × 25 = 11250.
const VOL_A: f64 = 40.0 * 20.0 * 25.0;
const VOL_B: f64 = 30.0 * 15.0 * 25.0;

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
    let pt = |sk: &mut Sketch, id, x, y| {
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
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point2: p0e,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point2: p1e,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        rid(rec),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ the worker's V1 first-region fallback
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
    };
    OperationRecord::new(
        rid(rec),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

/// A single-edge fillet carrying a typed [`ElementRef`] whose `primary.body` is the
/// operated body — the input the dependency graph turns into a body edge.
fn fillet_record(rec: u128, body: BodyId, edge_el: ElementId, anchor: Vec3) -> OperationRecord {
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: edge_el.clone(),
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
        rid(rec),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(2.0),
            edge_ids: vec![edge_el],
            edges: vec![edge_ref],
            chain_tangent_edges: false,
            extra: Default::default(),
        })),
    )
}

/// Appends one INDEPENDENT NewBody box (a sketch op then an extrude op) — two
/// timeline steps. Boxes are laid out along +X so they never interact.
fn build_box(rt: &mut DocumentRuntime, sk_rec: u128, ex_rec: u128, x0: f64, w: f64, h: f64) {
    let sid = SketchId(Uuid::from_u128(sk_rec));
    add_op(
        rt,
        sketch_record(sk_rec, &rect_sketch(sid, sk_rec << 8, x0, 0.0, w, h)),
    );
    add_op(rt, extrude_record(ex_rec, sid, 25.0));
}

/// Box A (40 × 20 × 25) at steps 0,1.
fn build_box_a(rt: &mut DocumentRuntime) {
    build_box(rt, SK_A, EX_A, 0.0, 40.0, 20.0);
}

/// Box B (30 × 15 × 25), far from A, at steps 2,3.
fn build_box_b(rt: &mut DocumentRuntime) {
    build_box(rt, SK_B, EX_B, 60.0, 30.0, 15.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers (from wire_contract.rs / topology_rebind.rs)
// ─────────────────────────────────────────────────────────────────────────────

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;

fn vertex(blob: &[u8], pbase: usize, i: usize) -> [f64; 3] {
    let o = pbase + i * 12;
    [
        f32_le(blob, o) as f64,
        f32_le(blob, o + 4) as f64,
        f32_le(blob, o + 8) as f64,
    ]
}

/// Signed volume via the divergence theorem — EXACT for a closed, planar-faced
/// polyhedron (box arithmetic to f32 precision).
fn mesh_volume(view: &MeshHeaderView, blob: &[u8]) -> f64 {
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let (pbase, ibase) = (pos.offset as usize, idx.offset as usize);
    let mut vol6 = 0.0f64;
    for t in 0..view.triangle_count as usize {
        let o = ibase + t * 12;
        let a = vertex(blob, pbase, u32_le(blob, o) as usize);
        let b = vertex(blob, pbase, u32_le(blob, o + 4) as usize);
        let c = vertex(blob, pbase, u32_le(blob, o + 8) as usize);
        vol6 += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (vol6 / 6.0).abs()
}

fn id_table(
    view: &MeshHeaderView,
    blob: &[u8],
    offs_ty: u32,
    chars_ty: u32,
    count: usize,
) -> Vec<String> {
    let offs = view.section(offs_ty).expect("id-offs");
    let chars = view.section(chars_ty).expect("id-chars");
    let (obase, cbase) = (offs.offset as usize, chars.offset as usize);
    (0..count)
        .map(|i| {
            let lo = u32_le(blob, obase + i * 4) as usize;
            let hi = u32_le(blob, obase + (i + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[cbase + lo..cbase + hi]).into_owned()
        })
        .collect()
}

/// Pick the edge with the greatest world-Z extent (a vertical box edge, always safely
/// fillettable) → `(topoKey, centroid-anchor)`.
fn vertical_edge_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
    assert!(view.has_edges(), "MESH1 must carry edges for a fillet pick");
    let er = view.section(SEC_EDGE_RANGES).expect("EDGE_RANGES");
    let ep = view.section(SEC_EDGE_POSITIONS).expect("EDGE_POSITIONS");
    let keys = id_table(
        view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let (erbase, epbase) = (er.offset as usize, ep.offset as usize);
    let mut best: Option<(usize, f64, Vec3)> = None;
    for i in 0..view.edge_count as usize {
        let first = u32_le(blob, erbase + i * 8) as usize;
        let count = u32_le(blob, erbase + i * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let (mut zmin, mut zmax) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut sx, mut sy, mut sz) = (0.0f64, 0.0f64, 0.0f64);
        for p in 0..count {
            let o = epbase + (first + p) * 12;
            let (x, y, z) = (
                f32_le(blob, o) as f64,
                f32_le(blob, o + 4) as f64,
                f32_le(blob, o + 8) as f64,
            );
            zmin = zmin.min(z);
            zmax = zmax.max(z);
            sx += x;
            sy += y;
            sz += z;
        }
        let span = zmax - zmin;
        let centroid = Vec3::new_unchecked(sx / count as f64, sy / count as f64, sz / count as f64);
        if best.is_none_or(|(_, s, _)| span > s) {
            best = Some((i, span, centroid));
        }
    }
    let (idx, _span, centroid) = best.expect("at least one edge");
    (keys[idx].clone(), centroid)
}

async fn body_volume(rt: &mut DocumentRuntime, body: BodyId) -> f64 {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh");
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    mesh_volume(&view, &mesh)
}

/// Promotes a vertical edge of `body` in `snap_id` to a stable [`ElementId`], returning
/// it with the world anchor a fillet ref carries.
async fn promote_vertical_edge(
    rt: &mut DocumentRuntime,
    body: BodyId,
    snap_id: SnapshotId,
) -> (ElementId, Vec3) {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("body mesh");
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    let (edge_key, edge_anchor) = vertical_edge_pick(&view, &mesh);
    let promoted = rt
        .promote_selection(
            snap_id,
            body,
            vec![(
                TopoKey::new(&edge_key),
                Some(AnchorIntent {
                    world_point: edge_anchor,
                    surface_uv: None,
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
            )],
        )
        .await
        .expect("promote edge");
    (ElementId::new(promoted[0].element_id.clone()), edge_anchor)
}

/// The published body ids of a regen, sorted for a stable comparison.
fn body_ids(report: &RegenReport, what: &str) -> Vec<BodyId> {
    let mut ids: Vec<BodyId> = published(report, what)
        .bodies
        .iter()
        .map(|b| b.body)
        .collect();
    ids.sort();
    ids
}

// ─────────────────────────────────────────────────────────────────────────────
// (1) Cascade: suppressing an extrude also suppresses the fillet on its body
// ─────────────────────────────────────────────────────────────────────────────

/// The graph's body edge exists only because a committed regen writes the produced
/// body back onto the extrude record's `outputs` — without that,
/// `DependencyGraph::downstream` is empty for every body-producing op and a
/// "cascading" suppression leaves the fillet standing on a body that no longer
/// regenerates (it would ERROR instead of going quiet).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppress_cascades_to_downstream() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // sketch → extrude → regen, then promote a vertical edge of the extruded box.
    build_box_a(&mut rt);
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "box A");
    let body = body_of(EX_A);
    let snap_id = SnapshotId(report.snapshot_id);
    assert_eq!(snap.bodies.len(), 1, "box A published");

    let (edge_el, edge_anchor) = promote_vertical_edge(&mut rt, body, snap_id).await;

    // → fillet on that body (step 2).
    add_op(&mut rt, fillet_record(FILLET, body, edge_el, edge_anchor));
    let filleted = regen_all(&mut rt).await;
    let _ = published(&filleted, "filleted box A");

    // Suppress the EXTRUDE with cascade → the fillet must follow.
    suppress(&mut rt, EX_A, true, true);
    assert!(is_suppressed(&rt, EX_A), "the extrude is suppressed");
    assert!(
        is_suppressed(&rt, FILLET),
        "the fillet standing on the extruded body cascaded"
    );
    assert!(
        !is_suppressed(&rt, SK_A),
        "the cascade is downstream-only — the profile sketch is untouched"
    );

    // …and the geometry follows: with both ops skipped no body remains.
    let after = regen_all(&mut rt).await;
    assert_eq!(
        published(&after, "cascade-suppressed").bodies.len(),
        0,
        "a cascade-suppressed extrude+fillet leaves no body"
    );

    // HONEST SUPPRESSION (TRUST F3): the cascade is a SUPPRESS-only policy. Un-suppressing
    // carries `cascade: false` (the frontend sends `cascade = suppressed`), so only the
    // op the user actually un-suppressed comes back — a downstream op stays suppressed
    // until the user says otherwise. Cascading the `false` here would silently resurrect
    // features the user had deliberately turned off.
    suppress(&mut rt, EX_A, false, false);
    assert!(!is_suppressed(&rt, EX_A));
    assert!(
        is_suppressed(&rt, FILLET),
        "un-suppress does NOT cascade — the fillet stays suppressed"
    );
    let restored = regen_all(&mut rt).await;
    assert_eq!(
        published(&restored, "cascade-restored").bodies.len(),
        1,
        "the box is back"
    );
    let vol = body_volume(&mut rt, body_of(EX_A)).await;
    assert!(
        (vol - VOL_A).abs() < 1.0,
        "and it is the UNFILLETED box ({VOL_A}) — the suppressed fillet did not re-run, got {vol}"
    );

    // Un-suppressing the fillet explicitly brings it back (suppression is not a
    // one-way door — it just is not undone behind the user's back).
    suppress(&mut rt, FILLET, false, false);
    let refilleted = regen_all(&mut rt).await;
    assert_eq!(published(&refilleted, "re-filleted").bodies.len(), 1);
    let vol = body_volume(&mut rt, body_of(EX_A)).await;
    assert!(
        vol < VOL_A - 1.0,
        "the fillet ran again (radius 2 shaves the box below {VOL_A}), got {vol}"
    );

    wm.shutdown().await;
    eprintln!("CASCADE PASS: suppress(extrude, cascade) → fillet suppressed, 0 bodies; un-suppress does NOT cascade");
}

// ─────────────────────────────────────────────────────────────────────────────
// (1b) TRUST F3: un-suppress must not resurrect a DELIBERATELY suppressed downstream
// ─────────────────────────────────────────────────────────────────────────────

/// The reason the cascade is suppress-only. The user turns the fillet off on purpose,
/// then toggles the upstream extrude off and back on. Under the old symmetric policy the
/// un-suppress cascaded `false` down the closure and the fillet — which the user never
/// asked to bring back — silently re-ran. Under honest suppression the fillet's own flag
/// is the only thing that governs it, and the geometry proves it: the restored body is
/// the sharp-edged box, not the filleted one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsuppress_does_not_resurrect_deliberately_suppressed_downstream() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    let report = regen_all(&mut rt).await;
    let body = body_of(EX_A);
    let snap_id = SnapshotId(report.snapshot_id);
    let (edge_el, edge_anchor) = promote_vertical_edge(&mut rt, body, snap_id).await;
    add_op(&mut rt, fillet_record(FILLET, body, edge_el, edge_anchor));
    let filleted = regen_all(&mut rt).await;
    assert_eq!(published(&filleted, "filleted").bodies.len(), 1);
    let filleted_vol = body_volume(&mut rt, body).await;
    assert!(
        filleted_vol < VOL_A - 1.0,
        "baseline: the fillet really removed material ({filleted_vol} < {VOL_A})"
    );

    // The user DELIBERATELY suppresses the fillet (no cascade — it has no dependents).
    suppress(&mut rt, FILLET, true, true);
    let sharp = regen_all(&mut rt).await;
    assert_eq!(published(&sharp, "fillet off").bodies.len(), 1);
    let vol = body_volume(&mut rt, body).await;
    assert!(
        (vol - VOL_A).abs() < 1.0,
        "the sharp box is back, got {vol}"
    );

    // Now the upstream extrude goes off (cascade) and back on (no cascade).
    suppress(&mut rt, EX_A, true, true);
    assert!(
        is_suppressed(&rt, FILLET),
        "still suppressed (it already was)"
    );
    suppress(&mut rt, EX_A, false, false);
    assert!(!is_suppressed(&rt, EX_A), "the extrude is back");
    assert!(
        is_suppressed(&rt, FILLET),
        "the deliberately-suppressed fillet STAYS suppressed across the upstream toggle"
    );

    let after = regen_all(&mut rt).await;
    assert_eq!(published(&after, "restored").bodies.len(), 1);
    let vol = body_volume(&mut rt, body).await;
    assert!(
        (vol - VOL_A).abs() < 1.0,
        "the restored body is the SHARP box ({VOL_A}) — the fillet was not resurrected, got {vol}"
    );

    wm.shutdown().await;
    eprintln!("HONEST-SUPPRESSION PASS: upstream off→on leaves the user-suppressed fillet off (volume {vol:.1})");
}

// ─────────────────────────────────────────────────────────────────────────────
// (1c) TRUST F1: an incremental (checkpoint-accelerated) regen must not ERASE the
//      `outputs` provenance of the records it did not execute
// ─────────────────────────────────────────────────────────────────────────────

/// `outputs` is the ONLY source of the dependency graph's body-producer edges, and the
/// suppression cascade rides on those edges. It used to be rewritten from the whole
/// regen mirror on every commit — but a checkpoint-accelerated regen reconstructs its
/// base from the immutable artifacts, and that registry has an EMPTY lifecycle log, so
/// the map only ever described the post-checkpoint suffix. Every earlier record had its
/// `outputs` CLEARED, the cascade for the whole prefix went silently dead, and because
/// `outputs` is serialized with the record the damage survived save/reopen.
///
/// Repro shape: regen → checkpoint at head (what `save_document` mints) → append a
/// feature → incremental regen. Then check the prefix's provenance is intact, both live
/// and after a save/reopen round trip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incremental_regen_preserves_earlier_record_outputs() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("outputs.onecad");
    let mut rt = runtime_over(&wm);

    // Steps 0,1 = box A; step 2 = a fillet on it. A from-0 regen writes the body
    // provenance onto BOTH records, which is what gives the graph its body edge.
    build_box_a(&mut rt);
    let report = regen_all(&mut rt).await;
    let body = body_of(EX_A);
    let snap_id = SnapshotId(report.snapshot_id);
    let (edge_el, edge_anchor) = promote_vertical_edge(&mut rt, body, snap_id).await;
    add_op(&mut rt, fillet_record(FILLET, body, edge_el, edge_anchor));
    let _ = published(&regen_all(&mut rt).await, "filleted box A");

    // The save-time checkpoint policy: mint a durable acceleration base at the head.
    rt.take_checkpoint_at_head().await;
    assert_eq!(rt.checkpoint_count(), 1, "a checkpoint at step 2 (head)");

    // Append ONE more feature (box B at steps 3,4) and regen incrementally — the plan
    // restores the checkpoint and executes ONLY steps 3,4.
    build_box_b(&mut rt);
    let incremental = rt
        .run_regen(RegenRequest::ToEnd { from: 3 }, CancelToken::new())
        .await;
    assert!(
        rt.last_regen_used_checkpoint(),
        "the appended feature must regen from the checkpoint (the repro's precondition)"
    );
    assert_eq!(
        body_ids(&incremental, "incremental"),
        {
            let mut v = vec![body_of(EX_A), body_of(EX_B)];
            v.sort();
            v
        },
        "the incremental regen still publishes both boxes"
    );

    rt.save(&path, save_meta()).expect("save");

    // THE ASSERTION: the pre-checkpoint records kept their `outputs`, so the graph
    // still has the extrude → fillet body edge and a cascade reaches it.
    suppress(&mut rt, EX_A, true, true);
    assert!(
        is_suppressed(&rt, FILLET),
        "the cascade must still reach the fillet — an incremental regen may NOT erase \
         the `outputs` of the records it did not execute"
    );
    let after = regen_all(&mut rt).await;
    assert_eq!(
        body_ids(&after, "cascade after incremental"),
        vec![body_of(EX_B)],
        "and the cascade is geometric: only box B survives"
    );

    // The durable half — `outputs` rides on the record through serde, so a wipe would
    // have reached disk. Re-open the container saved right after the incremental regen
    // and check the cascade there too.
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    suppress(&mut reopened, EX_A, true, true);
    assert!(
        is_suppressed(&reopened, FILLET),
        "the provenance survived save/reopen — the cascade works on the reopened document"
    );

    wm.shutdown().await;
    eprintln!("OUTPUTS PASS: checkpoint-accelerated regen preserved the prefix's `outputs` (live + reopened)");
}

// ─────────────────────────────────────────────────────────────────────────────
// (1d) TRUST F2: suppressing EVERY op publishes an EMPTY result, not a NoOp
// ─────────────────────────────────────────────────────────────────────────────

/// "No ops to execute" has two very different causes and the runtime used to conflate
/// them. A request that starts past the applied end really is a no-op. A from-0 request
/// whose op list is empty **because every op is suppressed** is not: the correct geometry
/// is NO geometry. Reporting `NoOp` left the projection unchanged, so the stale bodies
/// stayed on screen — and, since `save` writes the regen mirror, also went to disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppressing_every_op_clears_the_geometry() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    let baseline = regen_all(&mut rt).await;
    assert_eq!(published(&baseline, "baseline").bodies.len(), 1);
    assert_eq!(rt.projection().bodies.len(), 1, "the tree shows the box");

    // Suppress the WHOLE chain — the sketch op as well as the extrude.
    suppress(&mut rt, EX_A, true, true);
    suppress(&mut rt, SK_A, true, true);

    let cleared = regen_all(&mut rt).await;
    assert!(
        cleared.published(),
        "an all-suppressed from-0 regen must PUBLISH the empty result, got {:?}",
        cleared.outcome
    );
    assert_eq!(
        published(&cleared, "cleared").bodies.len(),
        0,
        "the published snapshot has no bodies"
    );
    // The viewport-facing delta must actually tell the frontend to drop the mesh.
    let change = cleared
        .document_change()
        .expect("a published clear emits a document-changed payload");
    assert!(
        change.changed_bodies.is_empty(),
        "nothing to (re)draw, got {:?}",
        change.changed_bodies
    );
    assert_eq!(
        change.removed_bodies,
        vec![body_of(EX_A).to_string()],
        "the previously-published body is reported REMOVED"
    );
    assert!(
        rt.projection().bodies.is_empty(),
        "and the tree is empty — no stale row survives a full suppression"
    );

    // Un-suppressing brings the geometry back unchanged.
    suppress(&mut rt, SK_A, false, false);
    suppress(&mut rt, EX_A, false, false);
    let back = regen_all(&mut rt).await;
    assert_eq!(published(&back, "restored").bodies.len(), 1);
    let vol = body_volume(&mut rt, body_of(EX_A)).await;
    assert!(
        (vol - VOL_A).abs() < 1.0,
        "box A returned at its own volume ({VOL_A}), got {vol}"
    );

    wm.shutdown().await;
    eprintln!("CLEAR PASS: all-suppressed → published 0 bodies + removedBodies; un-suppress → volume {vol:.1}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (2) THE inertness test: a suppressed op does not execute
// ─────────────────────────────────────────────────────────────────────────────

/// Two independent NewBody extrudes; extrude B is suppressed. Before the fix the
/// planner filtered on a `StepState::Suppressed` that nothing ever set, so B ran
/// anyway and the regen published TWO bodies.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppressed_op_is_skipped_by_regen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    build_box_b(&mut rt);
    let baseline = regen_all(&mut rt).await;
    assert_eq!(
        body_ids(&baseline, "baseline"),
        {
            let mut v = vec![body_of(EX_A), body_of(EX_B)];
            v.sort();
            v
        },
        "baseline publishes both boxes"
    );

    // Suppress extrude B (it has no dependents, so `cascade` is immaterial).
    suppress(&mut rt, EX_B, true, true);
    let after = regen_all(&mut rt).await;
    let snap = published(&after, "B suppressed");
    assert_eq!(
        snap.bodies.len(),
        1,
        "exactly ONE body survives, got {:?}",
        snap.bodies.iter().map(|b| b.body).collect::<Vec<_>>()
    );
    assert_eq!(
        snap.bodies[0].body,
        body_of(EX_A),
        "the survivor is box A, not the suppressed B"
    );

    let vol = body_volume(&mut rt, body_of(EX_A)).await;
    assert!(
        (vol - VOL_A).abs() < 1.0,
        "the survivor's volume is box A alone ({VOL_A}), got {vol}"
    );
    assert!(
        rt.get_mesh(body_of(EX_B), Lod::Coarse, None)
            .await
            .is_none(),
        "the suppressed op's body is gone from the published snapshot"
    );

    wm.shutdown().await;
    eprintln!(
        "SKIP PASS: suppressed extrude B never ran — 1 body, volume {vol:.1} == box A {VOL_A}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (3) Un-suppress restores the geometry
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsuppress_restores_geometry() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    build_box_b(&mut rt);
    let _ = regen_all(&mut rt).await;

    suppress(&mut rt, EX_B, true, true);
    let off = regen_all(&mut rt).await;
    assert_eq!(published(&off, "suppressed").bodies.len(), 1);

    suppress(&mut rt, EX_B, false, true);
    assert!(!is_suppressed(&rt, EX_B), "the record flag cleared");
    let on = regen_all(&mut rt).await;
    assert_eq!(
        published(&on, "unsuppressed").bodies.len(),
        2,
        "both boxes are back"
    );
    let vol_b = body_volume(&mut rt, body_of(EX_B)).await;
    assert!(
        (vol_b - VOL_B).abs() < 1.0,
        "box B regenerated at its own volume ({VOL_B}), got {vol_b}"
    );

    wm.shutdown().await;
    eprintln!("UNSUPPRESS PASS: 1 → 2 bodies, box B volume {vol_b:.1}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (4) Undo/redo: the suppression command is a normal undoable edit
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppress_survives_undo_redo() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    build_box_b(&mut rt);
    let _ = regen_all(&mut rt).await;

    suppress(&mut rt, EX_B, true, true);
    let suppressed = regen_all(&mut rt).await;
    assert_eq!(published(&suppressed, "suppressed").bodies.len(), 1);

    // Undo the suppression → both bodies (the undo memento restores the record,
    // and `from_records` re-derives the step state from it).
    assert!(rt.undo(), "the suppression is an undoable edit");
    assert!(!is_suppressed(&rt, EX_B), "undo cleared the record flag");
    let undone = regen_all(&mut rt).await;
    assert_eq!(
        published(&undone, "undone").bodies.len(),
        2,
        "undo brings the suppressed body back"
    );

    // Redo → back to one body.
    assert!(rt.redo().expect("redo ok"), "the suppression redoes");
    assert!(is_suppressed(&rt, EX_B), "redo re-set the record flag");
    let redone = regen_all(&mut rt).await;
    assert_eq!(
        published(&redone, "redone").bodies.len(),
        1,
        "redo re-suppresses the body"
    );

    wm.shutdown().await;
    eprintln!("UNDO/REDO PASS: suppress → undo (2 bodies) → redo (1 body)");
}

// ─────────────────────────────────────────────────────────────────────────────
// (5) Checkpoint invalidation — the load-bearing half of the same-commit coupling
// ─────────────────────────────────────────────────────────────────────────────

/// A checkpoint stores the `historyPrefixHash` of the prefix it captured. Because the
/// hash now OMITS suppressed records (matching what the planner actually executes),
/// suppressing an op inside a checkpointed prefix changes that prefix's hash and the
/// checkpoint is correctly rejected — a from-0 replay, still exactly right (Invariant
/// 7). A checkpoint taken AFTER the suppression matches again and IS reused, proving
/// the rejection is targeted staleness detection rather than a permanent break.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppress_invalidates_checkpoint() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin.clone()).await;
    let tmp = tempfile::tempdir().unwrap();
    let mut rt = runtime_over(&wm);

    // Box A (steps 0,1) + box B (steps 2,3) → regen → checkpoint at the head (step 3),
    // the save-time policy that mints one.
    build_box_a(&mut rt);
    build_box_b(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "boxes A+B");
    rt.take_checkpoint_at_head().await;
    assert_eq!(rt.checkpoint_count(), 1, "a checkpoint at step 3");
    rt.save(&tmp.path().join("ckpt.onecad"), save_meta())
        .expect("save");

    // CONTROL: append box C (steps 4,5) and regen from 4 — the dirty floor is past the
    // checkpoint, nothing is suppressed, so the planner SELECTS it.
    build_box(&mut rt, SK_C, EX_C, 120.0, 20.0, 20.0);
    let control = rt
        .run_regen(RegenRequest::ToEnd { from: 4 }, CancelToken::new())
        .await;
    assert!(
        rt.last_regen_used_checkpoint(),
        "control: an unsuppressed prefix keeps the checkpoint usable"
    );
    assert_eq!(published(&control, "control").bodies.len(), 3);

    // Now suppress extrude A — step 1, INSIDE the checkpointed prefix. The stored
    // hash was taken over a prefix that included it, so the checkpoint is stale.
    suppress(&mut rt, EX_A, true, true);
    let after = rt
        .run_regen(RegenRequest::ToEnd { from: 4 }, CancelToken::new())
        .await;
    assert!(
        !rt.last_regen_used_checkpoint(),
        "the pre-suppress checkpoint must NOT be reused — its stored prefix hash no \
         longer matches the suppressed-free hash of records[0..=3]"
    );
    let snap = published(&after, "post-suppress");
    let mut got: Vec<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    got.sort();
    let mut want = vec![body_of(EX_B), body_of(EX_C)];
    want.sort();
    assert_eq!(
        got, want,
        "the from-0 fallback is still exactly right: box A gone, B and C intact"
    );
    let vol = body_volume(&mut rt, body_of(EX_B)).await;
    assert!(
        (vol - VOL_B).abs() < 1.0,
        "box B reproduced exactly ({VOL_B}), got {vol}"
    );

    // A checkpoint minted AFTER the suppression describes the CURRENT (suppressed-free)
    // prefix, so the next incremental regen selects it again — targeted staleness
    // detection, not a permanent rejection.
    rt.take_checkpoint_at_head().await;
    assert_eq!(
        rt.checkpoint_count(),
        1,
        "only the fresh step-5 checkpoint: suppressing step 1 truncation-evicted the \
         step-3 one, which its own prefix hash had already made unusable"
    );
    build_box(&mut rt, SK_D, EX_D, 180.0, 20.0, 20.0);
    let reused = rt
        .run_regen(RegenRequest::ToEnd { from: 6 }, CancelToken::new())
        .await;
    assert!(
        rt.last_regen_used_checkpoint(),
        "the post-suppress checkpoint is usable again"
    );
    assert_eq!(
        published(&reused, "reused").bodies.len(),
        3,
        "restored base (B, C) + the new box D — box A stays suppressed"
    );

    wm.shutdown().await;
    eprintln!("CHECKPOINT PASS: control reuse → suppress rejects it (from-0 correct) → fresh checkpoint reused");
}
