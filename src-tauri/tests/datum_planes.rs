//! Datum offset planes against the REAL C++ OCCT/PlaneGCS worker (DATUM W1).
//!
//! Datums are **core-owned state that never crosses the OCW1 wire** (SCHEMA §7.3
//! plus the 2026-07-26 changelog entry): the worker only ever sees the RESOLVED
//! basis, arriving as a `plane.kind: "custom"` sketch plane. So the only way to
//! prove the datum path is end-to-end geometry — the solid an extrude produces
//! off a datum-hosted sketch has to sit on the datum's frame.
//!
//! The chain under test:
//!
//! 1. `AddDatumPlane` — the session RESOLVES the parametric definition at
//!    creation and overwrites whatever frame the caller sent.
//! 2. `AddSketch` with `SketchAttachment::Datum` — the session STAMPS the
//!    sketch's plane from the datum's resolved frame (never the FE round-trip).
//! 3. Sketch op + Extrude in one plan — the worker gets `custom` + the stamped
//!    basis and builds the profile there.
//! 4. MESH1 bbox — the published solid sits at z ∈ [10, 15], not [0, 5]. A
//!    dropped datum frame (sketch left on `Sketch::new`'s XY placeholder) lands
//!    at the origin and this fails.
//!
//! Harness mirrors `sketch_regions.rs` (real worker via `ONECAD_WORKER_PATH`,
//! else the dev-tree fallback; `ONECAD_REQUIRE_WORKER=1` hard-fails a missing
//! binary).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::datum::DatumPlane;
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{BodyId, DatumPlaneId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::regen::{CancelToken, EngineError, GeometryEngine, Lod, Outcome, RegenRequest};
use onecad_core::sketch::{Sketch, SketchAttachment, SketchEntity};

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors sketch_regions.rs)
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

// ── fixture ──────────────────────────────────────────────────────────────────

const P0: u128 = 0x100;
const P1: u128 = 0x101;
const P2: u128 = 0x102;
const P3: u128 = 0x103;
const L0: u128 = 0x200;
const L1: u128 = 0x201;
const L2: u128 = 0x202;
const L3: u128 = 0x203;

const W: f64 = 40.0;
const H: f64 = 20.0;
/// The datum's offset from XY. XY's normal is +Z, so the frame lands at z = 10.
const DATUM_Z: f64 = 10.0;
/// Extrude distance along the frame normal.
const DEPTH: f64 = 5.0;

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

/// A closed 40×20 rectangle. `attachment` decides where its frame comes from —
/// the plane the session stamps is what actually reaches the kernel.
fn rect_sketch(sid: SketchId, attachment: SketchAttachment) -> Sketch {
    let mut sk = Sketch::new(sid, "Datum sketch", attachment);
    point(&mut sk, P0, 0.0, 0.0);
    point(&mut sk, P1, W, 0.0);
    point(&mut sk, P2, W, H);
    point(&mut sk, P3, 0.0, H);
    for (l, a, b) in [(L0, P0, P1), (L1, P1, P2), (L2, P2, P3), (L3, P3, P0)] {
        sk.add_entity(SketchEntity::line(eid(l), eid(a), eid(b), false))
            .unwrap();
    }
    sk
}

/// A `Sketch` timeline op carrying the sketch's (core-stamped) basis as a
/// `custom` plane — the wire form a datum-hosted sketch takes.
fn sketch_op(rec: RecordId, sk: &Sketch) -> OperationRecord {
    let (_p, entities, constraints) = onecad_lib::worker::wire::sketch_wire(sk);
    OperationRecord::new(
        rec,
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sk.id,
            plane: SketchPlaneRef {
                kind: PlaneKind::Custom,
                origin: sk.plane.origin,
                x_axis: sk.plane.x_axis,
                y_axis: sk.plane.y_axis,
                normal: sk.plane.normal,
                extra: Default::default(),
            },
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            extra: Default::default(),
        })),
    )
}

fn extrude_op(rec: RecordId, sid: SketchId, distance: f64) -> OperationRecord {
    OperationRecord::new(
        rec,
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            // Empty region id ⇒ first-region fallback (V1), which is the only
            // region a plain rectangle has.
            profile: Some(SketchRegionRef {
                sketch: sid,
                region: RegionId::new(""),
                extra: Default::default(),
            }),
            distance: Scalar::new(distance),
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
// THE proof: a datum-hosted extrude lands on the datum's frame.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extrude_off_a_datum_hosted_sketch_lands_on_the_datum_frame() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // 1. The datum. The caller sends the DEFINITION only; `resolved_plane` is
    //    left at the `offset_from_plane` default and the session overwrites it.
    let did = DatumPlaneId(Uuid::from_u128(0xDA10));
    rt.apply(EditCommand::AddDatumPlane {
        datum: DatumPlane::offset_from_plane(did, "Datum 1", "XY", DATUM_Z),
    })
    .expect("AddDatumPlane");

    // The projection is the frontend's only view of a datum — assert the resolved
    // frame reaches it, since the FE tool half will place its gizmo from exactly
    // this.
    let proj = rt.projection();
    let dto = proj
        .datums
        .get(&did.to_string())
        .expect("the datum reaches the projection");
    assert!(dto.resolved_valid, "an XY-based offset must resolve");
    assert_eq!(dto.kind, "OffsetFromPlane");
    assert_eq!(dto.base_plane_id, "XY");
    assert_eq!(dto.offset, DATUM_Z);
    assert_eq!(
        dto.plane.origin,
        [0.0, 0.0, DATUM_Z],
        "XY's normal is +Z, so the frame slid to z = {DATUM_Z}"
    );
    // The axes are XY's, carried verbatim (SCHEMA §7.3 non-standard basis).
    assert_eq!(dto.plane.x_axis, [0.0, 1.0, 0.0]);
    assert_eq!(dto.plane.y_axis, [-1.0, 0.0, 0.0]);

    // 2. A sketch attached to the datum. It is built with `Sketch::new`, whose
    //    non-world attachments default to the XY PLACEHOLDER at the origin — so
    //    if the core did not stamp the frame, everything below would land at z=0.
    let sid = SketchId(Uuid::from_u128(0xDA11));
    let authored = Sketch::new(sid, "Datum sketch", SketchAttachment::Datum { datum: did });
    assert_eq!(
        authored.plane.origin.z, 0.0,
        "fixture precondition: the AUTHORED sketch carries the origin placeholder"
    );
    rt.apply(EditCommand::AddSketch { sketch: authored })
        .expect("AddSketch on the datum");

    // The CORE stamped the plane — read it back through `getSketch`, which is
    // exactly what the frontend sees (the wire form of the stored sketch).
    let read_back = rt.get_sketch(sid).expect("getSketch");
    assert_eq!(
        read_back.plane["kind"], "custom",
        "a datum-hosted sketch sends a custom basis (SCHEMA §7.3)"
    );
    let stamped_plane: onecad_core::sketch::SketchPlane =
        serde_json::from_value(read_back.plane.clone()).expect("the wire plane parses");
    assert_eq!(
        stamped_plane.origin,
        onecad_core::math::Vec3::new_unchecked(0.0, 0.0, DATUM_Z),
        "the core must stamp the sketch frame FROM the datum, not trust the caller"
    );
    // The op's plane comes from the READ-BACK frame, never from the fixture.
    let mut stamped = rect_sketch(sid, SketchAttachment::Datum { datum: did });
    stamped.plane = stamped_plane;

    // 3. Solve + finish the sketch, then plan Sketch → Extrude.
    rt.enter_sketch(sid).await.expect("enter_sketch");
    rt.sketch_upsert(sid, edit_ops(&stamped))
        .await
        .expect("sketch_upsert");
    rt.finish_sketch(sid).await.expect("finish_sketch");

    let sketch_rec = RecordId(Uuid::from_u128(0xDA12));
    let extrude_rec = RecordId(Uuid::from_u128(0xDA13));
    rt.apply(EditCommand::AddOperation {
        record: sketch_op(sketch_rec, &stamped),
        at_cursor: true,
    })
    .expect("AddOperation sketch");
    rt.apply(EditCommand::AddOperation {
        record: extrude_op(extrude_rec, sid, DEPTH),
        at_cursor: true,
    })
    .expect("AddOperation extrude");

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(_) = &report.outcome else {
        panic!("datum-hosted extrude did not publish: {:?}", report.outcome);
    };

    // 4. THE assertion. The body id is worker-minted from the EXTRUDE record
    //    (decision D1, `body_<opId>`).
    let mesh = rt
        .get_mesh(BodyId(extrude_rec.0), Lod::Coarse, None)
        .await
        .expect("fetch the datum-hosted extrude's mesh");
    let view = onecad_protocol::mesh::validate_mesh_blob(&mesh).expect("MESH1 validates");
    let zmin = f64::from(view.bbox_min[2]);
    let zmax = f64::from(view.bbox_max[2]);
    eprintln!("datum-hosted extrude bbox z = [{zmin}, {zmax}] (datum z = {DATUM_Z})");
    assert!(
        (zmin - DATUM_Z).abs() < 1e-3 && (zmax - (DATUM_Z + DEPTH)).abs() < 1e-3,
        "the solid must sit on the datum frame z ∈ [{DATUM_Z}, {}], got [{zmin}, {zmax}] \
         (z ∈ [0, {DEPTH}] means the datum frame was dropped)",
        DATUM_Z + DEPTH
    );
    // In-plane extent is untouched by the offset (the axes were carried verbatim).
    let dx = f64::from(view.bbox_max[0]) - f64::from(view.bbox_min[0]);
    let dy = f64::from(view.bbox_max[1]) - f64::from(view.bbox_min[1]);
    assert!(
        ((dx - H).abs() < 1e-3 && (dy - W).abs() < 1e-3),
        "XY's basis maps sketch X→world Y and sketch Y→world −X, so a {W}×{H} \
         rectangle spans {H}×{W} in world XY; got {dx}×{dy}"
    );

    wm.shutdown().await;
}

/// A datum whose definition the core cannot resolve is stored but CANNOT host a
/// sketch — the rejection happens at `AddSketch`, before any worker traffic, so
/// no half-placed sketch ever reaches the kernel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unresolved_datum_cannot_host_a_sketch() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let did = DatumPlaneId(Uuid::from_u128(0xDA20));
    rt.apply(EditCommand::AddDatumPlane {
        datum: DatumPlane::offset_from_plane(did, "Bad", "NOT_A_PLANE", 3.0),
    })
    .expect("a lenient insert: an unresolvable definition still loads");
    let proj = rt.projection();
    assert!(!proj.datums[&did.to_string()].resolved_valid);

    let sid = SketchId(Uuid::from_u128(0xDA21));
    let err = rt
        .apply(EditCommand::AddSketch {
            sketch: rect_sketch(sid, SketchAttachment::Datum { datum: did }),
        })
        .expect_err("an unresolved datum must not host a sketch");
    assert!(
        err.to_string().contains("unresolved"),
        "the error must say why: {err}"
    );
    assert!(
        rt.projection().sketches.is_empty(),
        "nothing partially applied"
    );

    wm.shutdown().await;
}

/// `DeleteDatum` refuses while a sketch is attached, and succeeds once it is not
/// — the referenced-guard, proven through the app runtime (not just the core).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_datum_is_guarded_by_its_sketches() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let did = DatumPlaneId(Uuid::from_u128(0xDA30));
    rt.apply(EditCommand::AddDatumPlane {
        datum: DatumPlane::offset_from_plane(did, "Datum 1", "XY", DATUM_Z),
    })
    .expect("AddDatumPlane");
    let sid = SketchId(Uuid::from_u128(0xDA31));
    rt.apply(EditCommand::AddSketch {
        sketch: rect_sketch(sid, SketchAttachment::Datum { datum: did }),
    })
    .expect("AddSketch");

    // A DATUM-hosted sketch is not a FACE-hosted one: its projection row carries
    // no `hostFace`, so the frontend's double-click-a-face re-entry can never
    // mistake it for a sketch living on model geometry (SKETCH-ON-FACE W3).
    assert!(
        rt.projection().sketches[&sid.to_string()]
            .host_face
            .is_none(),
        "a datum-hosted sketch has no host FACE"
    );

    let err = rt
        .apply(EditCommand::DeleteDatum { datum: did })
        .expect_err("a referenced datum must not be deletable");
    assert!(
        err.to_string().contains(&sid.to_string()),
        "the error must NAME the referencing sketch: {err}"
    );
    assert!(rt.projection().datums.contains_key(&did.to_string()));

    rt.apply(EditCommand::DeleteSketch { sketch: sid })
        .expect("DeleteSketch");
    rt.apply(EditCommand::DeleteDatum { datum: did })
        .expect("an unreferenced datum deletes");
    assert!(rt.projection().datums.is_empty());

    wm.shutdown().await;
}

// Silence the unused-import lint when the worker is absent and every test bails
// early (`EngineError` is only named through the trait objects above).
const _: Option<EngineError> = None;
