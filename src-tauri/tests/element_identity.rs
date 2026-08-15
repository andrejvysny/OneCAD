//! HISTORY-HARDEN H4 — element-id promotion is SNAPSHOT-SCOPED (VF-M3 + VF-M4).
//!
//! ## The hazard
//!
//! A `TopoKey` (`"f:3"`) is a 1-based ordinal into `TopExp::MapShapes` — SCHEMA §9
//! snapshot-scoped **evidence**, never identity. Two independent paths used to
//! launder one into a persistent, op-referencable `ElementId` for geometry the user
//! never picked:
//!
//! * **VF-M3, the stale request.** `AcquireElementIds` carried a `snapshotId` that
//!   BOTH sides ignored, so a pick captured before a regen resolved happily against
//!   the new head — a different face under the same ordinal.
//! * **VF-M4, the stale cache.** `DocumentRuntime::promoted` was keyed
//!   `(BodyId, TopoKey)` and nothing ever invalidated it, so after an ordinal
//!   renumber a re-pick got back a WRONG id (not merely a missing one). Blast
//!   radius: face-dblclick sketch re-entry, sketch-on-face arming, selection
//!   stamps, and a worker double-mint (one face, two ids).
//!
//! Both are the H5-B class this migration exists to kill. `NeedsRepair` / a loud
//! refusal / a fresh id all beat a silent wrong bind.
//!
//! ## What each case pins
//!
//! * `stale_snapshot_promotion_is_refused` — a pick addressed to a superseded
//!   snapshot is REFUSED (recoverable, naming both ids) and returns no id at all.
//!   Reverting `gate_stale_pick` makes it return the pre-edit id instead.
//! * `ordinal_renumber_mints_a_fresh_id` — a fillet renumbers the ordinals, and the
//!   same `TopoKey` at the NEW head resolves to different geometry (asserted on the
//!   kernel descriptor, not on a tag), so it must mint a FRESH id. Reverting the
//!   `(SnapshotId, …)` cache key makes it hand back the pre-fillet id.
//! * `unchanged_face_keeps_its_id_across_a_regen` — Invariant 1 must still survive a
//!   regen when the geometry genuinely did not move: the descriptor-pinned reuse
//!   rung returns the SAME id, so face-dblclick re-enters the same hosted sketch.
//!   Reverting the rung mints a fresh id every regen.
//!
//! **Why the re-entry case adds a second BODY rather than a hosted sketch + a
//! fillet elsewhere.** A hosted sketch REFERENCES its host face, so
//! `PlanExecutor::resolve_input_refs` mints a worker partition entry for it and the
//! next `AcquireElementIds` echoes that binding — the reuse rung would never run and
//! the case would not be red-first for it. An independent second body leaves the
//! host face untouched. Its binding correctly exists on the original head, but a
//! full replay publishes a new head without guessing topology history; the
//! descriptor-pinned Rust reuse rung then returns and reinstalls the same id.
//!
//! Gated on `ONECAD_WORKER_PATH` / `ONECAD_REQUIRE_WORKER` like every other
//! worker-backed test in this suite (see `wire_contract.rs`'s `real_worker`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, EngineError, GeometryEngine, Outcome, RegenRequest, ResolveOutcome, ResolveRef,
    ResolveRequest,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::export::GeometryExporter;
use onecad_lib::state::AppState;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_on_face.rs / wire_contract.rs — each worker-backed test
// file keeps its own small harness rather than sharing a `tests/common` module).
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

fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn GeometryExporter> = Arc::new(wm.clone());
        let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
        let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
        let face_projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
        let step_import: Arc<dyn StepImport> = Arc::new(wm.clone());
        let circuit: Arc<dyn CircuitControl> = Arc::new(wm.clone());
        let readiness: Arc<dyn WorkerReadiness> = Arc::new(wm.clone());
        (
            engine,
            meshes,
            solver,
            exporter,
            elements,
            preview,
            face_projection,
            step_import,
            circuit,
            readiness,
        )
    }))
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

fn assert_published(report: &RegenReport, what: &str) {
    match &report.outcome {
        Outcome::Published(_) => {}
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

// ── Fixtures ────────────────────────────────────────────────────────────────

const SKETCH_A: u128 = 0xE00;
const EXTRUDE_A: u128 = 0xE01;
const SKETCH_B: u128 = 0xE02;
const EXTRUDE_B: u128 = 0xE03;
const FILLET_REC: u128 = 0xE04;

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(1.0, 0.0, 0.0),
        y_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A fully-constrained rectangle (same marshaller-shaped fixture as
/// `sketch_on_face::rect_sketch`).
fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id, x: f64, y: f64| {
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
    let params = SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // empty ⇒ V1 first-region fallback
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

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
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids: vec![edge_el],
            edges: vec![edge_ref],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

// ── MESH1 pick helpers (copied from sketch_on_face.rs / topology_rebind.rs) ──

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
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

/// The face with the greatest average world-Z (the extrude's far cap): its `TopoKey`.
fn top_face_pick(view: &MeshHeaderView, blob: &[u8]) -> String {
    let fr = view.section(SEC_FACE_RANGES).expect("FACE_RANGES");
    let idx = view.section(SEC_INDICES).expect("INDICES");
    let pos = view.section(SEC_POSITIONS).expect("POSITIONS");
    let (frbase, ibase, pbase) = (fr.offset as usize, idx.offset as usize, pos.offset as usize);
    let keys = id_table(
        view,
        blob,
        SEC_FACE_ID_OFFS,
        SEC_FACE_ID_CHARS,
        view.face_count as usize,
    );
    let mut best: Option<(usize, f64)> = None;
    for f in 0..view.face_count as usize {
        let first = u32_le(blob, frbase + f * 8) as usize;
        let count = u32_le(blob, frbase + f * 8 + 4) as usize;
        let (mut sz, mut n) = (0.0, 0.0f64);
        for t in first..first + count {
            let io = ibase + t * 12;
            for k in 0..3 {
                let v = vertex(blob, pbase, u32_le(blob, io + k * 4) as usize);
                sz += v[2];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let z = sz / n;
        if best.is_none_or(|(_, bz)| z > bz) {
            best = Some((f, z));
        }
    }
    keys[best.expect("at least one face").0].clone()
}

/// The edge with the greatest world-Z extent (a vertical box edge, always safely
/// fillettable) → `(topoKey, centroid anchor)`.
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

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, onecad_core::regen::Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

/// The kernel's own evidence for one `TopoKey` in one snapshot, as the comparable
/// tuple this file asserts geometric identity on. `magnitude` is the exact `GProp`
/// quantity (a face's area), not a tessellation estimate.
async fn descriptor_of(
    wm: &WorkerManager,
    snap: SnapshotId,
    body: BodyId,
    topo_key: &str,
) -> (f64, [f64; 3], f64) {
    let info = ElementQuery::query_element_by_topo_key(wm, snap, body, topo_key)
        .await
        .expect("QueryElement(by topoKey)")
        .expect("the topoKey resolves against the body shape");
    (info.magnitude, info.center, info.size)
}

/// A 40×20×25 box from one rect sketch + one blind extrude.
async fn box_document(rt: &mut DocumentRuntime) -> (BodyId, SnapshotId, String) {
    let sa = SketchId(Uuid::from_u128(0xA1));
    add_op(
        rt,
        sketch_record(SKETCH_A, &rect_sketch(sa, 0x2000, 0.0, 0.0, 40.0, 20.0)),
    );
    add_op(rt, extrude_record(EXTRUDE_A, sa, 25.0));
    let report = regen_all(rt).await;
    assert_published(&report, "box");
    let snap = SnapshotId(report.snapshot_id);
    assert!(snap.0 > 0, "a published regen mints a positive snapshot id");
    let body = body_of(EXTRUDE_A);

    let mesh = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    assert_eq!(view.face_count, 6, "the extruded rect is a 6-faced box");
    (body, snap, top_face_pick(&view, &mesh))
}

fn box_topo_keys() -> Vec<TopoKey> {
    [("f", 6_u32), ("e", 12), ("v", 8)]
        .into_iter()
        .flat_map(|(kind, count)| {
            (1..=count).map(move |ordinal| TopoKey::new(format!("{kind}:{ordinal}")))
        })
        .collect()
}

fn primary_only_ref(body: BodyId, element_id: &str, kind: &str) -> ElementRef {
    let kind = match kind {
        "face" => ElementKind::Face,
        "edge" => ElementKind::Edge,
        "vertex" => ElementKind::Vertex,
        other => panic!("unexpected promoted kind `{other}`"),
    };
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(element_id),
            kind,
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}

async fn assert_fresh_identity(
    wm: &WorkerManager,
    rt: &DocumentRuntime,
    snap: SnapshotId,
    body: BodyId,
    topo_key: &str,
    element_id: &str,
    kind: &str,
) {
    let info = ElementQuery::query_element(wm, snap, body, element_id)
        .await
        .expect("QueryElement(by freshly promoted elementId)");
    let resolutions = rt
        .resolve_refs(ResolveRequest {
            snapshot_id: snap,
            refs: vec![ResolveRef {
                ref_id: format!("fresh.{topo_key}"),
                element: primary_only_ref(body, element_id, kind),
            }],
        })
        .await
        .expect("ResolveRefs(fresh primary-only ref)");

    let info = info.unwrap_or_else(|| {
        panic!(
            "REF-FRESH-1: `{element_id}` for `{topo_key}` is absent from unchanged worker head; \
             ResolveRefs returned {:?}",
            resolutions.first().map(|resolution| &resolution.outcome)
        )
    });
    assert_eq!(info.element_id, element_id, "REF-FRESH-1: id changed");
    assert_eq!(info.topo_key, topo_key, "REF-FRESH-1: TopoKey changed");
    assert_eq!(info.kind, kind, "REF-FRESH-1: kind changed");
    assert_eq!(resolutions.len(), 1, "one result per fresh ref");
    match &resolutions[0].outcome {
        ResolveOutcome::Unchanged {
            element_id: Some(id),
        } => {
            assert_eq!(id.as_str(), element_id, "REF-FRESH-1: resolve changed id");
        }
        outcome => panic!("REF-FRESH-1: expected unchanged, got {outcome:?}"),
    }
}

// REF-H0 — freshly acquired identity must be exact on the unchanged worker head.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_box_subelements_round_trip_on_unchanged_head() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (engine, meshes, solver) = app_state_over(&wm).make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    let (body, snap, _) = box_document(&mut rt).await;
    let keys = box_topo_keys();
    let picks = keys.iter().cloned().map(|key| (key, None)).collect();
    let promoted = rt
        .promote_selection(snap, body, picks)
        .await
        .expect("promote all box subelements");

    assert_eq!(promoted.len(), keys.len(), "all box subelements promoted");
    for item in &promoted {
        assert_fresh_identity(
            &wm,
            &rt,
            snap,
            body,
            &item.topo_key,
            &item.element_id,
            &item.kind,
        )
        .await;
    }
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// VF-M3 — a pick addressed to a superseded snapshot is REFUSED
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_snapshot_promotion_is_refused() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (engine, meshes, solver) = app_state_over(&wm).make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let (body, snap1, top_key) = box_document(&mut rt).await;

    let first = rt
        .promote_selection(snap1, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote at the head snapshot");
    let id1 = first[0].element_id.clone();
    assert!(id1.starts_with("el_"), "Rust-minted id");

    // Any edit that republishes: a second, independent body.
    let sb = SketchId(Uuid::from_u128(0xB1));
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sb, 0x3000, 80.0, 0.0, 10.0, 10.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_B, sb, 10.0));
    let report2 = regen_all(&mut rt).await;
    assert_published(&report2, "second body");
    let snap2 = SnapshotId(report2.snapshot_id);
    assert_ne!(snap1, snap2, "the edit published a NEW snapshot");

    // The pick still names a real face — it is the SNAPSHOT that is stale.
    let err = rt
        .promote_selection(snap1, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect_err(
            "a pick taken against a superseded snapshot must be REFUSED — resolving it \
             would mint a persistent id for whatever face now carries that ordinal",
        );
    match err {
        EngineError::OpFailed {
            recoverable,
            ref message,
            ..
        } => {
            assert!(recoverable, "a stale pick leaves the session editable");
            assert!(
                message.contains(&snap1.0.to_string()) && message.contains(&snap2.0.to_string()),
                "the refusal must name BOTH ids so the frontend can explain itself: {message}"
            );
            assert!(
                message.contains("re-pick"),
                "the refusal must say what to do: {message}"
            );
        }
        other => panic!("expected a recoverable OpFailed, got {other:?}"),
    }

    // And the SAME pick at the new head is accepted — the gate is about staleness,
    // not about the element.
    rt.promote_selection(snap2, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote at the NEW head still works");
}

// ─────────────────────────────────────────────────────────────────────────────
// VF-M4 — an ordinal renumber must mint a FRESH id, never reuse the stale one
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinal_renumber_mints_a_fresh_id() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (engine, meshes, solver) = app_state_over(&wm).make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let (body, snap1, top_key) = box_document(&mut rt).await;
    let before = descriptor_of(&wm, snap1, body, &top_key).await;

    let first = rt
        .promote_selection(snap1, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote the top face");
    let id1 = first[0].element_id.clone();

    // Fillet a vertical edge: the result has MORE faces, so every ordinal above the
    // insertion point shifts and the picked key now names different geometry.
    let mesh = body_mesh(&mut rt, body).await;
    let view = validate_mesh_blob(&mesh).expect("box MESH1 validates");
    let (edge_key, edge_anchor) = vertical_edge_pick(&view, &mesh);
    let edge_el = ElementId::new(
        rt.promote_selection(
            snap1,
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
        .expect("promote the fillet edge")[0]
            .element_id
            .clone(),
    );
    add_op(&mut rt, fillet_record(body, edge_el, edge_anchor, 3.0));
    let report2 = regen_all(&mut rt).await;
    assert_published(&report2, "fillet");
    let snap2 = SnapshotId(report2.snapshot_id);

    let fmesh = body_mesh(&mut rt, body).await;
    let fview = validate_mesh_blob(&fmesh).expect("filleted MESH1 validates");
    assert!(
        fview.face_count > 6,
        "the fillet must actually add a face (got {} — the ordinals never moved, \
         so this case would prove nothing)",
        fview.face_count
    );

    // Ground truth FIRST, from the kernel: the same key now describes different
    // geometry. Without this the id assertion below could pass for the wrong reason.
    let after = descriptor_of(&wm, snap2, body, &top_key).await;
    assert_ne!(
        before, after,
        "setup: `{top_key}` must describe DIFFERENT geometry after the fillet \
         (area/centre/size {before:?} → {after:?})"
    );

    let second = rt
        .promote_selection(snap2, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote at the new head");
    assert_ne!(
        second[0].element_id, id1,
        "the renumbered `{top_key}` names different geometry, so it MUST get a fresh \
         id — handing back the pre-fillet id is the VF-M4 silent wrong bind"
    );

    // Invariant 1 still holds WITHIN the new snapshot.
    let third = rt
        .promote_selection(snap2, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("re-promote at the new head");
    assert_eq!(
        third[0].element_id, second[0].element_id,
        "re-picking the same element in the SAME snapshot returns the same id"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Invariant 1 across a regen — the descriptor-pinned reuse rung
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_face_keeps_its_id_across_a_regen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let (engine, meshes, solver) = app_state_over(&wm).make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);

    let (body, snap1, top_key) = box_document(&mut rt).await;
    let before = descriptor_of(&wm, snap1, body, &top_key).await;

    let first = rt
        .promote_selection(snap1, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("promote the host face");
    let id1 = first[0].element_id.clone();

    // A SECOND, independent body — body A's shape (and therefore every ordinal and
    // descriptor on it) is untouched. This is the "sketch on a face, then model
    // something else, then double-click that face again" flow.
    let sb = SketchId(Uuid::from_u128(0xB2));
    add_op(
        &mut rt,
        sketch_record(SKETCH_B, &rect_sketch(sb, 0x3000, 80.0, 0.0, 10.0, 10.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_B, sb, 10.0));
    let report2 = regen_all(&mut rt).await;
    assert_published(&report2, "second body");
    let snap2 = SnapshotId(report2.snapshot_id);
    assert_ne!(snap1, snap2, "the edit published a NEW snapshot");

    let after = descriptor_of(&wm, snap2, body, &top_key).await;
    assert_eq!(
        before, after,
        "setup: an unrelated second body must leave `{top_key}` describing the SAME \
         geometry — otherwise this case is testing the renumber path, not the pin"
    );

    // A fresh full regen published a different head. The same-head binding is not
    // topology history and therefore is not silently carried across that rebuild;
    // descriptor-pinned Rust reuse must reinstall it during the next promotion.
    assert!(
        ElementQuery::query_element(&wm, snap2, body, &id1)
            .await
            .expect("QueryElement(by elementId)")
            .is_none(),
        "a prior-head promotion is absent after a full rebuild — descriptor-pinned \
         Rust reuse must reinstall it on the new head"
    );

    let second = rt
        .promote_selection(snap2, body, vec![(TopoKey::new(&top_key), None)])
        .await
        .expect("re-promote after the regen");
    assert_eq!(
        second[0].element_id, id1,
        "the face did not move, so re-picking it after a regen must return the SAME \
         id (Invariant 1) — a fresh id here re-creates a sketch instead of re-entering it"
    );
}
