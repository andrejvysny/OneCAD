//! Region derivation + preview fill against the REAL C++ OCCT/PlaneGCS worker.
//!
//! Drives the app's single-writer [`DocumentRuntime`] exactly like
//! `sketch_edit.rs` / `m2_gate.rs` (real worker via `ONECAD_WORKER_PATH`, else
//! the dev-tree fallback; `ONECAD_REQUIRE_WORKER=1` hard-fails a missing binary).
//!
//! This is the cross-layer proof for the MODEL-OPS W0 fix. The worker's
//! `SketchRegions` used to ear-clip a region's OUTER loop only, so a rectangle
//! with an inner circle was served as a solid fill even though `FaceBuilder`
//! builds — and the kernel extrudes — a face WITH the hole. The drag preview
//! disagreed with the committed solid, and a click inside the hole hit-tested as
//! the enclosing region. Covered here:
//!
//! * **nesting** — a contained circle partitions the plane into two selectable
//!   cells: an annulus (circle as hole) and the inner disk.
//! * **hole subtraction** — the `previewTriangles` area that reaches the Rust DTO
//!   is `outer − hole`, not `outer`. Pre-fix this read 800 instead of ~749.
//! * **ring topology** — the only edges used by exactly ONE triangle are the
//!   outer and hole boundaries, so a consumer recovers exactly two rings. The
//!   frontend's `profileFromRegion` depends on this; a bridge segment that leaked
//!   out as a boundary edge would fabricate a wall.
//! * **no-hole control** — a plain rectangle still fills to its full area.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{BodyId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::{EngineError, GeometryEngine};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::dto::PreviewTrianglesDto;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::GestureTarget;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_edit.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail)"
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

fn edit_ops(sk: &Sketch) -> Vec<SketchEditOp> {
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

// ── fixture ids ──────────────────────────────────────────────────────────────
const P0: u128 = 0x100;
const P1: u128 = 0x101;
const P2: u128 = 0x102;
const P3: u128 = 0x103;
const PC: u128 = 0x110; // circle centre
const L0: u128 = 0x200;
const L1: u128 = 0x201;
const L2: u128 = 0x202;
const L3: u128 = 0x203;
const CIRC: u128 = 0x300;

const W: f64 = 40.0;
const H: f64 = 20.0;
const R: f64 = 5.0;

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}

fn point(sk: &mut Sketch, id: u128, x: f64, y: f64) {
    sk.add_entity(SketchEntity::point(
        eid(id),
        Vec2::new_unchecked(x, y),
        false,
        false,
    ))
    .unwrap();
}

/// A closed 40×20 rectangle, optionally with a concentric r=5 circle inside it.
fn rect_sketch(sid: SketchId, with_hole: bool) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Regions", WorldPlane::XY);
    point(&mut sk, P0, 0.0, 0.0);
    point(&mut sk, P1, W, 0.0);
    point(&mut sk, P2, W, H);
    point(&mut sk, P3, 0.0, H);
    for (l, a, b) in [(L0, P0, P1), (L1, P1, P2), (L2, P2, P3), (L3, P3, P0)] {
        sk.add_entity(SketchEntity::line(eid(l), eid(a), eid(b), false))
            .unwrap();
    }
    if with_hole {
        point(&mut sk, PC, W / 2.0, H / 2.0);
        sk.add_entity(SketchEntity::circle(eid(CIRC), eid(PC), R, false).expect("circle"))
            .unwrap();
    }
    sk
}

async fn build_and_finish(
    rt: &mut DocumentRuntime,
    sid: SketchId,
    with_hole: bool,
) -> Vec<onecad_lib::dto::SketchRegionDto> {
    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Regions", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&rect_sketch(sid, with_hole)))
        .await
        .expect("sketch_upsert");
    rt.finish_sketch(sid).await.expect("finish_sketch").regions
}

/// Summed area of every emitted triangle.
fn fill_area(t: &PreviewTrianglesDto) -> f64 {
    let mut area = 0.0;
    let p = |i: u32| -> (f64, f64) {
        let k = i as usize * 2;
        (t.positions[k], t.positions[k + 1])
    };
    for tri in t.indices.as_chunks::<3>().0 {
        let (ax, ay) = p(tri[0]);
        let (bx, by) = p(tri[1]);
        let (cx, cy) = p(tri[2]);
        area += (((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)) / 2.0).abs();
    }
    area
}

/// Undirected edges used by exactly one triangle — the real boundary. Anything a
/// producer added internally (bridges) must NOT show up here.
fn boundary_edge_count(t: &PreviewTrianglesDto) -> usize {
    let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
    for tri in t.indices.as_chunks::<3>().0 {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            *uses.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0) += 1;
        }
    }
    uses.values().filter(|&&n| n == 1).count()
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) A contained circle is a HOLE, and the fill subtracts it.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contained_circle_is_a_hole_and_the_fill_subtracts_it() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let regions = build_and_finish(&mut rt, SketchId(Uuid::from_u128(0x9E00)), true).await;

    assert_eq!(regions.len(), 2, "annulus + inner disk, got {regions:#?}");
    let region = regions
        .iter()
        .find(|region| !region.holes.is_empty())
        .expect("annulus cell");
    let disk = regions
        .iter()
        .find(|region| region.holes.is_empty())
        .expect("inner disk cell");
    assert_ne!(region.region_id, disk.region_id);
    assert_eq!(
        region.holes.len(),
        1,
        "the circle is reported as the region's single hole loop"
    );

    let tris = region
        .preview_triangles
        .as_ref()
        .expect("the region carries a preview fill");

    // The kernel builds this face WITH the hole (FaceBuilder adds the inner wire),
    // so the fill must match: 40·20 − π·5² ≈ 721.46. Pre-fix this was 800 — the
    // preview claimed a slab while the commit produced a tube.
    let want = W * H - std::f64::consts::PI * R * R;
    let got = fill_area(tris);
    eprintln!("fill area = {got:.4} (outer {} − hole ≈ {want:.4})", W * H);
    assert!(
        (got - want).abs() < 2.0,
        "fill area {got:.4} must be outer−hole ≈ {want:.4}, not the un-subtracted \
         {:.1} (the circle is tessellated, hence the tolerance)",
        W * H
    );
    assert!(
        got < W * H - 40.0,
        "sanity: the fill is clearly smaller than the un-subtracted rectangle"
    );
    assert_eq!(
        tris.holes_subtracted, 1,
        "annulus fill proves its required hole was removed"
    );
    let disk_area = fill_area(disk.preview_triangles.as_ref().expect("disk fill"));
    assert!(
        (disk_area - std::f64::consts::PI * R * R).abs() < 2.0,
        "inner disk is independently selectable: {disk_area}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) Ring topology — exactly two single-use loops reach a consumer.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_fill_exposes_exactly_the_outer_and_hole_boundaries() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let regions = build_and_finish(&mut rt, SketchId(Uuid::from_u128(0x9E01)), true).await;
    let tris = regions
        .iter()
        .find(|region| !region.holes.is_empty())
        .expect("annulus cell")
        .preview_triangles
        .as_ref()
        .expect("preview fill present");

    // Every vertex sits on either the rectangle or the circle; the boundary is
    // therefore exactly (outer vertices + hole vertices) edges. Any extra
    // single-use edge would be a bridge that leaked out as a boundary — which is
    // what `prismPreview.profileFromRegion` would turn into a phantom wall.
    let verts = tris.positions.len() / 2;
    assert_eq!(
        boundary_edge_count(tris),
        verts,
        "a triangulated annulus has exactly one boundary edge per vertex \
         (outer loop + hole loop, closed); bridges must be interior"
    );
    assert!(
        verts > 4,
        "the hole contributes its own vertices ({verts} total)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) Control — no hole, full area, single loop.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_rectangle_still_fills_completely() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let regions = build_and_finish(&mut rt, SketchId(Uuid::from_u128(0x9E02)), false).await;
    assert_eq!(regions.len(), 1, "one closed loop ⇒ one region");
    assert!(regions[0].holes.is_empty(), "no holes on a plain rectangle");

    let tris = regions[0]
        .preview_triangles
        .as_ref()
        .expect("preview fill present");
    assert!(
        (fill_area(tris) - W * H).abs() < 1e-6,
        "the un-holed rectangle still fills to its full {} area",
        W * H
    );
    assert_eq!(
        boundary_edge_count(tris),
        4,
        "a plain rectangle exposes exactly its four boundary edges"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-OPS W2 — a sketch placed on a model FACE.
//
// `SketchAttachment::HostFace` has been typed in Rust AND mirrored in C++ since
// M1, but nothing ever CONSTRUCTED one, so every sketch that reached the worker
// was a world XY/XZ/YZ plane — which caps the app at single-sketch parts. No
// worker change was needed to fix that: `plane_kind_str` already maps a hostFace
// attachment to wire kind `"custom"` and `WireSketch::parse_plane` has always
// accepted an arbitrary custom basis. These prove the whole path really works:
// the custom-basis sketch solves, its regions derive, and an extrude off it lands
// where the frame says it should.
// ─────────────────────────────────────────────────────────────────────────────

/// The sketch plane a face-hosted sketch freezes, for a face whose normal is +Z
/// at height `z` — exactly what `plane_from_point_normal` produces (that rule's
/// own lock tests live in `onecad-core sketch::plane`).
fn top_face_plane(z: f64) -> onecad_core::sketch::SketchPlane {
    onecad_core::sketch::plane_from_point_normal(
        onecad_core::math::Vec3::new_unchecked(0.0, 0.0, z),
        onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 1.0),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_custom_basis_sketch_solves_and_derives_regions() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // A sketch on a "face" 10mm up — the frame a real top-face pick would freeze.
    let plane = top_face_plane(10.0);
    let sid = SketchId(Uuid::from_u128(0x9E10));
    let mut sk = Sketch::on_world_plane(sid, "OnFace", WorldPlane::XY);
    sk.plane = plane;
    sk.attachment = onecad_core::sketch::SketchAttachment::HostFace {
        // The attachment's face ref is not consumed by the worker (the wire
        // carries only the resolved basis), so an empty ref is the honest
        // fixture here — this test is about the FRAME, not the binding.
        face: onecad_core::document::refs::ElementRef {
            primary: None,
            intent: None,
            anchor: None,
            extra: Default::default(),
        },
        projected_boundary_version: 0,
    };

    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch on a host face");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    let built = rect_sketch(sid, false);
    let upsert = rt
        .sketch_upsert(sid, edit_ops(&built))
        .await
        .expect("sketch_upsert on a custom-basis sketch");
    eprintln!("custom-basis sketch: dof={}", upsert.dof);

    let regions = rt.finish_sketch(sid).await.expect("finish_sketch").regions;
    assert_eq!(
        regions.len(),
        1,
        "the rectangle closes into one region on a custom basis exactly as on XY"
    );
    let tris = regions[0]
        .preview_triangles
        .as_ref()
        .expect("the custom-basis region carries a fill");
    // Region geometry is PLANE-LOCAL (u,v), so it is identical to the XY case —
    // the basis only decides where that plane sits in the world.
    assert!(
        (fill_area(tris) - W * H).abs() < 1e-6,
        "plane-local region area is basis-independent ({} expected)",
        W * H
    );
    eprintln!(
        "custom-basis sketch PASS: 1 region, area {}",
        fill_area(tris)
    );
    wm.shutdown().await;
}

/// The frozen basis is what the geometry actually uses: an extrude off a sketch
/// hosted 10mm up lands 10mm up, not at the origin.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extrude_off_a_face_hosted_sketch_lands_on_that_frame() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    const HOST_Z: f64 = 10.0;
    let sid = SketchId(Uuid::from_u128(0x9E11));
    let mut sk = Sketch::on_world_plane(sid, "OnFace", WorldPlane::XY);
    sk.plane = top_face_plane(HOST_Z);
    sk.attachment = onecad_core::sketch::SketchAttachment::HostFace {
        // The attachment's face ref is not consumed by the worker (the wire
        // carries only the resolved basis), so an empty ref is the honest
        // fixture here — this test is about the FRAME, not the binding.
        face: onecad_core::document::refs::ElementRef {
            primary: None,
            intent: None,
            anchor: None,
            extra: Default::default(),
        },
        projected_boundary_version: 0,
    };
    rt.apply(EditCommand::AddSketch { sketch: sk.clone() })
        .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&rect_sketch(sid, false)))
        .await
        .expect("sketch_upsert");
    rt.finish_sketch(sid).await.expect("finish_sketch");

    // Extrude the profile 5mm along the frame's normal.
    let rec = RecordId(Uuid::from_u128(0x9E12));
    let params = onecad_core::document::record::ExtrudeParams {
        profile: Some(onecad_core::document::refs::SketchRegionRef {
            sketch: sid,
            region: RegionId::new(""),
            region_identity_version: None,
            extra: Default::default(),
        }),
        distance: onecad_core::document::variables::Scalar::new(5.0),
        draft_angle_deg: onecad_core::document::variables::Scalar::new(0.0),
        mode: onecad_core::document::record::ExtrudeMode::Blind,
        boolean_mode: onecad_core::document::record::BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: onecad_core::document::record::ExtrudeMode::Blind,
        distance2: onecad_core::document::variables::Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    };
    // The Sketch op must precede the extrude in the plan (the worker resolves a
    // profile only from sketches materialized in the same plan).
    let (_p, entities, constraints) =
        onecad_lib::worker::wire::sketch_wire(&rect_sketch(sid, false));
    let sketch_params = onecad_core::document::record::SketchOpParams {
        sketch: sid,
        plane: onecad_core::document::record::SketchPlaneRef {
            kind: onecad_core::document::record::PlaneKind::Custom,
            origin: sk.plane.origin,
            x_axis: sk.plane.x_axis,
            y_axis: sk.plane.y_axis,
            normal: sk.plane.normal,
            extra: Default::default(),
        },
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    rt.apply(EditCommand::AddOperation {
        record: onecad_core::document::record::OperationRecord::new(
            RecordId(Uuid::from_u128(0x9E13)),
            0,
            "Sketch",
            onecad_core::document::record::Operation::Known(
                onecad_core::document::record::KnownOperation::Sketch(sketch_params),
            ),
        ),
        at_cursor: true,
    })
    .expect("AddOperation sketch");
    rt.apply(EditCommand::AddOperation {
        record: onecad_core::document::record::OperationRecord::new(
            rec,
            0,
            "Extrude",
            onecad_core::document::record::Operation::Known(
                onecad_core::document::record::KnownOperation::Extrude(params),
            ),
        ),
        at_cursor: true,
    })
    .expect("AddOperation extrude");

    let report = rt
        .run_regen(
            onecad_core::regen::RegenRequest::ToEnd { from: 0 },
            onecad_core::regen::CancelToken::new(),
        )
        .await;
    let onecad_core::regen::Outcome::Published(_snap) = &report.outcome else {
        panic!("face-hosted extrude did not publish: {:?}", report.outcome);
    };

    let body = BodyId(Uuid::from_u128(0x9E13_u128));
    let _ = body;
    // The body id is worker-minted from the EXTRUDE record (D1 `body_<opId>`).
    let mesh = rt
        .get_mesh(
            BodyId(Uuid::from_u128(0x9E12)),
            onecad_core::regen::Lod::Coarse,
            None,
        )
        .await
        .expect("fetch the face-hosted extrude's mesh");
    let view = onecad_protocol::mesh::validate_mesh_blob(&mesh).expect("MESH1 validates");
    // THE assertion: the solid sits on the hosted frame (z ∈ [10, 15]), not at the
    // world origin. A dropped/ignored custom basis would put it at z ∈ [0, 5].
    let zmin = f64::from(view.bbox_min[2]);
    let zmax = f64::from(view.bbox_max[2]);
    eprintln!("face-hosted extrude bbox z = [{zmin}, {zmax}] (host z = {HOST_Z})");
    assert!(
        (zmin - HOST_Z).abs() < 1e-3 && (zmax - (HOST_Z + 5.0)).abs() < 1e-3,
        "the extrude must sit on the hosted frame z ∈ [{HOST_Z}, {}], got [{zmin}, {zmax}]",
        HOST_Z + 5.0
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Mid-gesture solver-cache clobber guard, against the REAL worker.
//
// `prepare_sketch_regions`/`drive()` re-upserts the sketch into the worker's
// solver cache to derive fresh regions. Mid-drag, that upsert would push the
// PRE-DRAG sketch back into the same cache `BeginGesture`/`SolveDrag` are
// reading from, so the next `SolveDrag` reply applies onto stale geometry
// (drag snaps back / diverges). Covered here against the real OCCT/PlaneGCS
// worker (not just the FakeBackend unit lane in document_runtime/tests.rs).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_gesture_refuses_region_snapshot_then_unblocks_after_end() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x9E20));
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(sid, false),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    rt.begin_gesture(sid, eid(P0), GestureTarget::point(eid(P0)))
        .await
        .expect("begin_gesture");

    let err = match rt.prepare_sketch_regions(sid) {
        Err(e) => e,
        Ok(_) => panic!("getSketchRegions must refuse while the drag is live"),
    };
    assert!(
        matches!(
            err,
            EngineError::OpFailed {
                recoverable: true,
                ..
            }
        ),
        "must be a recoverable op failure, got {err:?}"
    );

    // Pointer-up: the gesture ends, so a region query is unblocked again.
    rt.end_gesture(None).await.expect("end_gesture");
    let regions = rt
        .prepare_sketch_regions(sid)
        .expect("gesture ended ⇒ prepare unblocked")
        .drive()
        .await
        .expect("drive succeeds")
        .regions;
    assert_eq!(regions.len(), 1, "the rectangle closes into one region");

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finish_sketch_mid_gesture_clears_it_so_regions_succeed() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x9E21));
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(sid, false),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    rt.begin_gesture(sid, eid(P0), GestureTarget::point(eid(P0)))
        .await
        .expect("begin_gesture");

    // The Enter/E finish handoff: the frontend's pointer-up cancel lost the
    // race, so finish_sketch runs while the gesture is still open. It must
    // clear the dangling gesture (same as cancel_sketch) rather than leave it
    // to block every future region query on this sketch.
    let regions = rt
        .finish_sketch(sid)
        .await
        .expect("finish_sketch mid-gesture")
        .regions;
    assert_eq!(regions.len(), 1, "the rectangle closes into one region");

    let regions_again = rt
        .prepare_sketch_regions(sid)
        .expect("finish cleared the gesture ⇒ prepare unblocked")
        .drive()
        .await
        .expect("drive succeeds")
        .regions;
    assert_eq!(regions_again.len(), 1);

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// SP-0 D3 — the OTHER direction of the same race, which the gesture-refusal
// above could not close.
//
// `prepare_sketch_regions` captures its sketch under the runtime lock and then
// `drive()`s it WITHOUT the lock. The refusal above is a check-then-act: it only
// looks at `active_gesture` at prepare time, so a `beginGesture` that lands
// during the unlocked drive sails through, and the drive's re-upsert then lands
// on a worker whose gesture is already open. The `Claim` held by
// `PreparedSketchRegions` for its whole lifetime closes that window BY
// CONSTRUCTION.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_region_query_in_flight_refuses_a_new_drag_on_the_same_sketch() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x9E40));
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(sid, false),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");

    // Prepared but NOT yet driven — exactly the window `get_sketch_regions`
    // leaves open between releasing the runtime lock and the worker round-trip.
    let prepared = rt
        .prepare_sketch_regions(sid)
        .expect("no gesture yet ⇒ prepare succeeds");

    let err = rt
        .begin_gesture(sid, eid(P0), GestureTarget::point(eid(P0)))
        .await
        .expect_err("a region query holding the solver lane must refuse a new drag");
    assert!(
        matches!(
            err,
            EngineError::OpFailed {
                recoverable: true,
                ..
            }
        ),
        "must be a recoverable op failure, got {err:?}"
    );
    match &err {
        EngineError::OpFailed { message, .. } => {
            assert!(message.contains(&sid.to_string()), "{message}");
            assert!(message.contains("region query"), "{message}");
        }
        other => panic!("expected OpFailed, got {other:?}"),
    }

    // Driving consumes the query, which drops its claim — the drag is free again.
    let regions = prepared.drive().await.expect("drive succeeds").regions;
    assert_eq!(regions.len(), 1, "the rectangle closes into one region");

    rt.begin_gesture(sid, eid(P0), GestureTarget::point(eid(P0)))
        .await
        .expect("the region query finished ⇒ the drag opens");
    rt.end_gesture(None).await.expect("end_gesture");

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_region_query_on_a_different_sketch_never_blocks_a_drag() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid_a = SketchId(Uuid::from_u128(0x9E41));
    let sid_b = SketchId(Uuid::from_u128(0x9E42));
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(sid_a, false),
    })
    .expect("AddSketch A");
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(sid_b, false),
    })
    .expect("AddSketch B");
    rt.enter_sketch(sid_a).await.expect("enter_sketch A");

    // The claim is per SKETCH, not a global solver mutex: B's query and A's drag
    // touch disjoint entries in the worker's `SketchStore`.
    let prepared_b = rt
        .prepare_sketch_regions(sid_b)
        .expect("prepare B succeeds");
    rt.begin_gesture(sid_a, eid(P0), GestureTarget::point(eid(P0)))
        .await
        .expect("B's region query must not block A's drag");

    let regions_b = prepared_b.drive().await.expect("drive B succeeds").regions;
    assert_eq!(regions_b.len(), 1);
    rt.end_gesture(None).await.expect("end_gesture");

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// W3 — ELLIPSE regions. Before this the whole chain was closed by ONE line in
// the worker's `WireSketch::translate` ("unsupported entity type 'Ellipse'"),
// even though `LoopDetector` has always tessellated ellipses and `FaceBuilder`
// has always built a true `Geom_Ellipse` edge for them. Rust's `wire_entity`
// skipped the variant to match. Both are wired now.
//
// Oracle: corpus/cases/i_multiregion_loop_detection.json case `regions_ellipse`
// — center (0,0), rx 6, ry 3, rotation 0.25 ⇒ 1 region, 0 holes, area > 50
// (provenance OneCAD-CPP tests/prototypes/proto_loop_detector.cpp:80-94, which
// asserts only `area() > 50.0`; π·6·3 = 56.549 is the analytic value).
// ─────────────────────────────────────────────────────────────────────────────

const PE: u128 = 0x120; // ellipse centre
const ELL: u128 = 0x400;
const MAJOR_R: f64 = 6.0;
const MINOR_R: f64 = 3.0;
const ROTATION: f64 = 0.25; // radians on the wire

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_standalone_ellipse_publishes_one_region_matching_the_corpus_oracle() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x9E30));
    let mut sk = Sketch::on_world_plane(sid, "Ellipse", WorldPlane::XY);
    point(&mut sk, PE, 0.0, 0.0);
    sk.add_entity(
        SketchEntity::ellipse(eid(ELL), eid(PE), MAJOR_R, MINOR_R, ROTATION, false)
            .expect("ellipse"),
    )
    .unwrap();

    rt.apply(EditCommand::AddSketch {
        sketch: Sketch::on_world_plane(sid, "Ellipse", WorldPlane::XY),
    })
    .expect("AddSketch");
    rt.enter_sketch(sid).await.expect("enter_sketch");
    let upsert = rt
        .sketch_upsert(sid, edit_ops(&sk))
        .await
        .expect("an ellipse now survives wire translation");
    // SCHEMA §7.4 deviation: the solver lane cannot represent an ellipse, so DOF
    // is counted naively. 7 = the core centre Point (2, sent as its own wire
    // `Point` entity) + the centre the worker mints from the inlined `center`
    // (2) + the ellipse's own shape DOF (3). The double-minted centre is the
    // EXISTING Circle/Arc behavior verbatim (`centerRef` exists precisely so the
    // frontend can re-own the real one) — the ellipse does not introduce it.
    assert_eq!(
        upsert.dof, 7,
        "naive DOF for a free ellipse authored through core"
    );

    let regions = rt.finish_sketch(sid).await.expect("finish_sketch").regions;
    assert_eq!(
        regions.len(),
        1,
        "a standalone closed ellipse is exactly ONE selectable cell, got {regions:#?}"
    );
    assert!(regions[0].holes.is_empty(), "the ellipse cell has no holes");

    let tris = regions[0]
        .preview_triangles
        .as_ref()
        .expect("the ellipse region carries a preview fill");
    let got = fill_area(tris);
    let analytic = std::f64::consts::PI * MAJOR_R * MINOR_R; // 56.549
    eprintln!("ellipse fill area = {got:.4} (analytic {analytic:.4})");
    assert!(got > 50.0, "corpus oracle: area > 50.0, got {got:.4}");
    assert!(
        (got - analytic).abs() / analytic < 0.01,
        "fill area {got:.4} within 1% of π·a·b = {analytic:.4} \
         (the fill is a tessellation, hence the band)"
    );

    wm.shutdown().await;
}
