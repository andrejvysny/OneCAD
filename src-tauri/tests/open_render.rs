//! SAVE/OPEN render gate (user-reported): reopening a saved `.onecad` must make the
//! saved bodies fetchable for the viewport — the exact `open_document` command chain.
//!
//! The reported defect: save works, reopen shows the full tree, but NO body ever
//! renders. Every prior worker-backed test drives `DocumentRuntime` directly; none
//! exercised the app-glue chain the real open path uses:
//!
//! ```text
//! open_document  =  DocumentRuntime::open (fresh worker, api/mod.rs:76-77)
//!                 → projection (pre-regen, persisted bodies seeded)
//!                 → sched.request(ToEnd{from:0})            (api/mod.rs:87-89)
//!                 → production driver → publish → emit document-changed
//!                 → frontend get_mesh(bodyId, coarse, None)  (api/mod.rs:506)
//! ```
//!
//! * `saved_document_reopens_and_serves_meshes` — build + save through the real
//!   save path (checkpoint-at-head + `rt.save`, as `save_document` does), then
//!   reopen on a FRESH worker (the app spawns a new backend per open) and drive
//!   the open-regen through the REAL scheduler/driver. Asserts, in order:
//!   1. the pre-regen projection already lists the saved body (tree parity);
//!   2. `get_mesh` before the first publish is a **HIT**, served from the mesh
//!      caches the explicit save embedded in the container — and the projection
//!      says `geometrySource == "cached"` so the UI can label it honestly;
//!   3. the open-regen publishes and its `document-changed` names the body;
//!   4. `get_mesh` after the publish returns a non-empty MESH1 blob (magic-checked),
//!      at the LIVE generation, with `geometrySource == "live"`.
//!
//!   **Assertion 2 used to pin the opposite** (a documented pre-publish MISS, the
//!   window the frontend retried around). That window is what the container mesh
//!   caches exist to close: a reopened document now paints last-saved geometry
//!   immediately instead of showing an empty viewport for the whole from-0 regen.
//!   The miss survives only where there is nothing to serve —
//!   `container_without_mesh_caches_still_misses_before_the_publish` pins that for
//!   a legacy / autosave-shaped container, so the frontend's retry path stays
//!   exercised.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary (CI). A missing worker is a quiet local-dev skip.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::{watch, Mutex};

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, SnapshotId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{GeometryEngine, Lod, RegenRequest, RegenScheduler, SchedulerHandle};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport, SaveCaches};
use onecad_lib::dto::{DocumentChange, DocumentProjection};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};
use onecad_lib::{regen_driver_with_emitter, RegenEmitter};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors scheduler_commit.rs)
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

/// A [`MeshProvider`] decorator that counts `Tessellate` pulls. The whole point of
/// the inline `artifacts.tessellate` rider is that a post-regen `get_mesh` needs
/// ZERO of them — the worker already tessellated every prepared body and shipped
/// the MESH1 blobs in the `ExecutePlan` terminal's binary tail.
struct CountingMeshes {
    inner: Arc<dyn MeshProvider>,
    fetches: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl MeshProvider for CountingMeshes {
    async fn fetch_mesh(
        &self,
        body: BodyId,
        lod: Lod,
        snapshot: SnapshotId,
    ) -> Result<Vec<u8>, onecad_core::regen::EngineError> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.inner.fetch_mesh(body, lod, snapshot).await
    }
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

const SKETCH_A: u128 = 0xA00;
const EXTRUDE_A: u128 = 0xA01;

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

/// A fully-constrained rectangle (identical to `scheduler_commit::rect_sketch`).
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
            region: RegionId::new(""), // empty ⇒ the worker's V1 first-region fallback
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
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    )
}

type Runtime = Arc<Mutex<Option<DocumentRuntime>>>;
type Emit = (String, Option<DocumentChange>, DocumentProjection);

/// The exact `lib.rs` wiring over an already-constructed runtime.
async fn wire_with(runtime: Runtime) -> (Runtime, SchedulerHandle, UnboundedReceiver<Emit>) {
    let (tx, rx) = unbounded_channel::<Emit>();
    let emit: RegenEmitter = Arc::new(
        move |report: &RegenReport, projection: &DocumentProjection| {
            let _ = tx.send((
                report.outcome_str().to_string(),
                report.document_change(),
                projection.clone(),
            ));
        },
    );
    let autosave_tick = Arc::new(watch::channel(0u64).0);
    let driver = regen_driver_with_emitter(runtime.clone(), emit, autosave_tick);
    let (scheduler, sched) = RegenScheduler::new(driver);
    tokio::spawn(scheduler.run());
    (runtime, sched, rx)
}

async fn recv(rx: &mut UnboundedReceiver<Emit>) -> Emit {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("regen completion emitted within 15s")
        .expect("emitter channel stayed open")
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-08-02T00:00:00Z".into(),
        modified: "2026-08-02T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate: save → fresh-worker reopen → open-regen publishes → get_mesh serves
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_document_reopens_and_serves_meshes() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };

    // ── Session 1: author sketch+extrude via the production scheduler, save. ──
    let wm1 = spawn_worker(bin.clone()).await;
    let runtime: Runtime = Arc::new(Mutex::new(Some(runtime_over(&wm1))));
    let (runtime, sched1, mut rx1) = wire_with(runtime).await;

    let outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        let sa = SketchId(Uuid::from_u128(0xA));
        rt.apply(EditCommand::AddOperation {
            record: sketch_record(SKETCH_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
            at_cursor: false,
        })
        .expect("add sketch op");
        rt.apply(EditCommand::AddOperation {
            record: extrude_record(EXTRUDE_A, sa, 25.0),
            at_cursor: false,
        })
        .expect("add extrude op")
    };
    sched1.handle(&outcome);
    let (s, change, _) = recv(&mut rx1).await;
    assert_eq!(s, "published", "session-1 commit published");
    let body_id_str = change
        .expect("published change")
        .changed_bodies
        .first()
        .expect("one body")
        .body_id
        .clone();

    // The REAL save path: checkpoint-at-head + save (api::save_document :126-129).
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reopen.onecad");
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.take_checkpoint_at_head().await;
        rt.save(&path, save_meta()).expect("save");
    }
    sched1.shutdown();
    wm1.shutdown().await;

    // ── Session 2: the EXACT open_document chain on a FRESH worker. ──────────
    let wm2 = spawn_worker(bin).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm2.clone());
    let fetches = Arc::new(AtomicUsize::new(0));
    let meshes: Arc<dyn MeshProvider> = Arc::new(CountingMeshes {
        inner: Arc::new(wm2.clone()),
        fetches: fetches.clone(),
    });
    let solver: Arc<dyn SolverEngine> = Arc::new(wm2.clone());
    let rt2 = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");

    // (1) Tree parity: the pre-regen projection already lists the saved body.
    let pre = rt2.projection();
    assert!(
        pre.bodies.contains_key(&body_id_str),
        "pre-regen projection lists the saved body {body_id_str}; got {:?}",
        pre.bodies.keys().collect::<Vec<_>>()
    );

    let body = BodyId::from_str(&body_id_str).expect("projection body id parses as BodyId");

    assert_eq!(
        pre.geometry_source, "cached",
        "the pre-regen projection must say the geometry is LAST-SAVED, not live"
    );

    let runtime2: Runtime = Arc::new(Mutex::new(Some(rt2)));

    // (2) The pre-publish window is now SERVED: the explicit save embedded this
    //     body's coarse mesh, so the viewport paints before the regen finishes.
    let cached_bytes = {
        let mut guard = runtime2.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.get_mesh(body, Lod::Fine, None)
            .await
            .expect("the container's mesh cache serves the pre-publish window")
    };
    assert_mesh1(&cached_bytes, "container mesh cache");
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        0,
        "the pre-publish hit comes from the container — no provider call, and no worker \
         round-trip is even possible before the first publish"
    );
    // A caller that PINS a generation still misses: these bytes belong to no
    // published snapshot, and generations are minted from 1.
    {
        let mut guard = runtime2.lock().await;
        let rt = guard.as_mut().expect("open");
        assert!(
            rt.get_mesh(body, Lod::Fine, Some(1)).await.is_none(),
            "a generation-pinned request must never be answered with container bytes"
        );
    }

    // (3) The open-regen: exactly api::open_document :87-89.
    let (runtime2, sched2, mut rx2) = wire_with(runtime2).await;
    sched2.request(RegenRequest::ToEnd { from: 0 });
    let (s, change, proj) = recv(&mut rx2).await;
    assert_eq!(s, "published", "open-regen publishes the replayed timeline");
    let change = change.expect("published open-regen carries a document_change");
    assert!(
        change
            .changed_bodies
            .iter()
            .any(|b| b.body_id == body_id_str),
        "document-changed names the saved body {body_id_str}; got {:?}",
        change.changed_bodies
    );
    assert!(
        proj.bodies.contains_key(&body_id_str),
        "post-regen projection still lists the body"
    );
    assert_eq!(
        proj.geometry_source, "live",
        "a publish flips the projection off the cached-geometry label"
    );

    // (4) The viewport fetch: a non-empty MESH1 blob for that body — served from the
    //     cache the open-regen SEEDED, so no Tessellate round-trip happens at all.
    let bytes = {
        let mut guard = runtime2.lock().await;
        let rt = guard.as_mut().expect("open");
        rt.get_mesh(body, Lod::Fine, None)
            .await
            .expect("get_mesh serves the replayed body's mesh")
    };
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        0,
        "the open-regen's inline artifacts must make the FIRST get_mesh a cache hit — \
         a Tessellate pull here means the body was tessellated twice"
    );
    assert_mesh1(&bytes, "post-publish get_mesh");

    sched2.shutdown();
    wm2.shutdown().await;
    eprintln!(
        "OPEN-RENDER PASS: save → fresh-worker reopen → container cache served {} bytes \
         BEFORE the regen → open-regen published → get_mesh served {} bytes with {} \
         Tessellate pulls",
        cached_bytes.len(),
        bytes.len(),
        fetches.load(Ordering::SeqCst)
    );
}

/// The complement of assertion 2: a container with **no** mesh caches — a legacy
/// file, or one written by the autosave lane ([`SaveCaches::none`]) — still misses
/// before the first publish. The frontend's pre-publish retry path is therefore
/// still reachable and must stay correct; the caches close the window, they do not
/// abolish it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn container_without_mesh_caches_still_misses_before_the_publish() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };

    let wm1 = spawn_worker(bin.clone()).await;
    let runtime: Runtime = Arc::new(Mutex::new(Some(runtime_over(&wm1))));
    let (runtime, sched1, mut rx1) = wire_with(runtime).await;

    let outcome = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        let sa = SketchId(Uuid::from_u128(0xB));
        rt.apply(EditCommand::AddOperation {
            record: sketch_record(SKETCH_A, &rect_sketch(sa, 0x2000, 0.0, 0.0, 30.0, 15.0)),
            at_cursor: false,
        })
        .expect("add sketch op");
        rt.apply(EditCommand::AddOperation {
            record: extrude_record(EXTRUDE_A, sa, 12.0),
            at_cursor: false,
        })
        .expect("add extrude op")
    };
    sched1.handle(&outcome);
    let (s, change, _) = recv(&mut rx1).await;
    assert_eq!(s, "published", "commit published");
    let body_id_str = change
        .expect("published change")
        .changed_bodies
        .first()
        .expect("one body")
        .body_id
        .clone();

    // The AUTOSAVE lane's payload: document + imports, no cache section at all.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("no-caches.onecad");
    {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().expect("open");
        let payload = rt.build_save_payload(save_meta(), SaveCaches::none());
        DocumentRuntime::write_payload(&path, &payload).expect("write");
    }
    sched1.shutdown();
    wm1.shutdown().await;

    let names = zip_entry_names(&path);
    assert!(
        !names.iter().any(|n| n.starts_with("meshes/")),
        "SaveCaches::none() must write no mesh section, got {names:?}"
    );

    let wm2 = spawn_worker(bin).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm2.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm2.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm2.clone());
    let mut rt2 = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");

    let body = BodyId::from_str(&body_id_str).expect("body id parses");
    assert!(
        rt2.projection().bodies.contains_key(&body_id_str),
        "the tree still renders from the persisted body registry"
    );
    assert_eq!(
        rt2.projection().geometry_source,
        "none",
        "no snapshot and no cached meshes ⇒ nothing to paint"
    );
    assert!(
        rt2.get_mesh(body, Lod::Fine, None).await.is_none(),
        "without a mesh cache section the pre-publish window is still a MISS"
    );

    wm2.shutdown().await;
}

/// Asserts `bytes` is a non-trivial blob leading with the MESH1 magic.
fn assert_mesh1(bytes: &[u8], what: &str) {
    assert!(
        bytes.len() > 8,
        "{what}: MESH1 blob is non-trivial ({} bytes)",
        bytes.len()
    );
    let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    assert_eq!(
        magic,
        0x4D45_5348,
        "{what}: blob leads with the MESH1 magic (LE u32; got {:?})",
        &bytes[0..4]
    );
}

/// Every non-directory entry name inside a container.
fn zip_entry_names(path: &std::path::Path) -> Vec<String> {
    let file = std::fs::File::open(path).expect("open container");
    let mut zip = zip::ZipArchive::new(file).expect("read container zip");
    (0..zip.len())
        .filter_map(|i| {
            let f = zip.by_index(i).ok()?;
            (!f.is_dir()).then(|| f.name().to_string())
        })
        .collect()
}
