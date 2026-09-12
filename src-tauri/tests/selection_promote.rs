//! WP-U4 — **can the frontend act on every id the viewport shows it?**
//!
//! The 2026-09-11 native UX review hit "Selection is out of date — pick again"
//! (`RefUnresolved` from [`DocumentRuntime::promote_selection`]) on a face pick
//! taken against the CURRENT head snapshot, 24 s after a hole regen published it:
//! `gate_stale_pick` passed, so the refusal came from the worker's
//! `AcquireElementIds` dropping the pick (`ElementIdentity.cpp` `resolve_pick`
//! matched neither the topoKey nor the anchor).
//!
//! The cause, measured by the first version of this file: the MESH1 id table is a
//! **two-namespace** table. `Tessellate.cpp` substitutes the minted ElementId for
//! any element that already has a live binding — the Hole's host face, right
//! after the hole regen — and leaves every other element named by its
//! snapshot-scoped TopoKey. The viewport hands whichever label it read straight
//! back (`Picker.ts` → `promotePick`), and `resolve_pick` parses only
//! `f:N`/`e:N`/`v:N`.
//!
//! So the contract the product depends on is per-namespace, and this file pins
//! both halves of it for every id the worker's own tessellation names at the head
//! snapshot:
//!
//! > * a **TopoKey** promotes at that snapshot and comes back naming the key that
//! >   was asked for;
//! > * a **minted ElementId** is REFUSED by promotion — by name, because it is
//! >   already the persistent handle promotion mints — and `QueryElement` reports
//! >   it present, so the pick needs nothing further to be usable.
//!
//! Each test drives the REAL C++ OCCT worker through the app's single-writer
//! [`DocumentRuntime`], and after EVERY published regen it re-fetches the body
//! mesh, enumerates FACE_ID_CHARS + EDGE_ID_CHARS, and checks each id against the
//! rung that owns it. Failures are accumulated and reported together — one run
//! gives the whole picture rather than the first casualty.
//!
//! Coverage (the reviewer's own sequences):
//! * `revolved_flange_mesh_ids_promote_after_every_hole` — a stepped closed
//!   profile + construction axis revolved 360°, then FOUR Ø6.6 through-holes on
//!   the flange's flat face, the host face re-promoted from its topoKey after
//!   each publish (the "revolve + 4 holes + face pick" scenario, verbatim).
//! * `extruded_box_mesh_ids_promote_after_chamfer` — rectangle → Extrude 12 →
//!   equal-leg Chamfer 3 on one vertical edge (the "displaced highlight after
//!   chamfer" scenario).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! is a quiet local skip unless `ONECAD_REQUIRE_WORKER=1` (CI hard-fails).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ChamferParams, ExtrudeMode, ExtrudeParams, HoleParams, HoleType, KnownOperation,
    Operation, OperationRecord, PlaneKind, RevolveParams, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, AxisRef, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId, TopoKey,
};
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
// Harness (mirrors chamfer_angle.rs / topology_rebind.rs)
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

async fn body_mesh(rt: &mut DocumentRuntime, body: BodyId) -> Arc<Vec<u8>> {
    rt.get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh")
}

fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

fn anchor_at(at: Vec3) -> AnchorIntent {
    AnchorIntent {
        world_point: at,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    }
}

fn anchored_ref(body: BodyId, element: &ElementId, kind: ElementKind, at: Vec3) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: element.clone(),
            kind,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(anchor_at(at)),
        extra: Default::default(),
    }
}

// Record ids — the worker mints `body_<opId>`, so the body UUID IS the record's.
const SKETCH_FLANGE: u128 = 0xF100;
const REVOLVE_FLANGE: u128 = 0xF101;
const HOLE_1: u128 = 0xF110;
const HOLE_2: u128 = 0xF111;
const HOLE_3: u128 = 0xF112;
const HOLE_4: u128 = 0xF113;
const SKETCH_BOX: u128 = 0xB100;
const EXTRUDE_BOX: u128 = 0xB101;
const CHAMFER_BOX: u128 = 0xB102;

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders
// ─────────────────────────────────────────────────────────────────────────────

/// `PlaneKind::Xy`'s canonical frame: `toWorld(u, v) = (-v, u, 0)`. The worker
/// owns the frame for a NAMED plane kind and ignores the explicit axes, so only
/// `kind` is load-bearing on the wire; the axes below document the mapping.
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

fn point(sk: &mut Sketch, id: EntityId, x: f64, y: f64) {
    sk.add_entity(SketchEntity::point(
        id,
        Vec2::new_unchecked(x, y),
        false,
        false,
    ))
    .unwrap();
}

/// A closed, fully-constrained polyline through `pts`, built the marshaller way:
/// two points per segment, `Coincident` at every join, `Fixed` on every segment
/// start. `2·N` points ⇒ `4·N` DOF, met exactly by `N` `Fixed` (`2·N`) plus `N`
/// `Coincident` (`2·N`) — determined, never over-constrained.
///
/// Entity ids: points `base+0 … base+2N-1`, lines `base+0x20 … base+0x20+N-1`.
/// Constraint ids: coincident `base+0x40+i`, fixed `base+0x60+i`.
fn closed_polyline(sk: &mut Sketch, base: u128, pts: &[(f64, f64)]) {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let n = pts.len();
    for (i, &(x, y)) in pts.iter().enumerate() {
        let (nx, ny) = pts[(i + 1) % n];
        let i = i as u128;
        point(sk, e(2 * i), x, y);
        point(sk, e(2 * i + 1), nx, ny);
        sk.add_entity(SketchEntity::line(
            e(0x20 + i),
            e(2 * i),
            e(2 * i + 1),
            false,
        ))
        .unwrap();
    }
    for (i, &(x, y)) in pts.iter().enumerate() {
        let next = (i + 1) % n;
        let (i, next) = (i as u128, next as u128);
        sk.add_constraint(Constraint::Coincident {
            id: c(i),
            point1: e(2 * i + 1),
            point2: e(2 * next),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
        sk.add_constraint(Constraint::Fixed {
            id: c(0x20 + i),
            point: e(2 * i),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(x, y),
        })
        .unwrap();
    }
}

/// The stepped flange profile plus its revolve axis, in sketch `(u, v)`. `u` is
/// the radius from the axis at `u = 0`, `v` the axial coordinate, so under
/// `toWorld(u, v) = (-v, u, 0)` the axis maps to the world X axis and the
/// profile's `v = 0` edge revolves into a flat annulus at world `x = 0` — the
/// flange face the holes are drilled through.
///
/// Flange: radius 4…25, axial 0…6. Hub: radius 4…12, axial 6…30. Bore Ø8.
/// The axis is a plain (open) line: it joins no closed loop, so region detection
/// ignores it while it stays a resolvable `SketchLine` for `AxisRef::SketchLine`.
fn flange_profile(sid: SketchId, base: u128) -> (Sketch, EntityId) {
    let mut sk = Sketch::on_world_plane(sid, "Flange", WorldPlane::XY);
    closed_polyline(
        &mut sk,
        base,
        &[
            (4.0, 0.0),
            (25.0, 0.0),
            (25.0, 6.0),
            (12.0, 6.0),
            (12.0, 30.0),
            (4.0, 30.0),
        ],
    );
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (a_s, a_e, a_line) = (e(0x30), e(0x31), e(0x32));
    point(&mut sk, a_s, 0.0, -5.0);
    point(&mut sk, a_e, 0.0, 35.0);
    sk.add_entity(SketchEntity::line(a_line, a_s, a_e, false))
        .unwrap();
    // Pin both endpoints so the worker's solve places the axis deterministically.
    sk.add_constraint(Constraint::Fixed {
        id: c(0x30),
        point: a_s,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(0.0, -5.0),
    })
    .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(0x31),
        point: a_e,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(0.0, 35.0),
    })
    .unwrap();
    (sk, a_line)
}

/// The 40 × 20 rectangle of scenario (c), the marshaller shape (8 points, 4
/// lines, coincident corners, H/V, a Fixed anchor + H/V dimensions).
fn rect_sketch(sid: SketchId, base: u128, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    point(&mut sk, p0s, 0.0, 0.0);
    point(&mut sk, p0e, w, 0.0);
    point(&mut sk, p1s, w, 0.0);
    point(&mut sk, p1e, w, h);
    point(&mut sk, p2s, w, h);
    point(&mut sk, p2e, 0.0, h);
    point(&mut sk, p3s, 0.0, h);
    point(&mut sk, p3e, 0.0, 0.0);
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
        at: Vec2::new_unchecked(0.0, 0.0),
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

fn revolve_record(
    rec: u128,
    sketch: SketchId,
    angle_deg: f64,
    axis_line: EntityId,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Revolve",
        Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""), // empty ⇒ V1 first-region fallback
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            angle_deg: Scalar::new(angle_deg),
            axis: Some(AxisRef::SketchLine {
                sketch,
                line: axis_line,
                extra: Default::default(),
            }),
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
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
                region: RegionId::new(""),
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

/// One Ø`diameter` through-all hole at world `at`, entering through the promoted
/// host `face` (anchored on that face's own bbox centre, which is the descriptor
/// centre the ladder scores against).
fn hole_record(
    rec: u128,
    body: BodyId,
    face: &ElementId,
    face_at: Vec3,
    at: Vec3,
    diameter: f64,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Hole",
        Operation::Known(KnownOperation::Hole(HoleParams {
            target_body: body,
            face: anchored_ref(body, face, ElementKind::Face, face_at),
            point: at,
            hole_type: HoleType::Simple,
            diameter: Scalar::new(diameter),
            depth: None, // through-all
            cb_diameter: None,
            cb_depth: None,
            cs_diameter: None,
            cs_angle_deg: None,
            thread: None,
            result_policy_version: Some(2),
            extra: Default::default(),
        })),
    )
}

/// An EQUAL-LEG chamfer: `distance2`/`angleDeg` absent, therefore no
/// `referenceFaces` pair (SCHEMA §7.3 — an equal-leg chamfer has no reference
/// face, and [`ChamferParams::validate`] refuses one that carries pairs anyway).
fn chamfer_record(
    rec: u128,
    body: BodyId,
    edge: &ElementId,
    at: Vec3,
    radius: f64,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Chamfer",
        Operation::Known(KnownOperation::Chamfer(ChamferParams {
            radius: Scalar::new(radius),
            distance2: None,
            angle_deg: None,
            edge_ids: vec![edge.clone()],
            edges: vec![anchored_ref(body, edge, ElementKind::Edge, at)],
            reference_faces: Vec::new(),
            reference_face_refs: Vec::new(),
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 id tables — the exact strings `Picker.ts` hands to `promotePick`
// ─────────────────────────────────────────────────────────────────────────────

const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;

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

/// `(faces, edges)` — every id the blob names, in table order.
fn mesh_element_ids(view: &MeshHeaderView, blob: &[u8]) -> (Vec<String>, Vec<String>) {
    assert!(
        view.has_edges(),
        "MESH1 must carry the edge id table — the viewport picks edges off it"
    );
    (
        id_table(
            view,
            blob,
            SEC_FACE_ID_OFFS,
            SEC_FACE_ID_CHARS,
            view.face_count as usize,
        ),
        id_table(
            view,
            blob,
            SEC_EDGE_ID_OFFS,
            SEC_EDGE_ID_CHARS,
            view.edge_count as usize,
        ),
    )
}

/// Pick the edge with the greatest world-Z extent (a vertical box edge, always
/// safely chamferable) → `(mesh id, centroid anchor)`.
fn vertical_edge_pick(view: &MeshHeaderView, blob: &[u8]) -> (String, Vec3) {
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

// ─────────────────────────────────────────────────────────────────────────────
// The gate: promote every id the mesh names, at the snapshot it names them for
// ─────────────────────────────────────────────────────────────────────────────

/// Walks every face/edge id of `body`'s head mesh and checks the id against the
/// rung that OWNS it, returning one human-readable line per FAILURE (empty ⇒ the
/// stage is clean).
///
/// The call shape is the frontend's: the id string verbatim, no anchor
/// (`promotePick` sends one only when the ref carries one, and a hover pick
/// promoted straight off the mesh does not).
///
/// The two namespaces have two different contracts, and conflating them is the
/// bug this file was opened for:
///
/// * a **TopoKey** (`f:N` / `e:N`) is snapshot-scoped evidence — it MUST promote
///   at the snapshot that named it, and the answer must name the key that was
///   asked for.
/// * a **minted ElementId** (`el_…`, substituted into the table by
///   `Tessellate.cpp` for any element with a live binding) is ALREADY the
///   persistent handle promotion exists to mint. Promoting it is a category
///   error, so `promote_selection` must refuse it by name — and `QueryElement`
///   by that elementId must report the element present, which is what makes the
///   refusal safe for the viewport (the pick needs no promotion at all).
async fn promote_every_mesh_id(
    rt: &mut DocumentRuntime,
    wm: &WorkerManager,
    snapshot: SnapshotId,
    body: BodyId,
    stage: &str,
) -> Vec<String> {
    let blob = body_mesh(rt, body).await;
    let view = validate_mesh_blob(&blob).expect("MESH1 validates");
    let (faces, edges) = mesh_element_ids(&view, &blob);
    let minted = faces
        .iter()
        .chain(edges.iter())
        .filter(|id| id.starts_with("el_"))
        .count();
    eprintln!(
        "{stage}: snapshot {} names {} faces + {} edges ({minted} already minted ElementIds)",
        snapshot.0,
        faces.len(),
        edges.len()
    );

    let mut failures = Vec::new();
    let picks = faces
        .iter()
        .map(|id| ("face", id))
        .chain(edges.iter().map(|id| ("edge", id)));
    for (kind, id) in picks {
        let promoted = rt
            .promote_selection(snapshot, body, vec![(TopoKey::new(id.as_str()), None)])
            .await;
        if id.starts_with("el_") {
            match promoted {
                Err(error) if error.to_string().contains("is an ElementId, not a TopoKey") => {}
                Err(error) => failures.push(format!(
                    "{stage}: {kind} id {id:?} (minted ElementId) was refused, but not as an \
                     ElementId — the message must say what the pick actually is: {error}"
                )),
                Ok(promoted) => failures.push(format!(
                    "{stage}: {kind} id {id:?} (minted ElementId) PROMOTED to {:?} — an already \
                     persistent id must never be re-minted",
                    promoted
                        .iter()
                        .map(|p| p.element_id.clone())
                        .collect::<Vec<_>>()
                )),
            }
            // …and it must still be a live element, or the viewport would be
            // holding a handle that resolves nowhere.
            match wm.query_element(snapshot, body, id).await {
                Ok(Some(_)) => {}
                Ok(None) => failures.push(format!(
                    "{stage}: {kind} id {id:?} (minted ElementId) is named by the head MESH1 but \
                     QueryElement reports it absent"
                )),
                Err(error) => failures.push(format!(
                    "{stage}: {kind} id {id:?} (minted ElementId) QueryElement FAILED: {error}"
                )),
            }
            continue;
        }
        match promoted {
            Ok(promoted) if promoted.len() == 1 && promoted[0].topo_key == *id => {}
            Ok(promoted) => failures.push(format!(
                "{stage}: {kind} id {id:?} (TopoKey) promoted to {:?} — expected exactly one \
                 entry naming the requested id",
                promoted
                    .iter()
                    .map(|p| p.topo_key.clone())
                    .collect::<Vec<_>>()
            )),
            Err(error) => failures.push(format!(
                "{stage}: {kind} id {id:?} (TopoKey) REFUSED: {error}"
            )),
        }
    }
    failures
}

/// The `TopoKey` + bbox centre of the flange's flat outer face: the planar
/// annulus at world `x = 0`, i.e. the largest face whose descriptor normal is
/// ±X and whose bbox centre lies on that plane (the hub's `x = −30` cap and the
/// flange's `x = −6` back face are the other ±X planes).
///
/// Scanned through `QueryElement` rather than off the mesh id table on purpose:
/// this is the pick the SCENARIO needs, and it must not depend on the very thing
/// the test is measuring.
async fn flange_face(
    wm: &WorkerManager,
    snapshot: SnapshotId,
    body: BodyId,
    face_count: u32,
) -> (String, Vec3) {
    let mut best: Option<(String, f64, Vec3)> = None;
    for ordinal in 1..=face_count {
        let key = format!("f:{ordinal}");
        let Some(info) = wm
            .query_element_by_topo_key(snapshot, body, &key)
            .await
            .expect("QueryElement by topoKey")
        else {
            continue;
        };
        // `normal` is the UN-oriented surface normal, so match on the AXIS plus
        // the face's own bbox centre (which for a planar face lies on the plane).
        if info.kind != "face" || !info.has_normal {
            continue;
        }
        if (info.normal[0].abs() - 1.0).abs() > 1e-6 || info.center[0].abs() > 1e-6 {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(_, area, _)| info.magnitude > *area)
        {
            best = Some((
                key,
                info.magnitude,
                Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]),
            ));
        }
    }
    let (key, area, at) = best.expect("the revolved flange has a flat face at world x = 0");
    eprintln!(
        "flange face at snapshot {}: {key} (area {area:.3}, centre [{:.3}, {:.3}, {:.3}])",
        snapshot.0, at.x, at.y, at.z
    );
    (key, at)
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) + (b) — revolve, then four holes on the flange face
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revolved_flange_mesh_ids_promote_after_every_hole() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xF1));
    let (sk, axis) = flange_profile(sid, 0x1000);
    add_op(&mut rt, sketch_record(SKETCH_FLANGE, &sk));
    add_op(&mut rt, revolve_record(REVOLVE_FLANGE, sid, 360.0, axis));

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "flange revolve");
    assert!(
        report.failed_steps.is_empty(),
        "the revolve must not fail: {:?}",
        report.failed_steps
    );
    assert_eq!(
        snap.bodies.len(),
        1,
        "360° NewBody revolve = exactly 1 body"
    );
    let body = body_of(REVOLVE_FLANGE);
    let mut snapshot = SnapshotId(report.snapshot_id);

    let mut failures = promote_every_mesh_id(&mut rt, &wm, snapshot, body, "after revolve").await;

    // Four Ø6.6 through-holes at radius 18 on the flange's flat face (radius
    // 4…25), at the four cardinal angles of the y–z plane the X axis revolves
    // into. Each is authored exactly the way the viewport does it: re-pick the
    // host face at the CURRENT head, promote it, then commit.
    let holes = [
        (HOLE_1, Vec3::new_unchecked(0.0, 18.0, 0.0)),
        (HOLE_2, Vec3::new_unchecked(0.0, 0.0, 18.0)),
        (HOLE_3, Vec3::new_unchecked(0.0, -18.0, 0.0)),
        (HOLE_4, Vec3::new_unchecked(0.0, 0.0, -18.0)),
    ];
    for (n, (rec, at)) in holes.iter().enumerate() {
        let face_count = {
            let blob = body_mesh(&mut rt, body).await;
            let view = validate_mesh_blob(&blob).expect("MESH1 validates");
            view.face_count
        };
        let (key, face_at) = flange_face(&wm, snapshot, body, face_count).await;
        let promoted = rt
            .promote_selection(
                snapshot,
                body,
                vec![(TopoKey::new(key.as_str()), Some(anchor_at(face_at)))],
            )
            .await
            .unwrap_or_else(|error| panic!("promote the flange face {key}: {error}"));
        assert_eq!(promoted.len(), 1, "one promoted face");
        assert_eq!(promoted[0].kind, "face");
        let face_id = ElementId::new(&promoted[0].element_id);

        add_op(
            &mut rt,
            hole_record(*rec, body, &face_id, face_at, *at, 6.6),
        );
        let report = regen_all(&mut rt).await;
        let hole_snap = published(&report, "hole commit");
        assert!(
            report.failed_steps.is_empty(),
            "hole {} must not fail: {:?}",
            n + 1,
            report.failed_steps
        );
        assert_eq!(
            hole_snap.repair_summary.needs_repair_count,
            0,
            "hole {} must bind its freshly promoted host face without repair",
            n + 1
        );
        snapshot = SnapshotId(report.snapshot_id);
        failures.extend(
            promote_every_mesh_id(
                &mut rt,
                &wm,
                snapshot,
                body,
                &format!("after hole {}", n + 1),
            )
            .await,
        );
    }

    wm.shutdown().await;
    assert!(
        failures.is_empty(),
        "every id the head MESH1 names must be actionable at that snapshot — a TopoKey \
         by promoting, a minted ElementId by already being persistent and present. The \
         viewport picks by handing these exact strings back. {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) — rectangle → extrude → chamfer
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extruded_box_mesh_ids_promote_after_chamfer() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0xB1));
    add_op(
        &mut rt,
        sketch_record(SKETCH_BOX, &rect_sketch(sid, 0x2000, 40.0, 20.0)),
    );
    add_op(&mut rt, extrude_record(EXTRUDE_BOX, sid, 12.0));
    let report = regen_all(&mut rt).await;
    assert!(
        report.failed_steps.is_empty(),
        "the extrude must not fail: {:?}",
        report.failed_steps
    );
    published(&report, "box extrude");
    let body = body_of(EXTRUDE_BOX);
    let snapshot = SnapshotId(report.snapshot_id);

    // The chamfer's edge pick comes off the SAME mesh table the gate reads, so
    // take it before anything has been promoted — the authoring must not be
    // blocked by the very defect this test measures.
    let (edge_key, edge_at) = {
        let blob = body_mesh(&mut rt, body).await;
        let view = validate_mesh_blob(&blob).expect("box MESH1 validates");
        assert_eq!(view.face_count, 6, "the stock is a sharp six-face box");
        vertical_edge_pick(&view, &blob)
    };
    let promoted = rt
        .promote_selection(
            snapshot,
            body,
            vec![(TopoKey::new(edge_key.as_str()), Some(anchor_at(edge_at)))],
        )
        .await
        .unwrap_or_else(|error| panic!("promote the vertical edge {edge_key}: {error}"));
    assert_eq!(promoted.len(), 1, "one promoted edge");
    assert_eq!(promoted[0].kind, "edge");
    let edge = ElementId::new(&promoted[0].element_id);

    let mut failures = promote_every_mesh_id(&mut rt, &wm, snapshot, body, "after extrude").await;

    add_op(
        &mut rt,
        chamfer_record(CHAMFER_BOX, body, &edge, edge_at, 3.0),
    );
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "chamfer commit");
    assert!(
        report.failed_steps.is_empty(),
        "the chamfer must not fail: {:?}",
        report.failed_steps
    );
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "a freshly promoted edge must chamfer without repair"
    );
    let snapshot = SnapshotId(report.snapshot_id);
    failures.extend(promote_every_mesh_id(&mut rt, &wm, snapshot, body, "after chamfer").await);

    wm.shutdown().await;
    assert!(
        failures.is_empty(),
        "every id the head MESH1 names must be actionable at that snapshot — a TopoKey \
         by promoting, a minted ElementId by already being persistent and present. The \
         viewport picks by handing these exact strings back. {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
