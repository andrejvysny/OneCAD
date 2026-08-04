//! M4a — the topology slice **gate**: parametric edit → history rebind via the
//! resolution ladder → the fillet SURVIVES, or a deterministic `NeedsRepair` (never
//! a silently-wrong bind). Driven through the app's single-writer [`DocumentRuntime`]
//! against the REAL C++ OCCT worker, exactly like `m2_gate.rs` / `wire_contract.rs`.
//!
//! This is the H5-B fix the corpus (`corpus/cases/e_naming_break_fillet_upstream_edit.json`)
//! documents as the anti-goal of the legacy app: an upstream edit orphaned every
//! downstream topological reference. The new stack must rebind through OCCT history +
//! descriptor/anchor matching (SCHEMA §10) or surface `NeedsRepair` STATE (SCHEMA §9).
//!
//! Coverage:
//! * `multi_region_extrude_binds_by_region_id` — M2-flag close: an explicit
//!   `regionId` selects a specific closed region (multi-region profile binding);
//!   omitting it keeps the first-region fallback (corpus case i shape).
//! * `h5b_fillet_survives_small_edit` — a SMALL parametric edit (extrude depth): the
//!   fillet re-applies, its bound `ElementId` is STABLE, no `NeedsRepair` (case e
//!   golden path).
//! * `h5b_destructive_edit_is_deterministic_needs_repair` — a DESTRUCTIVE edit
//!   (sketch replaced far away): deterministic `NeedsRepair`, the fillet does NOT
//!   silently apply to a wrong edge, and the payload is IDENTICAL on replay.
//! * `symmetric_ambiguity_resolves_to_needs_repair` — corpus case f: a symmetric
//!   descriptor tie ⇒ `NeedsRepair`, never a guess (real-worker `ResolveRefs`).
//! * `fillet_reedit_swaps_to_chamfer_and_regens` — FILLET-CHAMFER-UNIFY W3: the
//!   ONE sanctioned `opType` edit reaches the kernel (removed-volume ratio, not
//!   the record's own tag), rebinds the same edge, and undoes.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary skips.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ChamferParams, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation,
    Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::history::StepState;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, ResolveOutcome,
    ResolveRef, ResolveRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors m2_gate.rs / wire_contract.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the worker binary, honoring the CI / misconfiguration guards (MINOR-2 —
/// a missing binary must NOT silently read as a green skip):
/// * `ONECAD_WORKER_PATH` set but pointing at a **missing** file ⇒ PANIC;
/// * `ONECAD_REQUIRE_WORKER=1` and no worker resolves at all ⇒ PANIC (CI sets this,
///   so a regressed worker build step hard-fails);
/// * otherwise a missing worker is a quiet local-dev skip (`None`).
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

/// The EDIT LANE regen (`SchedulerHandle::handle` maps a `RegenHint::ToEnd` to
/// `ToEnd { from = dirty.from }`, and `dirty.from` is the index of the edited
/// record). `from > 0` is what makes the plan carry SCHEMA §7.2 `editedFrom`, so
/// this — not `regen_all` — is the lane the descriptor-tie veto sees.
async fn regen_from(rt: &mut DocumentRuntime, from: usize) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await
}

fn open_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen saved container")
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "topology-rebind".into(),
        occt_fingerprint: None,
        created: "2026-08-04T00:00:00Z".into(),
        modified: "2026-08-04T00:00:00Z".into(),
    }
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixed record ids
// ─────────────────────────────────────────────────────────────────────────────

const SKETCH_REC: u128 = 0x5c00;
const EXTRUDE_REC: u128 = 0xe000;
const FILLET_REC: u128 = 0xf000;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders (marshaller shape: 8 points + 4 lines per rectangle)
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

/// Adds one fully-constrained (dof-0) rectangle at `(x0,y0)` size `w×h` into `sk`,
/// seeded from `base` (unique entity/constraint ids). The marshaller shape: 8
/// synthesized points, 4 lines, coincident corners, H/V, a Fixed anchor + H/V dims.
fn add_rect(sk: &mut Sketch, base: u128, x0: f64, y0: f64, w: f64, h: f64) {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let pt = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(sk, p0s, x0, y0);
    pt(sk, p0e, x0 + w, y0);
    pt(sk, p1s, x0 + w, y0);
    pt(sk, p1e, x0 + w, y0 + h);
    pt(sk, p2s, x0 + w, y0 + h);
    pt(sk, p2e, x0, y0 + h);
    pt(sk, p3s, x0, y0 + h);
    pt(sk, p3e, x0, y0);
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
    coincident(sk, c(1), p0e, p1s);
    coincident(sk, c(2), p1e, p2s);
    coincident(sk, c(3), p2e, p3s);
    coincident(sk, c(4), p3e, p0s);
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
}

fn single_rect(sid: SketchId, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    rect_at(sid, 0x1000, x0, y0, w, h)
}

/// A single-rectangle sketch seeded from `base` — distinct `base` ⇒ distinct entity
/// UUIDs ⇒ a distinct normative region id (region ids hash entity ids, not
/// positions), so replacing a sketch op's `base` makes a prior region id STALE.
fn rect_at(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    add_rect(&mut sk, base, x0, y0, w, h);
    sk
}

fn sketch_record(sk: &Sketch) -> OperationRecord {
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
        RecordId(Uuid::from_u128(SKETCH_REC)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_op(sketch: SketchId, region: &str, dist: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(region),
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
    }))
}

fn extrude_record(sketch: SketchId, region: &str, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(EXTRUDE_REC)),
        1,
        "Extrude",
        extrude_op(sketch, region, dist),
    )
}

/// A single-edge fillet, the edge carried as a typed [`ElementRef`] (Rust-minted
/// `ElementId` in `primary.element` + a world-midpoint anchor) — the SCHEMA §7.3
/// `inputs[]` shape the worker's ladder binds.
fn fillet_record(
    body: BodyId,
    edge_el: ElementId,
    edge_anchor: Vec3,
    radius: f64,
) -> OperationRecord {
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: edge_el.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: edge_anchor,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(FILLET_REC)),
        2,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids: vec![edge_el],
            edges: vec![edge_ref],
            chain_tangent_edges: false,
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 geometry helpers
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

/// Signed volume of a MESH1 body via the divergence theorem — EXACT for a closed,
/// planar-faced polyhedron (box arithmetic to f32 precision).
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
    assert!(
        view.has_edges(),
        "MESH1 must carry edges for the fillet pick"
    );
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

fn bbox_center(view: &MeshHeaderView) -> Vec3 {
    Vec3::new_unchecked(
        f64::from(view.bbox_min[0] + view.bbox_max[0]) / 2.0,
        f64::from(view.bbox_min[1] + view.bbox_max[1]) / 2.0,
        f64::from(view.bbox_min[2] + view.bbox_max[2]) / 2.0,
    )
}

/// The `StepState::Error { reason }` message for the extrude step (index 1) of a
/// published snapshot, if it failed (else `None`). The failed op's §8 message rides
/// on `perStepResults.message` → the snapshot's step Error reason.
fn extrude_error_reason(report: &RegenReport) -> Option<String> {
    let Outcome::Published(snap) = &report.outcome else {
        return None;
    };
    snap.step_states.iter().find_map(|(idx, st)| match st {
        StepState::Error { reason } if *idx == 1 => Some(reason.clone()),
        _ => None,
    })
}

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch mesh")
}

// ─────────────────────────────────────────────────────────────────────────────
// Deliverable 1 — multi-region extrude profile binding (M2 flag close, corpus i)
// ─────────────────────────────────────────────────────────────────────────────

/// One sketch, TWO disjoint rectangles of DIFFERENT areas (A: 40×20 = 800; B:
/// 20×10 = 200). An explicit `regionId` selects a specific region; omitting it keeps
/// the first-region fallback. Proving `regionId` is HONORED: extruding each region by
/// its id yields DISTINCT footprints (if `regionId` were ignored, both would be the
/// first region's footprint — a single value).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_region_extrude_binds_by_region_id() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    const DIST: f64 = 5.0;
    const AREA_A: f64 = 40.0 * 20.0; // 800
    const AREA_B: f64 = 20.0 * 10.0; // 200

    let sid = SketchId(Uuid::from_u128(0x2A));
    let mut sketch = Sketch::on_world_plane(sid, "TwoRects", WorldPlane::XY);
    add_rect(&mut sketch, 0x1000, 0.0, 0.0, 40.0, 20.0); // region A
    add_rect(&mut sketch, 0x3000, 100.0, 100.0, 20.0, 10.0); // region B (disjoint)

    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "TwoRects", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let solved = rt
        .sketch_upsert(sid, edit_ops(&sketch))
        .await
        .expect("sketch_upsert");
    assert_eq!(solved.dof, 0, "two fully-constrained rectangles ⇒ dof 0");
    let finished = rt.finish_sketch(sid).await.expect("finish_sketch");
    assert_eq!(
        finished.regions.len(),
        2,
        "corpus i: two disjoint rectangles ⇒ two closed regions"
    );
    let region_ids: Vec<String> = finished
        .regions
        .iter()
        .map(|r| r.region_id.clone())
        .collect();
    assert!(
        region_ids.iter().all(|id| id.starts_with("r_")),
        "normative FNV region ids (SCHEMA §7.4), got {region_ids:?}"
    );
    eprintln!("multi-region: regionIds = {region_ids:?}");

    // finish_sketch minted the sketch's timeline record (production shape) — a
    // manual add here would create a DUPLICATE Sketch step for the same sketch,
    // something the app can never produce.

    // Extrude each region BY ID → collect the footprint volume.
    let mut vols_by_region: Vec<f64> = Vec::new();
    for region in &region_ids {
        add_op(&mut rt, extrude_record(sid, region, DIST));
        let report = regen_all(&mut rt).await;
        let _ = published(&report, "multi-region extrude");
        let body = report.changed[0].0;
        let mesh = body_mesh(&mut rt, body).await;
        let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
        let vol = mesh_volume(&view, &mesh);
        eprintln!("  region {region} → volume {vol:.1}");
        vols_by_region.push(vol);
        assert!(rt.undo(), "undo the extrude op for the next region");
    }

    // Each regionId selected a DISTINCT footprint (800·d and 200·d). Had regionId
    // been ignored (always first region), both would be equal.
    let mut sorted = vols_by_region.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        (sorted[0] - AREA_B * DIST).abs() < 1.0 && (sorted[1] - AREA_A * DIST).abs() < 1.0,
        "regionId honored: the two regions extrude to {{200·d=1000, 800·d=4000}}, got {vols_by_region:?}"
    );
    assert!(
        (sorted[1] - sorted[0]).abs() > 1.0,
        "the two regions have DISTINCT footprints (regionId is not ignored)"
    );

    // A NON-EMPTY regionId that matches NO detected region is a HARD FAILURE (M4a
    // strict rule) — NEVER a silent fallback to a different region.
    let bogus = "r_deadbeefdeadbeef";
    add_op(&mut rt, extrude_record(sid, bogus, DIST));
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "no-match extrude").clone();
    assert!(
        rep.changed.is_empty() && snap.bodies.is_empty(),
        "no-match regionId ⇒ NO body (downstream blocked), got changed={:?}",
        rep.changed
    );
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "a no-match is a deterministic FAILURE, NOT NeedsRepair"
    );
    let reason = extrude_error_reason(&rep).expect("extrude step is Error on no-match");
    assert!(
        reason.contains(bogus),
        "the OP_FAILED message names the requested regionId, got {reason:?}"
    );
    assert!(
        reason.contains("available"),
        "the message lists the available region ids, got {reason:?}"
    );
    assert!(rt.undo(), "undo the no-match extrude");
    eprintln!("multi-region NO-MATCH PASS: '{bogus}' ⇒ OP_FAILED, no body — {reason}");

    // Omitting regionId (empty) ⇒ first-region fallback, deterministic across replays.
    add_op(&mut rt, extrude_record(sid, "", DIST));
    let r1 = regen_all(&mut rt).await;
    let _ = published(&r1, "empty-region extrude");
    let m1 = body_mesh(&mut rt, r1.changed[0].0).await;
    let v1 = mesh_volume(&validate_mesh_blob(&m1).unwrap(), &m1);
    let r2 = regen_all(&mut rt).await;
    let _ = published(&r2, "empty-region extrude replay");
    let m2 = body_mesh(&mut rt, r2.changed[0].0).await;
    let v2 = mesh_volume(&validate_mesh_blob(&m2).unwrap(), &m2);
    assert!(
        (v1 - v2).abs() < 1.0,
        "empty regionId → first-region fallback is deterministic ({v1} vs {v2})"
    );
    assert!(
        vols_by_region.iter().any(|v| (v - v1).abs() < 1.0),
        "the fallback footprint matches one of the detected regions ({v1})"
    );
    eprintln!("multi-region PASS: distinct footprints {vols_by_region:?}, fallback {v1:.1}");

    wm.shutdown().await;
}

/// Derive the ordered `SketchEditOp` batch from a populated sketch (as m2_gate does).
fn edit_ops(sk: &Sketch) -> Vec<onecad_core::edit::SketchEditOp> {
    use onecad_core::edit::SketchEditOp;
    let mut ops = Vec::new();
    for ent in sk.entities() {
        ops.push(SketchEditOp::AddEntity {
            entity: ent.clone(),
        });
    }
    for con in sk.constraints() {
        ops.push(SketchEditOp::AddConstraint {
            constraint: con.clone(),
        });
    }
    ops
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared H5-B setup: rectangle → extrude box → promote a vertical edge → fillet it.
// Returns the runtime, the body, the promoted edge ElementId, and the pre-edit
// (volume, face_count).
// ─────────────────────────────────────────────────────────────────────────────

struct FilletedBox {
    body: BodyId,
    edge_el: ElementId,
    /// The world midpoint of the filleted edge (the ref's anchor) — used to dry-run
    /// re-resolve the binding through the worker after the edit.
    edge_anchor: Vec3,
    filleted: bool,
    vol: f64,
    faces: u32,
    /// Volume of the SHARP box, measured before the fillet op is added. The edge-op
    /// swap proof compares REMOVED volume (`base_vol - vol`), not absolutes.
    base_vol: f64,
}

async fn build_filleted_box(rt: &mut DocumentRuntime, sid: SketchId) -> FilletedBox {
    let sketch = single_rect(sid, 0.0, 0.0, 40.0, 20.0);
    add_op(rt, sketch_record(&sketch));
    add_op(rt, extrude_record(sid, "", 25.0));
    let report = regen_all(rt).await;
    let _ = published(&report, "H5-B extrude");
    let body = report.changed[0].0;
    let snap_id = SnapshotId(report.snapshot_id);

    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "a box has 6 faces");
    let base_vol = mesh_volume(&view, &mesh);
    let (edge_key, edge_anchor) = vertical_edge_pick(&view, &mesh);

    // Promote the picked edge → a persistent Rust-minted ElementId.
    let anchor = AnchorIntent {
        world_point: edge_anchor,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let promoted = rt
        .promote_selection(snap_id, body, vec![(TopoKey::new(&edge_key), Some(anchor))])
        .await
        .expect("promote edge");
    let edge_el = ElementId::new(promoted[0].element_id.clone());
    assert!(edge_el.as_str().starts_with("el_"), "Rust-minted edge id");

    add_op(rt, fillet_record(body, edge_el.clone(), edge_anchor, 2.0));
    let fil_report = regen_all(rt).await;
    let fil_snap = published(&fil_report, "H5-B fillet").clone();
    let filleted = fil_snap.repair_summary.needs_repair_count == 0;
    let fmesh = body_mesh(rt, body).await;
    let fview = validate_mesh_blob(&fmesh).expect("filleted MESH1 validates");
    let vol = mesh_volume(&fview, &fmesh);
    eprintln!(
        "H5-B setup: filleted={filleted}, faces={}, baseVol={base_vol:.1}, vol={vol:.1}, edgeId={}, needsRepair={}",
        fview.face_count,
        edge_el.as_str(),
        fil_snap.repair_summary.needs_repair_count
    );
    FilletedBox {
        body,
        edge_el,
        edge_anchor,
        filleted,
        vol,
        faces: fview.face_count,
        base_vol,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// H6a — the EDIT-SCOPED descriptor-tie veto, proved in BOTH directions.
// ─────────────────────────────────────────────────────────────────────────────

/// H6a **flagship pin** — the gesture the whole ladder exists to serve, driven
/// through the EDIT LANE (`ToEnd { from: 1 }` ⇒ `editedFrom: 1`), plus the reopen
/// half of the gate.
///
/// The scene is symmetric on purpose: the four vertical edges of a plain box are
/// EXACT descriptor twins (same length, same tangent up to sign, same relative
/// adjacency hash), so the frozen `intent.descriptor` cannot separate them and the
/// stored ANCHOR is the only thing that can. That is the congruent-twin case H5
/// measured — and it is also the single most ordinary thing a user does: fillet an
/// edge of a box, then change the extrude depth.
///
/// Growing the depth slides that edge ALONG ITS OWN AXIS. Its midpoint moves, but
/// the edge still passes exactly through the stored anchor, so the anchor never went
/// stale for it — it is **anchor-exact**, the veto's carve-out applies, and the
/// fillet must re-bind CLEANLY. A veto without that carve-out fails right here
/// (a blanket edit-scoped refusal NeedsRepairs every box fillet after every edit);
/// `h6a_edit_lane_vetoes_a_drifted_twin` is the other half that proves it did not
/// simply give up on B3.
///
/// The reopen leg (`ToEnd { from: 0 }` ⇒ no `editedFrom`) pins the second axis: the
/// SAME document replayed from 0 off disk rebuilds byte-identical geometry, so the
/// anchor is authoritative there regardless of the carve-out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h6a_flagship_edit_lane_fillet_survives_and_reopens_clean() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("h6a.onecad");
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x6A));

    let setup = build_filleted_box(&mut rt, sid).await;
    assert!(
        setup.filleted,
        "H6a precondition: the fillet APPLIES on the clean box (faces {})",
        setup.faces
    );

    // Save the CLEAN, pre-edit document — this is what the reopen half replays.
    rt.save(&path, save_meta()).expect("save clean document");

    // ── (1) EDIT LANE: change the extrude depth at step 1, regen from 1. ──────
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EXTRUDE_REC)),
        op: extrude_op(sid, "", 30.0),
    })
    .expect("edit extrude depth");
    let edited = regen_from(&mut rt, 1).await;
    let edited_snap = match &edited.outcome {
        Outcome::Published(s) => s.clone(),
        // A non-Published outcome would mean the plan never reached the fillet at
        // all, which does not exercise the carve-out — fail loudly rather than pass.
        other => panic!("H6a edit lane: expected Published, got {other:?}"),
    };
    eprintln!(
        "H6a FLAGSHIP edit lane (editedFrom=1): needsRepair={}",
        edited_snap.repair_summary.needs_repair_count
    );
    assert_eq!(
        edited_snap.repair_summary.needs_repair_count, 0,
        "H6a FLAGSHIP: the filleted edge slid along its own axis and still passes \
         through its stored anchor — it is anchor-exact, so the edit-scoped veto \
         MUST NOT fire and the fillet must re-bind cleanly"
    );
    assert!(
        rt.repair_items().is_empty(),
        "H6a FLAGSHIP: no repair items after a surviving rebind"
    );
    // Not a vacuous zero: the fillet is really on the deeper box.
    let emesh = body_mesh(&mut rt, setup.body).await;
    let eview = validate_mesh_blob(&emesh).expect("post-edit MESH1 validates");
    assert!(
        eview.face_count >= 7,
        "H6a FLAGSHIP: the fillet SURVIVED (faces {} >= 7)",
        eview.face_count
    );
    assert!(
        mesh_volume(&eview, &emesh) > setup.vol,
        "H6a FLAGSHIP: the deeper box really is bigger"
    );

    // ── (2) REOPEN: the same document off disk, replayed from 0, no edit. ─────
    let mut reopened = open_over(&wm, &path);
    let replay = regen_from(&mut reopened, 0).await;
    let snap = published(&replay, "H6a reopen").clone();
    eprintln!(
        "H6a reopen (no editedFrom): needsRepair={}",
        snap.repair_summary.needs_repair_count
    );
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "H6a: a from-0 reopen rebuilds the geometry every anchor was authored \
         against — the anchor is authoritative there and the tie resolves clean"
    );
    assert!(
        reopened.repair_items().is_empty(),
        "H6a: no repair items survive a clean reopen"
    );
    // And the fillet really applied on reopen (not a vacuous zero).
    let mesh = body_mesh(&mut reopened, setup.body).await;
    let view = validate_mesh_blob(&mesh).expect("reopened MESH1 validates");
    assert!(
        view.face_count >= 7,
        "H6a reopen: the fillet is present (faces {} >= 7)",
        view.face_count
    );

    wm.shutdown().await;
}

/// H6a **the other half** — the DRIFT class the veto must still catch, so the
/// anchor-exact carve-out cannot be accused of quietly disabling it.
///
/// Same symmetric box, same edit lane, one difference: the fillet's stored anchor is
/// authored deliberately OFF its edge, pushed toward the body interior. After the
/// upstream edit nothing is sitting at that anchor any more — the ref's own edge is
/// no longer anchor-exact, and the only thing the ladder could do is bind whichever
/// congruent twin happens to be nearest a point that names nothing. That is exactly
/// review finding B3, and it must come back as deterministic `NeedsRepair`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h6a_edit_lane_vetoes_a_drifted_twin() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x6B));

    // Box 40×20×25 (steps 0,1), exactly as `build_filleted_box` — but the fillet is
    // authored with a DRIFTED anchor, so it is inlined rather than shared.
    let sketch = single_rect(sid, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(&sketch));
    add_op(&mut rt, extrude_record(sid, "", 25.0));
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "H6a drift extrude");
    let body = report.changed[0].0;
    let snap_id = SnapshotId(report.snapshot_id);

    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let (edge_key, edge_anchor) = vertical_edge_pick(&view, &mesh);
    let centre = bbox_center(&view);

    // Push the anchor 4 mm off its edge, toward the body centre. The anchor-exact
    // threshold on this body is 0.05 × 0.5 × ‖(40,20,30)‖ ≈ 1.35 mm, so 4 mm is
    // decisively OUTSIDE it — while still far nearer this edge than any twin
    // (≈20 mm away), so the pre-veto policy would happily auto-bind.
    let dx = centre.x - edge_anchor.x;
    let dy = centre.y - edge_anchor.y;
    let len = dx.hypot(dy);
    let drifted = Vec3::new_unchecked(
        edge_anchor.x + 4.0 * dx / len,
        edge_anchor.y + 4.0 * dy / len,
        edge_anchor.z,
    );

    // Promote with the TRUE pick (that is what the user clicked); the ref the op
    // carries is what drifts.
    let anchor = AnchorIntent {
        world_point: edge_anchor,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let promoted = rt
        .promote_selection(snap_id, body, vec![(TopoKey::new(&edge_key), Some(anchor))])
        .await
        .expect("promote edge");
    let edge_el = ElementId::new(promoted[0].element_id.clone());
    add_op(&mut rt, fillet_record(body, edge_el, drifted, 2.0));
    let clean = published(&regen_all(&mut rt).await, "H6a drift fillet").clone();
    assert_eq!(
        clean.repair_summary.needs_repair_count, 0,
        "H6a drift precondition: on the UNEDITED box the drifted anchor still \
         resolves (no edit context ⇒ the anchor is authoritative)"
    );

    // ── EDIT LANE: the upstream extrude moves, the anchor names nothing. ──────
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EXTRUDE_REC)),
        op: extrude_op(sid, "", 30.0),
    })
    .expect("edit extrude depth");
    let edited = regen_from(&mut rt, 1).await;
    let repairs = match &edited.outcome {
        Outcome::Published(s) => s.repair_summary.needs_repair_count,
        other => panic!("H6a drift edit lane: expected Published, got {other:?}"),
    };
    eprintln!("H6a DRIFT edit lane (editedFrom=1): needsRepair={repairs}");
    assert!(
        repairs > 0,
        "H6a DRIFT: with nothing sitting at the stale anchor, a congruent twin must \
         NOT be bound on proximity alone (review finding B3)"
    );
    let items = rt.repair_items();
    assert!(
        items
            .iter()
            .any(|i| i.ref_id.contains(&format!("{:x}", FILLET_REC))),
        "H6a DRIFT: the repair item names the fillet's ref — got {:?}",
        items.iter().map(|i| i.ref_id.clone()).collect::<Vec<_>>()
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Deliverable 5b — SMALL parametric edit: the fillet SURVIVES with a STABLE id.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5b_fillet_survives_small_edit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5B));

    let setup = build_filleted_box(&mut rt, sid).await;
    assert!(
        setup.filleted,
        "H5-B precondition: the fillet APPLIES on the clean box (faces {} > 6)",
        setup.faces
    );
    assert!(setup.faces >= 7, "filleting one edge adds a rolled face");

    // SMALL parametric edit: extrude depth 25 → 30 (the vertical edge grows; its XY
    // corner + direction are unchanged, so the ladder rebinds it — case e golden path).
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EXTRUDE_REC)),
        op: extrude_op(sid, "", 30.0),
    })
    .expect("edit extrude depth");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "H5-B small edit").clone();

    // (1) The fillet step is Ok — NOT NeedsRepair.
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "small edit: the fillet REBINDS (no NeedsRepair) — H5-B fixed"
    );
    assert!(
        report.needs_repair.is_empty(),
        "small edit: the needs-repair event set is empty (repairs cleared)"
    );

    // (2) The fillet still applies (rolled face present; volume grew with the depth).
    let mesh = body_mesh(&mut rt, setup.body).await;
    let view = validate_mesh_blob(&mesh).expect("post-edit MESH1 validates");
    assert!(
        view.face_count >= 7,
        "the fillet SURVIVED the edit (faces {} ≥ 7, a rolled face still present)",
        view.face_count
    );
    let vol = mesh_volume(&view, &mesh);
    assert!(
        vol > setup.vol,
        "deeper box ⇒ larger volume ({vol:.1} > {:.1})",
        setup.vol
    );

    // (3) Close the loop through the WORKER's identity machinery (NOT Rust's stored
    // edge_ids copy, which regen never mutates — that would be tautological). Dry-run
    // re-resolve the filleted edge's ref against the NEW head snapshot via the worker
    // ladder (`resolve_refs`) and assert it re-binds to the SAME minted ElementId the
    // original promotion produced (Invariant 1: the id never changes because geometry
    // changed). The probe carries an UNBOUND `elementId` + the edge anchor so the
    // worker runs the ladder (not the `unchanged` short-circuit) and returns the
    // minted id for the resolved topoKey.
    let items = rt.repair_items();
    assert!(items.is_empty(), "no repair items after a surviving rebind");
    let probe = ResolveRef {
        ref_id: "fillet.edge.reresolve".into(),
        element: ElementRef {
            primary: Some(PrimaryRef {
                body: setup.body,
                element: setup.edge_el.clone(),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: setup.edge_anchor,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        },
    };
    let reres = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: SnapshotId(report.snapshot_id),
            refs: vec![probe.clone()],
        })
        .await
        .expect("resolve_refs dry-run against the new head");
    assert_eq!(reres.len(), 1);
    eprintln!("H5-B re-resolve outcome: {:?}", reres[0].outcome);
    // Close the loop through the WORKER's ladder (resolve_refs), NOT Rust's stored
    // edge_ids copy. A fillet CONSUMES its edge (rolls the sharp edge into a face), so
    // re-resolving that edge against the NEW head must NOT `autoBind`: a unique sharp
    // edge surviving at the promoted corner would exist ONLY if the fillet had bound to
    // a DIFFERENT edge (a mis-bind). NeedsRepair here — the worker's ladder finds the
    // two fillet-BORDER edges (a tie) at the promoted corner — is positive, worker-side
    // proof the fillet consumed the CORRECT (promoted) edge. (The reviewer's sketched
    // `autoBind` assumed the edge survives; for a fillet it does not, so the correct
    // discriminator is inverted — `autoBind` would be the failure.)
    let reason_and_keys = |o: &ResolveOutcome| -> (String, Vec<String>) {
        match o {
            ResolveOutcome::NeedsRepair(item) => (
                format!("{:?}", item.reason),
                item.candidates
                    .iter()
                    .map(|c| c.topo_key.as_str().to_string())
                    .collect(),
            ),
            other => panic!(
                "the promoted edge did NOT NeedsRepair — got {other:?}. An `autoBind` here \
                 means the promoted edge SURVIVED ⇒ the fillet mis-bound to a WRONG edge \
                 (never a silent wrong bind); the fillet must consume the edge it filleted."
            ),
        }
    };
    let (reason, keys) = reason_and_keys(&reres[0].outcome);
    if let ResolveOutcome::NeedsRepair(item) = &reres[0].outcome {
        assert_eq!(
            item.element_id.as_ref().map(ElementId::as_str),
            Some(setup.edge_el.as_str()),
            "the worker's ladder processed the SAME minted ElementId the promotion produced"
        );
        assert!(
            !item.candidates.is_empty() && keys.iter().all(|k| !k.is_empty()),
            "topoKey evidence present on the re-resolve candidates"
        );
    }
    // Determinism: the dry-run reproduces IDENTICAL evidence on replay.
    let reres2 = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: SnapshotId(report.snapshot_id),
            refs: vec![probe],
        })
        .await
        .expect("resolve_refs replay");
    let (reason2, keys2) = reason_and_keys(&reres2[0].outcome);
    assert_eq!(
        (&reason, &keys),
        (&reason2, &keys2),
        "the worker re-resolve is deterministic across replays"
    );
    eprintln!(
        "H5-B SMALL-EDIT PASS: fillet survived (faces {}), worker consumed the CORRECT edge \
         (re-resolve ⇒ {reason}, candidates {keys:?}), id {} carried through, vol {vol:.1}",
        view.face_count,
        setup.edge_el.as_str()
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// FILLET-CHAMFER-UNIFY W3 — a committed edge op's TYPE is re-editable.
// ─────────────────────────────────────────────────────────────────────────────

/// A re-edit that FLIPS a committed Fillet into a Chamfer regenerates the OTHER
/// kernel geometry, and undo puts the fillet back.
///
/// The proof is the REMOVED volume, never the record's own `opType` (the guard
/// just wrote that — asserting it is a tautology) and never the history label
/// (this file's `fillet_record` stamps `name: "Fillet"`, which
/// `document_runtime` prefers over `default_label`; the app's own FE-authored
/// records carry no name, so the label there does follow the swap).
///
/// Analytically, on one straight edge of length `L`:
///   * a fillet removes `(1 − π/4)·r²·L ≈ 0.2146·r²·L`,
///   * an equal-leg chamfer removes `0.5·d²·L`,
/// so at `r == d` the chamfer removes ≈ **2.33×** what the fillet did. The
/// assertion uses a 1.8 floor, comfortably clear of tessellation error on both
/// meshes while still excluding "the geometry never changed" (ratio ≈ 1.0).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fillet_reedit_swaps_to_chamfer_and_regens() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x60));

    let setup = build_filleted_box(&mut rt, sid).await;
    assert!(
        setup.filleted,
        "precondition: the fillet applies on the clean box"
    );
    let removed_fillet = setup.base_vol - setup.vol;
    assert!(
        removed_fillet > 0.0,
        "precondition: the fillet REMOVED material (base {:.3} > filleted {:.3})",
        setup.base_vol,
        setup.vol
    );

    // The re-edit the unified edge tool emits: SAME record, SAME edges, SAME size —
    // only the opType flips. This is the one `UpdateOperationParams` that may.
    let swap = Operation::Known(KnownOperation::Chamfer(ChamferParams {
        radius: Scalar::new(2.0),
        distance2: None,
        edge_ids: vec![setup.edge_el.clone()],
        edges: vec![ElementRef {
            primary: Some(PrimaryRef {
                body: setup.body,
                element: setup.edge_el.clone(),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: setup.edge_anchor,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        }],
        chain_tangent_edges: false,
        extra: Default::default(),
    }));
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(FILLET_REC)),
        op: swap,
    })
    .expect("core sanctions the Fillet→Chamfer swap");

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "edge-op swap").clone();
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the swapped op binds the SAME edge — no NeedsRepair"
    );
    assert!(
        report.needs_repair.is_empty(),
        "the needs-repair event set is empty after the swap"
    );

    let mesh = body_mesh(&mut rt, setup.body).await;
    let view = validate_mesh_blob(&mesh).expect("chamfered MESH1 validates");
    let vol_chamfer = mesh_volume(&view, &mesh);
    let removed_chamfer = setup.base_vol - vol_chamfer;
    let ratio = removed_chamfer / removed_fillet;
    eprintln!(
        "W3 swap: base={:.3} fillet={:.3} chamfer={vol_chamfer:.3} \
         removed(fillet)={removed_fillet:.3} removed(chamfer)={removed_chamfer:.3} ratio={ratio:.3}",
        setup.base_vol, setup.vol
    );

    // A ratio near 1.0 means the regen re-ran the SAME op (the swap never reached
    // the kernel); anything between is geometry neither model predicts. Both are
    // hard failures — there is no "close enough" reading of this number.
    assert!(
        ratio > 1.8,
        "the chamfer must remove ≈2.33× the fillet's material (analytic 0.5·d²·L / 0.2146·r²·L), \
         measured {ratio:.3}. A ratio near 1.0 means the opType swap never reached the worker; \
         anything in (0.9, 1.8) is AMBIGUOUS geometry and is a failure, not a pass."
    );
    assert!(
        view.face_count >= 7,
        "the chamfer still adds a face (faces {})",
        view.face_count
    );

    // Undo restores the whole prior record (opType included) — the fillet geometry
    // comes back on the next regen.
    assert!(rt.undo(), "the swap is undoable");
    let back_report = regen_all(&mut rt).await;
    let back_snap = published(&back_report, "edge-op swap undo").clone();
    assert_eq!(
        back_snap.repair_summary.needs_repair_count, 0,
        "the restored fillet re-binds cleanly too"
    );
    let back_mesh = body_mesh(&mut rt, setup.body).await;
    let back_view = validate_mesh_blob(&back_mesh).expect("restored MESH1 validates");
    let vol_back = mesh_volume(&back_view, &back_mesh);
    assert!(
        (vol_back - setup.vol).abs() < 1e-6 * setup.base_vol.max(1.0),
        "undo restores the FILLET volume ({vol_back:.3} ≈ {:.3}, chamfer was {vol_chamfer:.3})",
        setup.vol
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// WP-C T2a — the two-distance chamfer, end to end against the real kernel.
// ─────────────────────────────────────────────────────────────────────────────

/// The chamfer record the FE's unified edge tool emits for `setup`'s edge, with
/// an optional second leg (SCHEMA §7.3, 2026-08-03).
fn chamfer_op_on(setup: &FilletedBox, radius: f64, distance2: Option<f64>) -> Operation {
    Operation::Known(KnownOperation::Chamfer(ChamferParams {
        radius: Scalar::new(radius),
        distance2: distance2.map(Scalar::new),
        edge_ids: vec![setup.edge_el.clone()],
        edges: vec![ElementRef {
            primary: Some(PrimaryRef {
                body: setup.body,
                element: setup.edge_el.clone(),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: setup.edge_anchor,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        }],
        chain_tangent_edges: false,
        extra: Default::default(),
    }))
}

/// Exact kernel volume + centroid of a body (`QueryMassProperties`, SCHEMA §7.5) —
/// read from the BRep, not from a tessellation, so the analytic wedge below can be
/// asserted to kernel precision rather than to mesh-chord precision.
async fn exact_mass(wm: &WorkerManager, body: BodyId) -> (f64, [f64; 3]) {
    let m = onecad_lib::worker::ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties");
    (m.volume, m.centroid)
}

/// A two-distance chamfer removes the EXACT asymmetric wedge `d1·d2/2·L`, the
/// second leg is re-editable, and the SCHEMA §7.3 flip precondition holds against
/// the real single-writer: while `distance2` is set the record will not flip to
/// Fillet, and once cleared it flips exactly as the W3 swap always did.
///
/// The volume is the proof that `distance2` REACHED the kernel: an equal-leg
/// reading of the same params removes `d1²/2·L` or `d2²/2·L`, both far from
/// `d1·d2/2·L` at these legs (25 / 156.25 vs 62.5 mm³ on the 25 mm edge).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chamfer_two_distance_volume_reedit_and_flip_gate() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x61));

    let setup = build_filleted_box(&mut rt, sid).await;
    assert!(setup.filleted, "precondition: the fillet applies");
    // `build_filleted_box` extrudes a 40×20 rect 25 mm, so the picked vertical
    // edge — and therefore every chamfer swept along it — is 25 mm long.
    const EDGE_LEN: f64 = 25.0;
    // The sharp box's volume, measured from its MESH1 — a box tessellates into
    // exact triangles, so this is the same 20000.0 the kernel would report.
    let base_vol = setup.base_vol;
    let wedge = |d1: f64, d2: f64| 0.5 * d1 * d2 * EDGE_LEN;

    // (1) Fillet → two-distance Chamfer. The target type can hold every field, so
    // this direction of the swap is open (unlike the reverse, gated below).
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(FILLET_REC)),
        op: chamfer_op_on(&setup, 2.0, Some(5.0)),
    })
    .expect("Fillet → two-distance Chamfer is accepted");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "two-distance chamfer").clone();
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the two-distance chamfer binds the SAME edge"
    );

    let (vol_2x5, centroid_2x5) = exact_mass(&wm, setup.body).await;
    let removed = base_vol - vol_2x5;
    eprintln!(
        "T2a: base={base_vol:.4} vol(2×5)={vol_2x5:.4} removed={removed:.4} \
         analytic={:.4} centroid={centroid_2x5:?}",
        wedge(2.0, 5.0)
    );
    assert!(
        (removed - wedge(2.0, 5.0)).abs() < 1e-6 * base_vol,
        "removed {removed:.6} != analytic d1·d2/2·L = {:.6}",
        wedge(2.0, 5.0)
    );
    // …and it is NEITHER equal-leg reading of the same two numbers.
    for equal in [2.0, 5.0] {
        assert!(
            (removed - wedge(equal, equal)).abs() > 1.0,
            "removed {removed:.6} must not match the equal-leg {equal} cut ({:.6}) — \
             distance2 would not have reached the kernel",
            wedge(equal, equal)
        );
    }

    // (2) DETERMINISM: replaying the same record against the same predecessor
    // rebuilds the identical solid. Volume alone cannot see a flipped reference
    // face (d1·d2/2 is symmetric in the legs), so the CENTROID is asserted too.
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(FILLET_REC)),
        op: chamfer_op_on(&setup, 2.0, Some(5.0)),
    })
    .expect("re-authoring the same params dirties the step");
    let _ = published(&regen_all(&mut rt).await, "two-distance chamfer replay");
    let (vol_replay, centroid_replay) = exact_mass(&wm, setup.body).await;
    assert_eq!(
        (vol_replay, centroid_replay),
        (vol_2x5, centroid_2x5),
        "the reference-face rule is a pure function of the predecessor shape"
    );

    // (3) The SECOND LEG is re-editable on its own (the chip's `d2` field).
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(FILLET_REC)),
        op: chamfer_op_on(&setup, 2.0, Some(7.0)),
    })
    .expect("editing distance2 alone is a plain params edit");
    let _ = published(&regen_all(&mut rt).await, "distance2 re-edit");
    let (vol_2x7, _) = exact_mass(&wm, setup.body).await;
    assert!(
        ((base_vol - vol_2x7) - wedge(2.0, 7.0)).abs() < 1e-6 * base_vol,
        "d2 5→7 removes {:.6}, analytic {:.6}",
        base_vol - vol_2x7,
        wedge(2.0, 7.0)
    );

    // (4) The FLIP GATE: while distance2 is set, Chamfer→Fillet is refused with
    // the standard allow-list reason, NAMING the field — and writes nothing.
    let err = rt
        .apply(EditCommand::UpdateOperationParams {
            record: RecordId(Uuid::from_u128(FILLET_REC)),
            op: fillet_op(&setup, 2.0),
        })
        .expect_err("a two-distance chamfer is not flippable to Fillet");
    let msg = err.to_string();
    assert!(msg.contains("opType"), "standard allow-list reason: {msg}");
    assert!(
        msg.contains("distance2"),
        "the reason NAMES the field: {msg}"
    );
    let (vol_after_reject, _) = exact_mass(&wm, setup.body).await;
    assert_eq!(vol_after_reject, vol_2x7, "a rejected flip changes nothing");

    // (5) Clear the second leg (an ordinary params edit) — the equal-leg chamfer
    // is back — and NOW the plain W3 swap goes through and regenerates a fillet.
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(FILLET_REC)),
        op: chamfer_op_on(&setup, 2.0, None),
    })
    .expect("clearing distance2 is a plain params edit");
    let _ = published(&regen_all(&mut rt).await, "distance2 cleared").clone();
    let (vol_equal, _) = exact_mass(&wm, setup.body).await;
    assert!(
        ((base_vol - vol_equal) - wedge(2.0, 2.0)).abs() < 1e-6 * base_vol,
        "cleared ⇒ equal-leg d=2: removed {:.6}, analytic {:.6}",
        base_vol - vol_equal,
        wedge(2.0, 2.0)
    );

    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(FILLET_REC)),
        op: fillet_op(&setup, 2.0),
    })
    .expect("with distance2 cleared the W3 swap is the plain sanctioned one");
    let back = published(&regen_all(&mut rt).await, "chamfer → fillet").clone();
    assert_eq!(back.repair_summary.needs_repair_count, 0);
    let (vol_fillet, _) = exact_mass(&wm, setup.body).await;
    let removed_fillet = base_vol - vol_fillet;
    // (1 − π/4)·r²·L for r = 2 on the 25 mm edge.
    let analytic_fillet = (1.0 - std::f64::consts::FRAC_PI_4) * 4.0 * EDGE_LEN;
    assert!(
        (removed_fillet - analytic_fillet).abs() < 1e-6 * base_vol,
        "the flip really re-ran the FILLET: removed {removed_fillet:.6}, analytic {analytic_fillet:.6}"
    );

    wm.shutdown().await;
}

/// The Fillet the flip gate above swaps back to — same edge, same evidence.
fn fillet_op(setup: &FilletedBox, radius: f64) -> Operation {
    Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(radius),
        edge_ids: vec![setup.edge_el.clone()],
        edges: vec![ElementRef {
            primary: Some(PrimaryRef {
                body: setup.body,
                element: setup.edge_el.clone(),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: setup.edge_anchor,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        }],
        chain_tangent_edges: false,
        extra: Default::default(),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Deliverable 5c — DESTRUCTIVE edit: deterministic NeedsRepair, never a wrong bind.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5b_destructive_edit_is_deterministic_needs_repair() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5C));

    let setup = build_filleted_box(&mut rt, sid).await;
    assert!(
        setup.filleted,
        "precondition: fillet applies on the clean box"
    );

    // DESTRUCTIVE edit: replace the sketch with a tiny rectangle FAR from the original
    // (the filleted edge's geometry is gone — no candidate clears the 0.85/0.10 gate).
    let tiny_far = single_rect(sid, 500.0, 500.0, 3.0, 2.0);
    let new_sketch = sketch_record(&tiny_far);
    let new_op = new_sketch.op.clone();
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(SKETCH_REC)),
        op: new_op,
    })
    .expect("edit sketch (destructive)");

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "H5-B destructive edit").clone();

    // (1) Deterministic NeedsRepair on the fillet's ref — never a silent wrong bind.
    assert!(
        snap.repair_summary.needs_repair_count > 0,
        "destructive edit ⇒ NeedsRepair STATE (the filleted edge cannot rebind)"
    );
    let items = rt.repair_items();
    assert!(
        items.iter().any(|i| i.ref_id.contains("op_")
            || i.step_index == 2
            || i.element_id.as_ref().map(ElementId::as_str) == Some(setup.edge_el.as_str())),
        "the NeedsRepair item is the FILLET step's edge ref, items={items:?}"
    );

    // (2) The fillet did NOT apply to a wrong edge: the published body is the
    // UNFILLETED tiny box (m−1), volume = 3·2·25 = 150, exactly 6 faces.
    let mesh = body_mesh(&mut rt, body_of(EXTRUDE_REC)).await;
    let view = validate_mesh_blob(&mesh).expect("m−1 MESH1 validates");
    assert_eq!(
        view.face_count, 6,
        "NeedsRepair publishes the UNFILLETED box (6 faces — no wrong-edge fillet)"
    );
    let vol = mesh_volume(&view, &mesh);
    assert!(
        (vol - 3.0 * 2.0 * 25.0).abs() < 1.0,
        "m−1 body is the plain tiny box (vol 150, not a filleted shape), got {vol:.1}"
    );

    // (3) Determinism: a repeated replay produces the IDENTICAL NeedsRepair payload.
    let items_a: Vec<(usize, String, String)> = repair_key(rt.repair_items());
    let report2 = regen_all(&mut rt).await;
    let _ = published(&report2, "H5-B destructive replay");
    let items_b: Vec<(usize, String, String)> = repair_key(rt.repair_items());
    assert_eq!(
        items_a, items_b,
        "the NeedsRepair payload is IDENTICAL across replays (determinism)"
    );
    assert_eq!(
        report.needs_repair.len(),
        report2.needs_repair.len(),
        "the needs-repair event carries the same item count on replay"
    );
    assert!(
        !report.needs_repair.is_empty(),
        "the needs-repair event fired with items (opId/refId/reason/candidateCount)"
    );
    eprintln!(
        "H5-B DESTRUCTIVE PASS: deterministic NeedsRepair, m−1 tiny box (vol {vol:.1}, 6 faces), event items {:?}",
        report.needs_repair
    );

    // ── Finding 2: a refId-only resolve (exactly what RepairPanel sends) hydrates ──
    // the STORED fillet edge ref, so it returns the IDENTICAL evidence as passing the
    // full ref. Before the fix the lean request resolved against an EMPTY bodyId → a
    // degenerate "No candidates" even when the explicit ref surfaces candidates.
    let snap_id = SnapshotId(report.snapshot_id);
    let ref_id = rt
        .repair_items()
        .iter()
        .find(|i| i.step_index == 2)
        .map(|i| i.ref_id.clone())
        .expect("the fillet step has a repair item with a refId");

    // (a) refId-only — the panel's `resolveRefs([{refId}])`, an EMPTY ElementRef.
    let empty_ref = ElementRef {
        primary: None,
        intent: None,
        anchor: None,
        extra: Default::default(),
    };
    let lean = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: snap_id,
            refs: vec![ResolveRef {
                ref_id: ref_id.clone(),
                element: empty_ref,
            }],
        })
        .await
        .expect("refId-only resolve (hydrated from the stored ref)");

    // (b) explicit — the FULL stored fillet edge ref, read back from the RECORD.
    //
    // It is read rather than rebuilt because the single writer stamps
    // `intent.descriptor` onto the ref when the op is authored (HISTORY-HARDEN H5).
    // A hand-rolled `{primary, anchor}` probe would carry no intent, and the two
    // resolves would legitimately differ — comparing them would then only prove
    // that the test forgot the evidence, not that hydration works.
    let explicit_ref: ElementRef = serde_json::from_value(
        rt.operation_params(RecordId(Uuid::from_u128(FILLET_REC)))
            .expect("the fillet record's stored params")["edges"][0]
            .clone(),
    )
    .expect("the stored edge ref deserializes");
    assert_eq!(
        explicit_ref.primary.as_ref().map(|p| p.element.as_str()),
        Some(setup.edge_el.as_str()),
        "the stored ref still names the promoted element"
    );
    assert!(
        explicit_ref.intent.is_some(),
        "H5: the single writer stamped descriptor evidence on the authored ref"
    );
    let explicit = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: snap_id,
            refs: vec![ResolveRef {
                ref_id: ref_id.clone(),
                element: explicit_ref,
            }],
        })
        .await
        .expect("explicit-ref resolve");

    assert_eq!(lean.len(), 1);
    assert_eq!(explicit.len(), 1);
    // The hydrated refId-only resolve is byte-identical to the explicit-ref resolve.
    assert_eq!(
        lean[0].outcome, explicit[0].outcome,
        "Finding 2: refId-only resolve must hydrate the stored ref and match the explicit-ref result"
    );
    let candidate_keys = |o: &ResolveOutcome| -> Vec<String> {
        match o {
            ResolveOutcome::NeedsRepair(item) => item
                .candidates
                .iter()
                .map(|c| c.topo_key.as_str().to_string())
                .collect(),
            _ => Vec::new(),
        }
    };
    let lean_keys = candidate_keys(&lean[0].outcome);
    assert_eq!(
        lean_keys,
        candidate_keys(&explicit[0].outcome),
        "the hydrated resolve carries the SAME candidate evidence as the explicit ref"
    );
    assert!(
        matches!(lean[0].outcome, ResolveOutcome::NeedsRepair(_)),
        "the refId-only resolve is a proper NeedsRepair (never a silent bind), got {:?}",
        lean[0].outcome
    );
    assert!(
        !lean_keys.is_empty(),
        "the hydrated refId-only resolve surfaces candidates (NOT the pre-fix empty-body \
         'No candidates') — {} candidate(s)",
        lean_keys.len()
    );
    eprintln!(
        "FINDING-2 PASS: refId-only resolve ({ref_id}) hydrated to {} candidate(s), identical to explicit-ref",
        lean_keys.len()
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// M4a strict region rule — a STALE non-empty regionId after a sketch edit is a
// deterministic OP_FAILURE (never a silently-different body). Complements case (c):
// there the fillet ladder blocks; here the extrude's own region binding blocks.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_region_id_after_sketch_edit_fails_deterministically() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5E));

    // Sketch A → its real normative region id.
    let sketch_a = rect_at(sid, 0x1000, 0.0, 0.0, 40.0, 20.0);
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Rect", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&sketch_a))
        .await
        .expect("upsert A");
    let region_a = rt.finish_sketch(sid).await.expect("finish A").regions[0]
        .region_id
        .clone();
    assert!(
        region_a.starts_with("r_"),
        "real region id, got {region_a:?}"
    );

    // Extrude region A by its real id → binds cleanly, produces a body.
    // (finish_sketch minted the sketch's timeline record — production shape;
    // a manual sketch_record here would be an impossible duplicate step.)
    add_op(&mut rt, extrude_record(sid, &region_a, 25.0));
    let ok = regen_all(&mut rt).await;
    let _ = published(&ok, "region A extrude");
    assert_eq!(ok.changed.len(), 1, "region A extrude produces a body");
    assert!(
        extrude_error_reason(&ok).is_none(),
        "the extrude binds region A cleanly (no error)"
    );

    // DESTRUCTIVE edit: replace the sketch's edges (base 0x9000 ⇒ new entity UUIDs ⇒ a
    // NEW region id — region ids hash entity ids, not positions), so the extrude's
    // stored `region_a` is now STALE (delete-and-replace-the-line, corpus e spirit).
    // The target is the MINTED record — fetch its id from the projection.
    let minted_sketch_record = {
        let feature = rt
            .projection()
            .features
            .into_iter()
            .find(|f| f.op_type == "Sketch")
            .expect("finish minted the Sketch timeline record");
        RecordId(Uuid::parse_str(&feature.id).expect("record id parses"))
    };
    let sketch_b = rect_at(sid, 0x9000, 0.0, 0.0, 40.0, 20.0);
    rt.apply(EditCommand::UpdateOperationParams {
        record: minted_sketch_record,
        op: sketch_record(&sketch_b).op,
    })
    .expect("edit sketch (replace edges)");

    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "stale region extrude").clone();

    // Deterministic FAILURE — never a silently different body.
    assert!(
        rep.changed.is_empty() && snap.bodies.is_empty(),
        "stale regionId ⇒ NO body (downstream blocked), got changed={:?}",
        rep.changed
    );
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "a stale regionId is a deterministic FAILURE, not NeedsRepair"
    );
    let reason = extrude_error_reason(&rep).expect("extrude Error on a stale regionId");
    assert!(
        reason.contains(&region_a),
        "the OP_FAILED message names the stale requested id {region_a}, got {reason:?}"
    );

    // Replay reproduces the IDENTICAL failure.
    let rep2 = regen_all(&mut rt).await;
    let reason2 = extrude_error_reason(&rep2).expect("extrude Error on replay");
    assert_eq!(
        reason, reason2,
        "the failure reason is IDENTICAL on replay (determinism)"
    );
    eprintln!("STALE-REGION PASS: stale '{region_a}' after edit ⇒ deterministic OP_FAILED, no body — {reason}");

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Finding 1 — a fillet built from the EXACT JSON the FRONTEND MAPPER emits
// (`operationToEditCommand` → `filletParams`) applies through the real worker.
// Before the fix the UI authored a pure-`edgeIds` fillet whose `derive_inputs`
// populated NO body → `FilletChamferOp::target_body_of` failed "requires body input".
// The mapper now carries typed `edges` (bare-uuid `primary.bodyId` + anchor) in
// lockstep with `edgeIds`, so a UI-selected fillet reaches the worker with a body.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fillet_applies_from_frontend_mapper_json() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5F));

    // Build + publish a box; pick a vertical edge (topoKey + world-midpoint anchor).
    let sketch = single_rect(sid, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(&sketch));
    add_op(&mut rt, extrude_record(sid, "", 25.0));
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "frontend-json box");
    let body = report.changed[0].0;
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "a box has 6 faces");
    let (edge_key, edge_anchor) = vertical_edge_pick(&view, &mesh);

    // The EXACT `apply_edit_command` payload the frontend mapper emits for a
    // UI-selected fillet: `{recordId, opType, params}` with typed `edges` (bare-uuid
    // `primary.bodyId`, `elementId` == `edgeIds[0]` — F2 lockstep — + the pick anchor)
    // parallel to `edgeIds`. `bodyId` is the BARE uuid the frontend holds after
    // `bareBodyId` (core `BodyId` serde is `#[serde(transparent)]`).
    let record_id = Uuid::from_u128(FILLET_REC);
    let json = serde_json::json!({
        "cmd": "addOperation",
        "atCursor": true,
        "record": {
            "recordId": record_id.to_string(),
            "opType": "Fillet",
            "params": {
                "radius": { "value": 2.0 },
                "edgeIds": [edge_key.clone()],
                "chainTangentEdges": true,
                "edges": [{
                    "primary": { "bodyId": body.to_string(), "elementId": edge_key.clone(), "kind": "edge" },
                    "anchor": { "worldPoint": [edge_anchor.x, edge_anchor.y, edge_anchor.z] }
                }]
            }
        }
    });
    let cmd: EditCommand =
        serde_json::from_value(json).expect("frontend fillet JSON deserializes to an EditCommand");
    rt.apply(cmd)
        .expect("apply the frontend-shaped fillet (lockstep validation passes)");

    let rep_f = regen_all(&mut rt).await;
    let snap_f = published(&rep_f, "frontend-json fillet").clone();
    // The body input RESOLVED (not the pre-fix "requires body input"): the fillet APPLIES
    // — a rolled face is added on the same body (6 → ≥7). A vertical box edge reliably
    // applies (as `build_filleted_box` asserts), so NeedsRepair here would be a regression.
    assert_eq!(
        snap_f.repair_summary.needs_repair_count, 0,
        "the frontend-shaped fillet binds its body + applies (no NeedsRepair)"
    );
    let fmesh = body_mesh(&mut rt, body).await;
    let fview = validate_mesh_blob(&fmesh).expect("filleted MESH1 validates");
    assert!(
        fview.face_count >= 7,
        "fillet APPLIED — a rolled face is added (6 → ≥7), got {}",
        fview.face_count
    );
    eprintln!(
        "FINDING-1 PASS: frontend-mapper fillet JSON APPLIES — faces {} → {} on edge {edge_key}",
        view.face_count, fview.face_count
    );

    wm.shutdown().await;
}

/// A stable comparison key for a repair item set (ignores non-deterministic fields).
fn repair_key(items: &[onecad_core::document::repair::RepairItem]) -> Vec<(usize, String, String)> {
    let mut v: Vec<(usize, String, String)> = items
        .iter()
        .map(|i| {
            (
                i.step_index,
                i.ref_id.clone(),
                format!(
                    "{:?}/{:?}/{}",
                    i.ladder_failed,
                    i.reason,
                    i.candidates.len()
                ),
            )
        })
        .collect();
    v.sort();
    v
}

// ─────────────────────────────────────────────────────────────────────────────
// Deliverable 5d — SYMMETRIC ambiguity (corpus f): a descriptor tie ⇒ NeedsRepair,
// never a guess. Driven via the real worker's `ResolveRefs` (dry-run ladder) on a
// box whose top/bottom faces are EQUIDISTANT from a centre anchor (the exact §9
// symmetric-tie the worker's calibration fixture `resolve_refs.ndjson` pins).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symmetric_ambiguity_resolves_to_needs_repair() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5D));

    // Build + publish a plain box (40×20×25).
    let sketch = single_rect(sid, 0.0, 0.0, 40.0, 20.0);
    add_op(&mut rt, sketch_record(&sketch));
    add_op(&mut rt, extrude_record(sid, "", 25.0));
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "symmetric box");
    let body = report.changed[0].0;
    let snap_id = SnapshotId(report.snapshot_id);
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let centre = bbox_center(&view);

    // A face ref anchored at the body CENTRE: the top face and the bottom face are
    // EQUIDISTANT (a mirror-symmetric tie). Auto-bind requires score1 ≥ 0.85 AND
    // margin ≥ 0.10; a tie has margin ≈ 0 ⇒ NeedsRepair (never a guess). SCHEMA §10.
    let face_ref = ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new("el_symmetric_probe"),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: centre,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let res = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: snap_id,
            refs: vec![ResolveRef {
                ref_id: "op_fillet.input0".into(),
                element: face_ref,
            }],
        })
        .await
        .expect("ResolveRefs live");
    assert_eq!(res.len(), 1);
    match &res[0].outcome {
        ResolveOutcome::NeedsRepair(item) => {
            eprintln!(
                "symmetric tie ⇒ NeedsRepair: reason={:?}, ladder={:?}, candidates={}",
                item.reason,
                item.ladder_failed,
                item.candidates.len()
            );
        }
        other => {
            panic!("corpus f: a symmetric tie MUST NeedsRepair (never a guess), got {other:?}")
        }
    }
    eprintln!("SYMMETRIC PASS: descriptor tie ⇒ NeedsRepair (never a silent bind)");

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// HISTORY-HARDEN H5 — `intent.descriptor` evidence vs. a nearest-centroid anchor
//
// The shared body is a "comb": a base bar with two ribs of DIFFERENT cross
// section, extruded in +Z. The op under test fillets the WIDE rib's top edge; an
// upstream parametric edit then TRANSLATES the wide rib away and brings the narrow
// rib to the exact position the wide one vacated — a decoy sitting on the stale
// anchor.
//
// Anchor-only scoring (`Scoring.cpp`: `anchor` is the sole feature at weight 0.25,
// so `score == anchor_similarity`) sees a candidate coincident with the stale
// anchor and binds it at ≈1.0 with a wide margin. That bind is invisible in the
// repair state and only shows up in the REMOVED VOLUME: a fillet of radius r on an
// edge of length L removes exactly `(1 − π/4)·r²·L`, so the wide rib's 30 mm edge
// and the narrow rib's 12 mm edge leave signatures 2.5× apart.
//
// With `intent.descriptor` stamped, `magnitude` (edge length, weight 0.25), `type`
// (weight 0.20) and `tangent` (weight 0.20) enter the score, so the decoy can no
// longer win on position alone.
// ─────────────────────────────────────────────────────────────────────────────

/// Comb geometry (mm). The base bar spans `x ∈ [0, COMB_SPAN]`, `y ∈ [0, BASE_TOP]`;
/// both ribs rise from `BASE_TOP` to `RIB_TOP`; the solid is `COMB_DEPTH` deep in +Z.
const COMB_SPAN: f64 = 120.0;
const BASE_TOP: f64 = 10.0;
const RIB_TOP: f64 = 40.0;
/// Deep enough that the authored rim edge's CONGRUENT TWIN on the opposite face
/// (`z = COMB_DEPTH`, identical type/length/tangent/adjacency) stays outside the
/// 0.10 auto-bind margin. Descriptor evidence matches that twin perfectly, so the
/// anchor term is the ONLY separator and must carry ≥ 0.10/0.25 of its range:
/// `COMB_DEPTH ≥ 0.4 · (0.5 · bodyDiagonal)` ⇒ ≥ 25.8 mm here. 45 leaves headroom.
const COMB_DEPTH: f64 = 45.0;
const RIB_WIDE: f64 = 30.0;
const RIB_NARROW: f64 = 12.0;
/// Fillet radius used by every H5 case.
const H5_RADIUS: f64 = 2.0;

/// Sketch `(u, v)` → world, through the plane `sketch_record` stamps
/// ([`xy_plane_ref`]: `xAxis = +Y`, `yAxis = −X`), i.e. `world = (−v, u, z)`. The
/// worker builds the profile from the RECORD's plane, so a test that wants to
/// address a sketch feature in world space must go through the same mapping.
fn sketch_to_world(u: f64, v: f64, z: f64) -> Vec3 {
    let p = xy_plane_ref();
    Vec3::new_unchecked(
        p.origin.x + u * p.x_axis.x + v * p.y_axis.x + z * p.normal.x,
        p.origin.y + u * p.x_axis.y + v * p.y_axis.y + z * p.normal.y,
        p.origin.z + u * p.x_axis.z + v * p.y_axis.z + z * p.normal.z,
    )
}

/// Volume a radius-`r` fillet removes from a straight edge of length `l`:
/// the corner prism minus the quarter cylinder, `(1 − π/4)·r²·l`.
fn fillet_removed(r: f64, l: f64) -> f64 {
    (1.0 - std::f64::consts::FRAC_PI_4) * r * r * l
}

/// Adds a closed, fully-constrained (dof-0) polygon through `pts` (CCW).
///
/// Same marshaller shape as [`add_rect`] — two synthesized point entities per line
/// — chained corner-to-corner with `Coincident` and pinned with one `Fixed` per
/// corner. That is exactly `4N − 2N − 2N = 0` dof with no redundant constraint, and
/// it keeps the ENTITY IDS stable while the coordinates move, so an edit that
/// re-authors the polygon is a pure parametric translation (the region id hashes
/// entity ids, not positions).
fn add_poly(sk: &mut Sketch, base: u128, pts: &[(f64, f64)]) {
    let n = pts.len() as u128;
    let e = |k: u128| EntityId(Uuid::from_u128(base + k));
    let c = |k: u128| ConstraintId(Uuid::from_u128(base + 0x400 + k));
    for i in 0..n {
        let (x0, y0) = pts[i as usize];
        let (x1, y1) = pts[((i + 1) % n) as usize];
        sk.add_entity(SketchEntity::point(
            e(2 * i),
            Vec2::new_unchecked(x0, y0),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::point(
            e(2 * i + 1),
            Vec2::new_unchecked(x1, y1),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::line(
            e(0x100 + i),
            e(2 * i),
            e(2 * i + 1),
            false,
        ))
        .unwrap();
    }
    for i in 0..n {
        sk.add_constraint(Constraint::Coincident {
            id: c(i),
            point1: e(2 * i + 1),
            point2: e(2 * ((i + 1) % n)),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
        sk.add_constraint(Constraint::Fixed {
            id: c(0x100 + i),
            point: e(2 * i),
            at: Vec2::new_unchecked(pts[i as usize].0, pts[i as usize].1),
        })
        .unwrap();
    }
}

/// A comb outline whose LEFT and RIGHT rib are each `(x0, width)`. Twelve corners,
/// CCW from the origin; re-authoring with different rib placements moves geometry
/// only (identical entity ids, identical topology).
fn comb_sketch(sid: SketchId, left: (f64, f64), right: (f64, f64)) -> Sketch {
    let (l0, l1) = (left.0, left.0 + left.1);
    let (r0, r1) = (right.0, right.0 + right.1);
    assert!(
        0.0 < l0 && l1 < r0 && r1 < COMB_SPAN,
        "the outline must stay simple: 0 < {l0} < {l1} < {r0} < {r1} < {COMB_SPAN}"
    );
    let pts = [
        (0.0, 0.0),
        (COMB_SPAN, 0.0),
        (COMB_SPAN, BASE_TOP),
        (r1, BASE_TOP),
        (r1, RIB_TOP),
        (r0, RIB_TOP),
        (r0, BASE_TOP),
        (l1, BASE_TOP),
        (l1, RIB_TOP),
        (l0, RIB_TOP),
        (l0, BASE_TOP),
        (0.0, BASE_TOP),
    ];
    let mut sk = Sketch::on_world_plane(sid, "Comb", WorldPlane::XY);
    add_poly(&mut sk, 0x7000, &pts);
    sk
}

/// Exact volume of the comb solid (the extruded outline's area × depth) — the
/// analytic baseline the removed-volume proofs subtract from. Unchanged by an edit
/// that only MOVES the ribs.
fn comb_volume(left_w: f64, right_w: f64) -> f64 {
    (COMB_SPAN * BASE_TOP + (left_w + right_w) * (RIB_TOP - BASE_TOP)) * COMB_DEPTH
}

/// The MESH1 edge whose polyline centroid is nearest `target` →
/// `(topoKey, centroid, polyline length)`.
fn edge_pick_near(view: &MeshHeaderView, blob: &[u8], target: Vec3) -> (String, Vec3, f64) {
    assert!(view.has_edges(), "MESH1 must carry edges for the pick");
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
    let mut best: Option<(usize, f64, Vec3, f64)> = None;
    for i in 0..view.edge_count as usize {
        let first = u32_le(blob, erbase + i * 8) as usize;
        let count = u32_le(blob, erbase + i * 8 + 4) as usize;
        if count == 0 {
            continue;
        }
        let pt = |p: usize| -> [f64; 3] {
            let o = epbase + (first + p) * 12;
            [
                f32_le(blob, o) as f64,
                f32_le(blob, o + 4) as f64,
                f32_le(blob, o + 8) as f64,
            ]
        };
        let (mut sx, mut sy, mut sz) = (0.0f64, 0.0f64, 0.0f64);
        let mut len = 0.0f64;
        for p in 0..count {
            let v = pt(p);
            sx += v[0];
            sy += v[1];
            sz += v[2];
            if p > 0 {
                let u = pt(p - 1);
                len +=
                    ((v[0] - u[0]).powi(2) + (v[1] - u[1]).powi(2) + (v[2] - u[2]).powi(2)).sqrt();
            }
        }
        let n = count as f64;
        let centroid = Vec3::new_unchecked(sx / n, sy / n, sz / n);
        let d = (centroid.x - target.x).powi(2)
            + (centroid.y - target.y).powi(2)
            + (centroid.z - target.z).powi(2);
        if best.is_none_or(|(_, bd, _, _)| d < bd) {
            best = Some((i, d, centroid, len));
        }
    }
    let (idx, _d, centroid, len) = best.expect("at least one edge");
    (keys[idx].clone(), centroid, len)
}

/// `(score, margin, label)` of the ladder's verdict for one dry-run resolution —
/// the H6 decision gate's raw input. `NeedsRepair` reports its best candidate's
/// pair (SCHEMA §9 `score1`/`score1 − score2`).
fn score_margin(outcome: &ResolveOutcome) -> (f64, f64, String) {
    match outcome {
        ResolveOutcome::AutoBind {
            score,
            margin,
            topo_key,
            ..
        } => (
            *score,
            *margin,
            format!(
                "AutoBind(topoKey={})",
                topo_key.as_ref().map_or("-", TopoKey::as_str)
            ),
        ),
        ResolveOutcome::NeedsRepair(item) => {
            let best = item.candidates.first();
            (
                best.map_or(0.0, |c| c.score),
                best.map_or(0.0, |c| c.margin),
                format!(
                    "NeedsRepair({:?}/{:?}, {} candidates)",
                    item.ladder_failed,
                    item.reason,
                    item.candidates.len()
                ),
            )
        }
        ResolveOutcome::Unchanged { element_id } => (
            1.0,
            1.0,
            format!(
                "Unchanged({})",
                element_id.as_ref().map_or("-", ElementId::as_str)
            ),
        ),
    }
}

/// The `(score, margin)` the REGEN ITSELF recorded for the fillet step, when the
/// in-op ladder refused to bind. This is the primary H6 measurement: it is scored
/// against the true candidate pool the op saw (`m−1` geometry, the filleted edge
/// NOT yet consumed). `None` when the step bound cleanly — an auto-bind emits no
/// repair item, so its score is not observable from here.
fn in_op_score_margin(rt: &DocumentRuntime) -> Option<(f64, f64, String)> {
    let item = fillet_repair_item(rt)?;
    let best = item.candidates.first();
    Some((
        best.map_or(0.0, |c| c.score),
        best.map_or(0.0, |c| c.margin),
        format!(
            "{:?}/{:?}, {} candidates",
            item.ladder_failed,
            item.reason,
            item.candidates.len()
        ),
    ))
}

/// The regen's own repair item for the fillet's input slot, if the step refused.
fn fillet_repair_item(rt: &DocumentRuntime) -> Option<&onecad_core::document::repair::RepairItem> {
    let record = RecordId(Uuid::from_u128(FILLET_REC)).to_string();
    rt.repair_items()
        .iter()
        .find(|i| i.ref_id.contains(&record))
}

/// Prints the ladder's RANKED candidate list — the H6 gate reads these rows to see
/// which feature carried (or sank) the score.
fn print_candidates(rt: &DocumentRuntime, label: &str) {
    let Some(item) = fillet_repair_item(rt) else {
        eprintln!("  RANKED [{label}] (no repair item — the step bound cleanly)");
        return;
    };
    for (i, c) in item.candidates.iter().enumerate() {
        eprintln!(
            "  RANKED [{label}] #{i} {} score={:.4} margin={:.4} at ({:.1},{:.1},{:.1}) — {} | {}",
            c.topo_key.as_str(),
            c.score,
            c.margin,
            c.world_pos.x,
            c.world_pos.y,
            c.world_pos.z,
            c.summary,
            serde_json::to_string(&c.extra).unwrap_or_default()
        );
    }
}

/// Dry-runs the STORED fillet edge ref (input slot 0 of `FILLET_REC`) against
/// `snapshot` through the worker's ladder and prints `(score, margin)`.
///
/// Goes through the refId-only path on purpose: `resolve_refs` hydrates it from the
/// record the single writer actually stored, so the measurement is of the
/// production ref — `intent` included — and not of a probe re-built by the test.
///
/// **Caveat for the H6 gate**: a dry run resolves against the PUBLISHED head. When
/// the fillet applied, that head no longer contains the edge it consumed, so the
/// numbers describe a different pool than the op's own L2 saw (the same inversion
/// `h5b_fillet_survives_small_edit` documents). Prefer [`in_op_score_margin`] when
/// the step reported NeedsRepair.
async fn measure_fillet_ref(rt: &DocumentRuntime, snapshot: u64, label: &str) -> (f64, f64) {
    let ref_id = format!("{}.input0", RecordId(Uuid::from_u128(FILLET_REC)));
    let res = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: SnapshotId(snapshot),
            refs: vec![ResolveRef {
                ref_id,
                element: ElementRef {
                    primary: None,
                    intent: None,
                    anchor: None,
                    extra: Default::default(),
                },
            }],
        })
        .await
        .expect("ResolveRefs dry-run");
    assert_eq!(res.len(), 1);
    let (score, margin, what) = score_margin(&res[0].outcome);
    eprintln!("  MEASURED [{label}] dry-run score={score:.4} margin={margin:.4} → {what}");
    if let Some((s, m, w)) = in_op_score_margin(rt) {
        eprintln!("  MEASURED [{label}] in-op   score={s:.4} margin={m:.4} → {w}");
    } else {
        eprintln!("  MEASURED [{label}] in-op   (bound cleanly — no repair item)");
    }
    print_candidates(rt, label);
    (score, margin)
}

/// The `intent` the single writer stamped on the stored fillet edge ref (H5), or
/// `None` when the ref shipped anchor-only.
fn stored_fillet_intent(rt: &DocumentRuntime) -> Option<serde_json::Value> {
    rt.operation_params(RecordId(Uuid::from_u128(FILLET_REC)))?
        .get("edges")?
        .get(0)?
        .get("intent")
        .cloned()
}

/// Builds the comb, fillets the WIDE rib's top edge at `z = 0`, and returns
/// `(body, snapshotId, edgeElementId, anchor, edgeLength, filletedVolume)`.
struct CombFillet {
    body: BodyId,
    snapshot: u64,
    edge_len: f64,
    anchor: Vec3,
}

async fn build_comb_fillet(
    wm: &WorkerManager,
    rt: &mut DocumentRuntime,
    sid: SketchId,
    left: (f64, f64),
    right: (f64, f64),
    filleted_rib: (f64, f64),
) -> CombFillet {
    add_op(rt, sketch_record(&comb_sketch(sid, left, right)));
    add_op(rt, extrude_record(sid, "", COMB_DEPTH));
    let report = regen_all(rt).await;
    let _ = published(&report, "comb extrude");
    let body = report.changed[0].0;
    let base_vol = comb_volume(left.1, right.1);

    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("comb MESH1 validates");
    let measured = mesh_volume(&view, &mesh);
    assert!(
        (measured - base_vol).abs() < 1.0,
        "the comb outline extruded to the analytic {base_vol:.1}, got {measured:.1} — \
         the polygon or its constraints are wrong"
    );

    // The top edge of the rib under test, at z = 0 (in world coordinates).
    let target = sketch_to_world(filleted_rib.0 + filleted_rib.1 / 2.0, RIB_TOP, 0.0);
    let (edge_key, centroid, edge_len) = edge_pick_near(&view, &mesh, target);
    assert!(
        (edge_len - filleted_rib.1).abs() < 0.1,
        "picked the rib's top edge (length {edge_len:.3}, expected {:.3})",
        filleted_rib.1
    );

    let snapshot = report.snapshot_id;
    let anchor = AnchorIntent {
        world_point: centroid,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let promoted = rt
        .promote_selection(
            SnapshotId(snapshot),
            body,
            vec![(TopoKey::new(&edge_key), Some(anchor))],
        )
        .await
        .expect("promote the rib's top edge");
    let edge_el = ElementId::new(promoted[0].element_id.clone());

    add_op(rt, fillet_record(body, edge_el, centroid, H5_RADIUS));
    let fil = regen_all(rt).await;
    let snap = published(&fil, "comb fillet").clone();
    assert_eq!(
        snap.repair_summary.needs_repair_count,
        0,
        "precondition: the fillet APPLIES on the clean comb — repair items {:?}",
        rt.repair_items()
    );
    let (vol, _) = exact_mass(wm, body).await;
    let removed = base_vol - vol;
    let analytic = fillet_removed(H5_RADIUS, edge_len);
    assert!(
        (removed - analytic).abs() < 1e-6 * base_vol,
        "precondition: the fillet removed the analytic (1−π/4)·r²·L = {analytic:.6}, \
         measured {removed:.6}"
    );
    eprintln!(
        "  comb setup: base={base_vol:.3} filleted={vol:.3} removed={removed:.4} \
         (analytic {:.4}) edgeLen={edge_len:.3} anchor=({:.1},{:.1},{:.1}) intent={}",
        fillet_removed(H5_RADIUS, edge_len),
        centroid.x,
        centroid.y,
        centroid.z,
        stored_fillet_intent(rt).map_or("NONE".into(), |v| serde_json::to_string(&v)
            .unwrap_or_default())
    );
    CombFillet {
        body,
        snapshot: fil.snapshot_id,
        edge_len,
        anchor: centroid,
    }
}

/// **The headline case.** A NON-congruent decoy lands exactly on the stale anchor.
///
/// Before: the WIDE rib (30 mm) sits at sketch `u ∈ [20,50]`, the NARROW rib
/// (12 mm) at `u ∈ [60,72]`. The wide rib's top edge at `z = 0` is filleted — its
/// midpoint (sketch `u = 35`) becomes the ref's anchor.
///
/// The edit translates the wide rib to `u ∈ [45,75]` and the narrow rib to
/// `u ∈ [29,41]`, whose top-edge midpoint is `u = 35` — the stale anchor, exactly.
/// The two candidates are then:
///
/// | candidate            | midpoint      | length | fillet removes |
/// |----------------------|---------------|--------|----------------|
/// | narrow rib (decoy)   | sketch u = 35 | 12 mm  | `0.8584·12 = 10.30` |
/// | wide rib (authored)  | sketch u = 60 | 30 mm  | `0.8584·30 = 25.75` |
///
/// Removed volume is therefore a 2.5× discriminator that no repair state can hide.
/// The assertion is deliberately one-sided: whatever the ladder does, it must not
/// fillet the decoy while reporting success.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5_non_congruent_decoy_at_the_stale_anchor() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5001));

    let before_left = (20.0, RIB_WIDE); // [20,50], midpoint 35 — the authored rib
    let before_right = (60.0, RIB_NARROW); // [60,72], midpoint 66
    let after_left = (29.0, RIB_NARROW); // [29,41], midpoint 35 — ON the stale anchor
    let after_right = (45.0, RIB_WIDE); // [45,75], midpoint 60 — the authored rib, moved

    let setup = build_comb_fillet(&wm, &mut rt, sid, before_left, before_right, before_left).await;
    let authored_anchor = sketch_to_world(35.0, RIB_TOP, 0.0);
    assert!(
        (setup.anchor.x - authored_anchor.x).abs() < 0.01
            && (setup.anchor.y - authored_anchor.y).abs() < 0.01
            && (setup.edge_len - RIB_WIDE).abs() < 0.1,
        "precondition: the authored edge is the 30 mm one at sketch u=35, got ({:.3},{:.3}) len={:.3}",
        setup.anchor.x,
        setup.anchor.y,
        setup.edge_len
    );
    let (score0, margin0) = measure_fillet_ref(&rt, setup.snapshot, "clean comb").await;
    assert!(score0 > 0.0, "the clean comb resolves (sanity)");
    let _ = margin0;

    // The upstream parametric edit: both ribs move, the narrow one onto the anchor.
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(SKETCH_REC)),
        op: sketch_record(&comb_sketch(sid, after_left, after_right)).op,
    })
    .expect("translate the ribs");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "H5 decoy edit").clone();

    let base_vol = comb_volume(after_left.1, after_right.1);
    let (vol, _) = exact_mass(&wm, setup.body).await;
    let removed = base_vol - vol;
    let removed_decoy = fillet_removed(H5_RADIUS, RIB_NARROW);
    let removed_true = fillet_removed(H5_RADIUS, RIB_WIDE);
    let needs_repair = snap.repair_summary.needs_repair_count;
    let (score, margin) = measure_fillet_ref(&rt, report.snapshot_id, "post-edit").await;
    eprintln!(
        "H5 DECOY: needsRepair={needs_repair} base={base_vol:.3} vol={vol:.3} removed={removed:.4} \
         | decoy(12mm)={removed_decoy:.4} authored(30mm)={removed_true:.4} \
         | score={score:.4} margin={margin:.4} intent={}",
        stored_fillet_intent(&rt).map_or("NONE".into(), |v| serde_json::to_string(&v)
            .unwrap_or_default())
    );

    // The decision the wave exists to change: a silent bind to the decoy is a
    // FAILURE. Either the authored (30 mm) rib is filleted, or nothing is filleted
    // and the state says NeedsRepair. There is no third reading of this number.
    let is_decoy = (removed - removed_decoy).abs() < 0.1 * removed_decoy;
    let is_authored = (removed - removed_true).abs() < 0.1 * removed_true;
    assert!(
        !(is_decoy && needs_repair == 0),
        "SILENT WRONG BIND: the fillet consumed the DECOY (removed {removed:.4} ≈ the 12 mm \
         rib's {removed_decoy:.4}) and reported no repair — the ladder bound on position alone"
    );
    assert!(
        is_authored || needs_repair > 0,
        "the outcome must be the authored rib (removed ≈ {removed_true:.4}) or a deterministic \
         NeedsRepair; got removed {removed:.4} with needsRepair={needs_repair}"
    );
    if needs_repair > 0 {
        assert!(
            removed < 0.01 * base_vol,
            "a NeedsRepair publishes the UNFILLETED m−1 body, got removed {removed:.4}"
        );
        // The positive H5 claim, and the sharpest one available: even when the
        // 0.10 margin gate refuses to bind, the descriptor has RE-RANKED the pool.
        // The best candidate must be the authored 30 mm rib at its NEW position —
        // the decoy that sits ON the stale anchor (and therefore scores a perfect
        // 0.25 anchor contribution) must NOT lead.
        let item = fillet_repair_item(&rt).expect("a NeedsRepair carries its evidence");
        let best = item.candidates.first().expect("ranked candidates");
        let authored_now = sketch_to_world(after_right.0 + after_right.1 / 2.0, RIB_TOP, 0.0);
        let decoy_now = sketch_to_world(after_left.0 + after_left.1 / 2.0, RIB_TOP, 0.0);
        let near = |a: Vec3, b: Vec3| {
            (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5 && (a.z - b.z).abs() < 0.5
        };
        assert!(
            !near(best.world_pos, decoy_now),
            "the DECOY still leads the ranking ({:?}) — descriptor evidence did not reach the \
             ladder",
            best
        );
        assert!(
            near(best.world_pos, authored_now),
            "the authored 30 mm rib at {authored_now:?} must rank FIRST, got {best:?}"
        );
    }

    wm.shutdown().await;
}

/// The CONGRUENT twin: two identical 12 mm ribs, and the edit puts BOTH of them
/// equidistant from the stale anchor (sketch `u = 20` and `u = 40`, anchor `u = 30`).
///
/// Nothing in the descriptor can break this tie — the two candidate edges have the
/// same type, length and tangent — so the only correct answers are "the authored
/// rib" and "NeedsRepair". This test pins the NO-SILENT-WRONG floor only; actually
/// resolving congruent ties is the H6a veto's job.
///
/// Note the volume signature is deliberately NOT a discriminator here: congruent
/// ribs remove identical volume, which is exactly why the tie must be refused
/// rather than guessed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5_congruent_twin_never_silently_binds() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5002));

    // Both ribs NARROW so two of them fit either side of the anchor after the edit.
    let before_left = (24.0, RIB_NARROW); // [24,36], midpoint 30 — the authored rib
    let before_right = (80.0, RIB_NARROW); // [80,92] — its congruent twin, far away
    let after_left = (14.0, RIB_NARROW); // [14,26], midpoint 20 — 10 mm from the anchor
    let after_right = (34.0, RIB_NARROW); // [34,46], midpoint 40 — 10 mm the other way

    let setup = build_comb_fillet(&wm, &mut rt, sid, before_left, before_right, before_left).await;
    let authored_anchor = sketch_to_world(30.0, RIB_TOP, 0.0);
    assert!(
        (setup.anchor.x - authored_anchor.x).abs() < 0.01
            && (setup.anchor.y - authored_anchor.y).abs() < 0.01,
        "precondition: the authored edge is at sketch u=30, got ({:.3},{:.3})",
        setup.anchor.x,
        setup.anchor.y
    );

    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(SKETCH_REC)),
        op: sketch_record(&comb_sketch(sid, after_left, after_right)).op,
    })
    .expect("translate both ribs");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "H5 congruent edit").clone();

    let base_vol = comb_volume(after_left.1, after_right.1);
    let (vol, _) = exact_mass(&wm, setup.body).await;
    let removed = base_vol - vol;
    let needs_repair = snap.repair_summary.needs_repair_count;
    let (score, margin) = measure_fillet_ref(&rt, report.snapshot_id, "congruent post-edit").await;
    eprintln!(
        "H5 CONGRUENT: needsRepair={needs_repair} base={base_vol:.3} vol={vol:.3} \
         removed={removed:.4} (a congruent rib removes {:.4}) score={score:.4} margin={margin:.4}",
        fillet_removed(H5_RADIUS, RIB_NARROW)
    );

    // A congruent pair is either bound (to a geometrically equivalent edge) or
    // refused — but it must never be bound with the confidence of a unique match.
    // Whatever happens, the published body must be self-consistent: a fillet was
    // applied to SOME 30 mm rib, or none was applied at all.
    let one_rib = fillet_removed(H5_RADIUS, RIB_NARROW);
    assert!(
        removed < 0.01 * base_vol || (removed - one_rib).abs() < 0.1 * one_rib,
        "either nothing was filleted (NeedsRepair) or exactly ONE congruent rib was — \
         got removed {removed:.4}, one rib is {one_rib:.4}"
    );
    if needs_repair == 0 {
        assert!(
            (removed - one_rib).abs() < 0.1 * one_rib,
            "a clean regen must have actually applied the fillet, got removed {removed:.4}"
        );
    }

    wm.shutdown().await;
}

/// The BENIGN large edit: the fillet's edge stays the only edge of its kind but
/// moves FAR (extrude depth 25 → 100 quadruples the filleted vertical edge and
/// drags its midpoint 37.5 mm along +Z).
///
/// Anchor-only that is a big miss — the anchor similarity falls to
/// `1 − 37.5 / (0.5·diag)`. With `intent.descriptor` the type/length/tangent
/// features carry the score. The measured `(score, margin)` is the number the H6
/// gate consumes; the assertion is the invariant, not the number.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5_benign_large_edit_measures_the_ladder() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let sid = SketchId(Uuid::from_u128(0x5003));

    let setup = build_filleted_box(&mut rt, sid).await;
    assert!(
        setup.filleted,
        "precondition: the fillet applies at depth 25"
    );
    eprintln!(
        "H5 BENIGN: stored intent = {}",
        stored_fillet_intent(&rt).map_or("NONE".into(), |v| serde_json::to_string(&v)
            .unwrap_or_default())
    );

    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(EXTRUDE_REC)),
        op: extrude_op(sid, "", 100.0),
    })
    .expect("extrude depth 25 → 100");
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "H5 benign large edit").clone();
    let needs_repair = snap.repair_summary.needs_repair_count;

    let mesh = body_mesh(&mut rt, setup.body).await;
    let view = validate_mesh_blob(&mesh).expect("post-edit MESH1 validates");
    let vol = mesh_volume(&view, &mesh);
    // 40×20×100 minus the fillet on one 100 mm vertical edge.
    let base_vol = 40.0 * 20.0 * 100.0;
    let removed = base_vol - vol;
    let (score, margin) = measure_fillet_ref(&rt, report.snapshot_id, "depth 25→100").await;
    eprintln!(
        "H5 BENIGN: needsRepair={needs_repair} faces={} base={base_vol:.1} vol={vol:.1} \
         removed={removed:.4} (analytic {:.4}) score={score:.4} margin={margin:.4}",
        view.face_count,
        fillet_removed(H5_RADIUS, 100.0)
    );

    // The invariant: a clean regen means the fillet really re-applied to the SAME
    // (now longer) edge — proven by the removed volume, not by the repair count.
    if needs_repair == 0 {
        assert!(
            view.face_count >= 7,
            "a clean rebind keeps the rolled face (faces {})",
            view.face_count
        );
        assert!(
            (removed - fillet_removed(H5_RADIUS, 100.0)).abs()
                < 0.05 * fillet_removed(H5_RADIUS, 100.0),
            "the rebound fillet swept the FULL 100 mm edge: removed {removed:.4}, analytic {:.4}",
            fillet_removed(H5_RADIUS, 100.0)
        );
    } else {
        assert_eq!(
            view.face_count, 6,
            "a NeedsRepair publishes the UNFILLETED m−1 box (faces {})",
            view.face_count
        );
    }

    wm.shutdown().await;
}
