//! Component Library WP-0.2 `PlaceComponent` integration gate against the REAL
//! C++ OCCT worker, driven through the app's [`DocumentRuntime`] exactly like
//! `hole_ops.rs`.
//!
//! Proves the op end to end (Rust core params → wire lowering → worker
//! dispatch → OCCT → published body) with the EXACT kernel volume from
//! `QueryMassProperties` (SCHEMA §7.5), not a mesh-chord approximation — the
//! same discipline `hole_ops.rs` follows.
//!
//! * `place_component_generator_source_mints_one_body` — a `generator`-source
//!   `PlaceComponent` (P0/WP-0.2's hardcoded M6 SHCS) mints exactly one NewBody
//!   at the exact analytic volume.
//! * `place_component_survives_save_and_a_fresh_worker_reopen` — save → reopen
//!   on a FRESH worker replays to the same volume (P0's "zero library
//!   dependency" framing starts here: nothing in this op reads a library root).
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing
//! binary skips cleanly (`ONECAD_REQUIRE_WORKER=1` makes that a hard failure).

use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    ComponentMate, ComponentParamValue, ComponentSourceRef, DetachComponentParams, FrozenPlacement,
    KnownOperation, MateKind, Operation, OperationRecord, PlaceComponentParams,
};
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ElementId, RecordId, TopoKey};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::Vec3;
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors hole_ops.rs)
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

fn open_over(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
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

/// The record's exact `opType` string, read through `rt.projection()`
/// (`DocumentRuntime` exposes no direct timeline accessor to test code).
fn feature_op_type(rt: &DocumentRuntime, record: RecordId) -> String {
    let id = record.to_string();
    rt.projection()
        .features
        .into_iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("no feature for record {id}"))
        .op_type
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-08-12T00:00:00Z".into(),
        modified: "2026-08-12T00:00:00Z".into(),
    }
}

/// Exact kernel volume of a body (`QueryMassProperties`, SCHEMA §7.5) — read
/// from the BRep, never re-derived from the tessellation.
async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

/// The hardcoded ISO 4762 M6x20 solid's exact analytic volume (see
/// `worker/tests/test_component_ops.cpp` — the C++ side pins the same
/// constant, so a divergence between the two shows up on both sides).
fn m6_shcs_volume() -> f64 {
    PI * 5.0 * 5.0 * 6.0 + PI * 3.0 * 3.0 * 20.0
}

fn place_component_record(rec: u128) -> OperationRecord {
    let mut generator_params = std::collections::BTreeMap::new();
    generator_params.insert(
        "thread".to_string(),
        ComponentParamValue::Text("M6".to_string()),
    );
    let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id: "onecad.std.iso4762".to_string(),
        component_version: "1.0.0".to_string(),
        component_revision: format!("sha256:{}", "0".repeat(64)),
        params: generator_params,
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: std::collections::BTreeMap::new(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Place Component", op)
}

fn detach_component_op() -> Operation {
    Operation::Known(KnownOperation::DetachComponent(DetachComponentParams {
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: std::collections::BTreeMap::new(),
            extra: Default::default(),
        },
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_component_generator_source_mints_one_body() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    add_op(&mut rt, place_component_record(0xc1));
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "place_component");
    assert_eq!(
        snap.bodies.len(),
        1,
        "PlaceComponent mints exactly one body"
    );
    let body = snap.bodies[0].body;

    let vol = exact_volume(&wm, body).await;
    assert!(
        (vol - m6_shcs_volume()).abs() < 1.0,
        "PlaceComponent volume: got {vol}, want {} (M6 SHCS head+shank)",
        m6_shcs_volume()
    );

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_component_survives_save_and_a_fresh_worker_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm1 = spawn_worker(bin.clone()).await;
    let mut rt = runtime_over(&wm1);
    add_op(&mut rt, place_component_record(0xc2));
    let report = regen_all(&mut rt).await;
    let body_before = published(&report, "place_component").bodies[0].body;
    let vol_before = exact_volume(&wm1, body_before).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("component.onecad");
    rt.save(&path, save_meta()).expect("save");
    wm1.shutdown().await;

    // A FRESH worker process — proves the op replays from the document alone,
    // with no dependency on anything but the saved container (spec §0
    // invariant #4: "geometry is always cached locally").
    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_over(&wm2, &path);
    let report2 = regen_all(&mut rt2).await;
    let snap2 = published(&report2, "reopen");
    assert_eq!(
        snap2.bodies.len(),
        1,
        "reopen still publishes exactly one body"
    );
    let body_after = snap2.bodies[0].body;
    let vol_after = exact_volume(&wm2, body_after).await;
    assert!(
        (vol_after - vol_before).abs() < 1e-6,
        "reopen volume {vol_after} != original {vol_before}"
    );

    wm2.shutdown().await;
}

/// WP-1.2: the sanctioned `PlaceComponent` → `DetachComponent` swap, end to
/// end through the REAL worker — not just the Rust-core `op_type_edit_allowed`
/// unit test. Same `RecordId`, same `BodyId`, same volume, before and after;
/// only the op's identity provenance (component_id/version/revision/mate)
/// disappears — spec §3.4's "inert provenance."
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_component_preserves_body_and_volume_across_the_swap() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let record_id = RecordId(Uuid::from_u128(0xc3));
    add_op(&mut rt, place_component_record(0xc3));
    let report1 = regen_all(&mut rt).await;
    let snap1 = published(&report1, "place_component");
    assert_eq!(snap1.bodies.len(), 1);
    let body_before = snap1.bodies[0].body;
    let vol_before = exact_volume(&wm, body_before).await;
    assert_eq!(feature_op_type(&rt, record_id), "PlaceComponent");

    rt.apply(EditCommand::UpdateOperationParams {
        record: record_id,
        op: detach_component_op(),
    })
    .expect("the sanctioned PlaceComponent -> DetachComponent swap is accepted");
    assert_eq!(
        feature_op_type(&rt, record_id),
        "DetachComponent",
        "the record's op type actually changed"
    );

    let report2 = regen_all(&mut rt).await;
    let snap2 = published(&report2, "detach_component");
    assert_eq!(snap2.bodies.len(), 1, "still exactly one body after detach");
    let body_after = snap2.bodies[0].body;
    assert_eq!(
        body_after, body_before,
        "same BodyId across the swap (same RecordId)"
    );

    let vol_after = exact_volume(&wm, body_after).await;
    assert!(
        (vol_after - vol_before).abs() < 1e-6,
        "detach volume {vol_after} != pre-detach volume {vol_before}"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// P3 WP-3.1: persistent mate re-seating on regen (spec §5.5), end to end
// through the REAL worker + DocumentRuntime, not just the worker ctest matrix
// (`worker/tests/test_component_mate_reseat.cpp`, which calls
// `execute_place_component` directly and so cannot see the Rust-side wiring
// this WP adds: `PlanStepEvent.mate_placement` parsing, `Scratch` buffering,
// `Timeline::set_place_component_placement`, `sync_mate_placements`, and —
// the REAL bug this test caught — `PlaceComponent`'s `mate` used to ride in
// the wire `inputs[]`, which the worker's generic pre-flight resolves
// BEFORE the op runs and treats a failure there as blocking, so an
// unresolvable mate silently published NOTHING rather than the component at
// its frozen `placement` (fixed: `wire.rs::wire_op_inputs` no longer emits
// an input for `mate`; `ComponentOp.cpp::resolve_mate_reseat` owns
// resolution entirely, in-process, non-blocking).
// ─────────────────────────────────────────────────────────────────────────────

/// Classifies `body`'s faces (topo keys `f:1..=8`, generous for a fused
/// two-cylinder SHCS) and returns the first cylindrical one near `radius_mm`
/// — the SHCS's exact face ordering after the head/shank boolean fuse isn't
/// guaranteed, so this searches rather than assumes an ordinal. Returns the
/// topo key, the axis origin, AND a real point ON the surface (origin
/// offset by `radius_mm` perpendicular to the axis) — the ladder's anchor
/// scoring wants a point a real pick would land on, not the axis location
/// (which sits `radius_mm` inside the material and under-scores the
/// `anchor` feature contribution).
async fn find_cylindrical_face(
    wm: &WorkerManager,
    body: BodyId,
    radius_mm: f64,
) -> (String, [f64; 3], [f64; 3]) {
    for i in 1..=8 {
        let key = format!("f:{i}");
        let Some(dto) = ElementQuery::classify_element_by_topo_key(wm, body, &key)
            .await
            .expect("ClassifyElement")
        else {
            continue;
        };
        if dto.surface_type != "cylinder" {
            continue;
        }
        let Some(frame) = &dto.frame else { continue };
        if (frame.radius.unwrap_or(0.0) - radius_mm).abs() < 1e-6 {
            let axis = frame.axis.unwrap_or([0.0, 0.0, 1.0]);
            // Any unit vector perpendicular to `axis` — this SHCS's axis is
            // always (0,0,±1), so (1,0,0) is never near-parallel to it.
            let perp = if axis[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let surface_point = [
                frame.origin[0] + perp[0] * radius_mm,
                frame.origin[1] + perp[1] * radius_mm,
                frame.origin[2] + perp[2] * radius_mm,
            ];
            return (key, frame.origin, surface_point);
        }
    }
    panic!("no cylindrical face at radius {radius_mm}mm found on {body:?} within f:1..=8");
}

/// Promotes ONE snapshot-scoped `TopoKey` into a Rust-minted `ElementId`
/// (`AcquireElementIds`), mirroring `offset_face.rs`'s own `promote` helper.
async fn promote(
    rt: &mut DocumentRuntime,
    snapshot: onecad_core::ids::SnapshotId,
    body: BodyId,
    topo_key: &str,
    anchor: &AnchorIntent,
) -> ElementId {
    let promoted = rt
        .promote_selection(
            snapshot,
            body,
            vec![(TopoKey::new(topo_key), Some(anchor.clone()))],
        )
        .await
        .expect("AcquireElementIds promotes the shank face at the head snapshot");
    ElementId::new(&promoted[0].element_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_component_mate_reseats_on_the_first_regen_when_authored_off_axis() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // The TARGET: a plain, unmated PlaceComponent. Its shank (radius 3mm,
    // the M6 shank) is a real cylindrical face this test mates against.
    add_op(&mut rt, place_component_record(0xd1));
    let report1 = regen_all(&mut rt).await;
    let snap1 = published(&report1, "target place");
    let target_body = snap1.bodies[0].body;
    let snapshot = snap1.id;

    let (shank_key, shank_origin, _shank_surface_point) =
        find_cylindrical_face(&wm, target_body, 3.0).await;
    // The ladder's anchor scoring is relative to the candidate face's own
    // CENTROID, not the cylinder's axis-parametrization origin: `frame.
    // origin` is the shank's BASE (z=-20, `place_component_record`'s
    // `kDefaultLengthMm=20` shank running DOWN from the origin placement),
    // but the trimmed face's real centroid sits at its midpoint (z=-10).
    // Anchoring near the actual centroid — a real cursor pick would land
    // near the middle of a long face far more often than exactly on its
    // rim — is what a live gesture's anchor evidence would look like.
    let shank_centroid_z = shank_origin[2] + 10.0; // length/2 of the M6 default shank
    let anchor = AnchorIntent {
        world_point: Vec3::new_unchecked(shank_origin[0] + 3.0, shank_origin[1], shank_centroid_z),
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let element_id = promote(&mut rt, snapshot, target_body, &shank_key, &anchor).await;

    // The MATED component: `concentric` to the shank, authored deliberately
    // off-axis (translate.y shifted +5mm from the shank's actual axis) — the
    // first regen must reseat it onto the real axis.
    let mate_record = RecordId(Uuid::from_u128(0xd2));
    let mut mated = place_component_record(0xd2);
    let Operation::Known(KnownOperation::PlaceComponent(params)) = &mut mated.op else {
        unreachable!()
    };
    params.mate = Some(ComponentMate {
        self_attachment: "shankAxis".to_string(),
        target: ElementRef {
            primary: Some(PrimaryRef {
                body: target_body,
                element: element_id,
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(anchor),
            extra: Default::default(),
        },
        kind: MateKind::Concentric,
        flipped: false,
        extra: Default::default(),
    });
    params.placement.translate[0] = Scalar::new(shank_origin[0]);
    params.placement.translate[1] = Scalar::new(shank_origin[1] + 5.0); // off-axis on purpose
    params.placement.translate[2] = Scalar::new(shank_origin[2]);
    add_op(&mut rt, mated);

    let report2 = regen_all(&mut rt).await;
    let snap2 = published(&report2, "mated place");
    assert_eq!(snap2.bodies.len(), 2, "target body + reseated mated body");

    // The RECORD's own `placement` was rewritten (the derived, no-undo
    // writeback this WP adds — `sync_mate_placements`), not just the
    // published geometry.
    let params_after = rt
        .operation_params(mate_record)
        .expect("mated record still exists");
    let translate = params_after["placement"]["translate"]
        .as_array()
        .expect("translate is an array");
    let ty = translate[1]["value"]
        .as_f64()
        .or_else(|| translate[1].as_f64())
        .expect("translate.y reads as a number");
    assert!(
        (ty - shank_origin[1]).abs() < 1e-3,
        "record placement.translate.y was rewritten onto the shank axis: got {ty}, want {}",
        shank_origin[1]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_component_with_an_unresolvable_mate_still_publishes_at_its_frozen_placement() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // A mate whose target body was NEVER created — simulates "the plate was
    // deleted." This is the exact scenario `wire.rs::wire_op_inputs`'s old
    // shape got wrong: with `mate.target` in the wire `inputs[]`, the
    // worker's pre-flight `resolve_input_refs` failed to resolve it and
    // `run_single_op` never ran at all — NOTHING published. Spec §5.5:
    // "never drop it, never silently move it."
    let mut mated = place_component_record(0xd3);
    let Operation::Known(KnownOperation::PlaceComponent(params)) = &mut mated.op else {
        unreachable!()
    };
    params.mate = Some(ComponentMate {
        self_attachment: "shankAxis".to_string(),
        target: ElementRef {
            primary: Some(PrimaryRef {
                body: BodyId(Uuid::from_u128(0xdead)),
                element: ElementId::new("el_never_existed"),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        },
        kind: MateKind::Concentric,
        flipped: false,
        extra: Default::default(),
    });
    // A recognizable frozen placement — proves it's the FROZEN one that
    // publishes, not some fallback/default.
    params.placement.translate = [Scalar::new(11.0), Scalar::new(22.0), Scalar::new(33.0)];
    add_op(&mut rt, mated);

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "unresolvable-mate place");
    assert_eq!(
        snap.bodies.len(),
        1,
        "the component STILL PUBLISHES — a NeedsRepair mate never drops the body"
    );

    let vol = exact_volume(&wm, snap.bodies[0].body).await;
    assert!(
        (vol - m6_shcs_volume()).abs() < 1.0,
        "published at the frozen M6 SHCS geometry, unaffected by the unresolvable mate"
    );

    assert!(
        snap.repair_summary.needs_repair_count > 0,
        "the unresolvable mate is flagged NeedsRepair, not silently ignored"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// WP-3.2 — the blob-backed source kinds, end to end against the real worker
// ─────────────────────────────────────────────────────────────────────────────

/// Bakes a live body into the §7.3 replay form (SCHEMA §7.8 `ExportGeometry`)
/// and returns `(bytes, sha256, format)` — the shape a component package's
/// geometry pointer records, and the shape `stage_component_blob` wants.
async fn bake_body(wm: &WorkerManager, body: BodyId) -> (Vec<u8>, String, u32) {
    let dir = std::env::temp_dir().join(format!("onecad-bake-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("bake dir");
    let path = dir.join("component.brep");
    let baked = onecad_lib::worker::manager::WorkerManager::export_geometry(
        wm,
        &path.to_string_lossy(),
        &[body],
        "brep",
    )
    .await
    .expect("ExportGeometry");
    assert_eq!(baked.codec, "brep");
    assert_eq!(baked.solid_count, 1, "a component is exactly one solid");
    let bytes = std::fs::read(&path).expect("read baked bytes");
    assert_eq!(bytes.len() as u64, baked.bytes_written);
    let sha = onecad_lib::imports::sha256_hex(&bytes);
    let _ = std::fs::remove_dir_all(&dir);
    (bytes, sha, baked.format)
}

fn document_component_record(
    rec: u128,
    document_sha256: String,
    sha256: String,
    brep_format: u32,
) -> OperationRecord {
    let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id: "acme.bracket".to_string(),
        component_version: "1.0.0".to_string(),
        component_revision: format!("sha256:{}", "0".repeat(64)),
        params: std::collections::BTreeMap::new(),
        source: ComponentSourceRef::Document {
            document_sha256,
            sha256,
            codec: onecad_core::document::record::ImportSourceCodec::Brep,
            brep_format: Some(brep_format),
            params: std::collections::BTreeMap::new(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Place Component", op)
}

/// **The spec §12 claim, automated.** A user-authored (`document`-source)
/// component places as its baked solid, and that solid survives save → reopen
/// on a FRESH worker with NO library anywhere — because the bytes were copied
/// into the document itself at place time, refcounted at save, materialized at
/// open, and read back by the worker through the wire-injected path.
///
/// Every link in that chain is load-bearing and silent when broken: miss the
/// save-time refcount and the blob is dropped; miss the open-time
/// materialization and the wire lowers an empty path. Both fail HERE, and
/// nowhere else in the suite.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_baked_component_survives_save_and_reopen_with_no_library() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };

    // 1. Model something real, and bake it — the same lane "Save as Component"
    //    will use. The generator screw is the convenient source solid: its
    //    volume is an exact analytic oracle both tracks already pin.
    let wm1 = spawn_worker(bin.clone()).await;
    let mut authoring = runtime_over(&wm1);
    add_op(&mut authoring, place_component_record(0xd1));
    let report = regen_all(&mut authoring).await;
    let source_body = published(&report, "authoring").bodies[0].body;
    let baked_volume = exact_volume(&wm1, source_body).await;
    let (bytes, sha, format) = bake_body(&wm1, source_body).await;

    // 2. Place it into a DIFFERENT document as a `document`-source component,
    //    staging the bytes exactly as `library.rs::place_component_at` does.
    let mut rt = runtime_over(&wm1);
    rt.stage_component_blob(
        &sha,
        onecad_core::document::record::ImportSourceCodec::Brep,
        &bytes,
    )
    .expect("stage the baked solid");
    add_op(
        &mut rt,
        document_component_record(0xd2, "a".repeat(64), sha.clone(), format),
    );
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "place baked component");
    assert_eq!(snap.bodies.len(), 1, "a baked component mints one body");
    let vol = exact_volume(&wm1, snap.bodies[0].body).await;
    assert!(
        (vol - baked_volume).abs() < 1e-6,
        "the placed solid IS the baked solid: got {vol}, want {baked_volume}"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("baked.onecad");
    rt.save(&path, save_meta()).expect("save");
    wm1.shutdown().await;

    // 3. Reopen on a fresh worker. There is no library root in this test at
    //    all, and the authoring runtime is gone — the ONLY thing carrying the
    //    geometry is the saved container.
    let wm2 = spawn_worker(bin).await;
    let mut rt2 = open_over(&wm2, &path);
    assert!(
        rt2.import_blob_shas().contains(&sha),
        "the baked solid came back out of the container (the save-time refcount kept it)"
    );
    let report2 = regen_all(&mut rt2).await;
    let snap2 = published(&report2, "reopen baked component");
    assert_eq!(
        snap2.bodies.len(),
        1,
        "reopen still publishes the component"
    );
    let vol2 = exact_volume(&wm2, snap2.bodies[0].body).await;
    assert!(
        (vol2 - baked_volume).abs() < 1e-6,
        "reopened volume {vol2} != baked {baked_volume}"
    );

    wm2.shutdown().await;
}

/// A component whose blob is NOT in the document fails that ONE step, loudly,
/// and leaves the rest of the document alone — the `io::imports` blast-radius
/// rule. The dangerous alternative is publishing something else instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_component_with_no_cached_geometry_fails_only_its_own_step() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    // A healthy generator component FIRST, so there is something to lose.
    add_op(&mut rt, place_component_record(0xd3));
    add_op(
        &mut rt,
        document_component_record(0xd4, "a".repeat(64), "b".repeat(64), 4),
    );
    let report = regen_all(&mut rt).await;

    // The plan early-stops AT the broken step and publishes everything before
    // it (`StoppedReason` on the snapshot), so the healthy component is still
    // there and the failure is recorded rather than swallowed.
    let snap = published(&report, "blob-less component");
    assert_eq!(
        snap.bodies.len(),
        1,
        "the healthy component still published; only the blob-less one failed"
    );
    assert_ne!(
        snap.stopped_reason,
        onecad_core::regen::StoppedReason::Completed,
        "the missing-geometry step must be reported, not silently skipped"
    );

    wm.shutdown().await;
}
