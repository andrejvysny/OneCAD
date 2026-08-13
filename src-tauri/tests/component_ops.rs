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
    KnownOperation, LinearPatternParams, MateKind, Operation, OperationRecord,
    PlaceComponentParams,
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
        self_frame: None,
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
        self_frame: None,
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
        false,
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

/// Every SEEDED package (WP-A2) actually places through the real worker.
///
/// This is the pairing nothing else can check: a manifest names a
/// `source.generator`, the worker registers a set of generator ids, and the two
/// live in different languages in different directories. Before WP-A1 a
/// mismatch was invisible (every id built a socket cap screw); now it is an
/// `OP_FAILED`, and this test is what turns that from a runtime surprise into a
/// build-time one. It also proves the shipped `thread` domains are sizes the
/// generators accept.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_seeded_component_places_through_the_real_worker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    onecad_lib::library_seed::seed_library(dir.path()).expect("seed");
    let mut library = onecad_library::Library::open(dir.path()).expect("open seeded library");
    library.reindex().expect("reindex");

    let mut placements: Vec<(String, String, u32)> = Vec::new();
    for (id, versions) in &library.index().components {
        for entry in versions.values() {
            let generator = entry
                .generator_id
                .clone()
                .unwrap_or_else(|| panic!("{id}: a seeded package must be a generator source"));
            placements.push((id.clone(), generator, entry.generator_version.unwrap_or(1)));
        }
    }
    assert!(
        placements.len() >= 10,
        "the seed catalog should carry every seeded family, got {placements:?}"
    );

    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    for (i, (component_id, generator_id, generator_version)) in placements.iter().enumerate() {
        // M6 is in every seeded FASTENER family's declared `thread` domain —
        // the one size they all share, which is why it is the sweep's probe.
        // The non-fastener families (bearings, motors) are not keyed by a
        // thread at all: they ignore it and build their own documented
        // default, which is exactly what the library's preview lane (which
        // sends no params whatsoever) relies on.
        let mut generator_params = std::collections::BTreeMap::new();
        generator_params.insert(
            "thread".to_string(),
            ComponentParamValue::Text("M6".to_string()),
        );
        let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
            component_id: component_id.clone(),
            component_version: "1.0.0".to_string(),
            component_revision: format!("sha256:{}", "0".repeat(64)),
            params: generator_params.clone(),
            source: ComponentSourceRef::Generator {
                generator_id: generator_id.clone(),
                generator_version: *generator_version,
                params: generator_params,
                extra: Default::default(),
            },
            mate: None,
            // Spread them out so each is its own solid, never a coincident pile.
            placement: FrozenPlacement {
                translate: [
                    Scalar::new(60.0 * i as f64),
                    Scalar::new(0.0),
                    Scalar::new(0.0),
                ],
                rotate: Default::default(),
            },
            extra: Default::default(),
        }));
        add_op(
            &mut rt,
            OperationRecord::new(
                RecordId(Uuid::from_u128(0xe0 + i as u128)),
                0,
                "Place Component",
                op,
            ),
        );
    }

    let report = regen_all(&mut rt).await;
    let snap = published(&report, "seeded catalog");
    assert_eq!(
        snap.stopped_reason,
        onecad_core::regen::StoppedReason::Completed,
        "no seeded package may fail its step"
    );
    assert_eq!(
        snap.bodies.len(),
        placements.len(),
        "every seeded component publishes exactly one body"
    );
    for body in &snap.bodies {
        let vol = exact_volume(&wm, body.body).await;
        assert!(vol > 0.0, "a seeded component published an empty solid");
    }

    wm.shutdown().await;
}

/// "Save as Component" end to end against the real worker (WP-B2, spec §7):
/// author a body as a `document` package, then PLACE that package back into a
/// fresh document and land the same solid.
///
/// This is the whole authoring claim in one test. The bake
/// (`ExportGeometry`), the content-addressed blob, the manifest + revision, the
/// index, and the placement path that reads all of it are separately tested;
/// what only this can show is that they compose — an authored component is a
/// placeable component, with the geometry the author actually saw.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_saved_as_a_component_places_back_as_the_same_solid() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;

    // Something to author FROM: a generator component's own solid is a real
    // published body, which is all "Save as Component" needs.
    let runtime = tokio::sync::Mutex::new(Some(runtime_over(&wm)));
    let body = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        add_op(rt, place_component_record(0xa1));
        let report = regen_all(rt).await;
        published(&report, "author source").bodies[0].body
    };
    let authored_volume = exact_volume(&wm, body).await;

    let exporter: Arc<dyn onecad_lib::export::GeometryExporter> = Arc::new(wm.clone());
    let saved = onecad_lib::library::save_as_component_at(
        library_dir.path(),
        &runtime,
        exporter,
        body.to_string(),
        onecad_lib::library::NewComponentSpec {
            id: "acme.bracket".to_string(),
            version: "1.0.0".to_string(),
            name: "Bracket".to_string(),
            category: vec!["mine".to_string()],
            tags: vec!["authored".to_string()],
            designation: None,
            attachments: [(
                "seat".to_string(),
                onecad_lib::library::AttachmentSpecInput {
                    on: "face:seat".to_string(),
                    accepts: vec!["plane".to_string()],
                    frame: None,
                },
            )]
            .into_iter()
            .collect(),
            parameters: Default::default(),
        },
        None,
        false,
    )
    .await
    .expect("save as component");
    assert_eq!(saved.source_kind, "document");
    assert_eq!(saved.attachments.len(), 1);

    // The library on disk now carries a package whose geometry is a blob, not a
    // recipe — the authoring document is gone from the picture entirely.
    let library = onecad_library::Library::open(library_dir.path()).expect("open library");
    let (_v, entry) = library.get("acme.bracket", None).expect("indexed");
    assert_eq!(entry.revision, saved.revision);

    // Place it into a FRESH document. Resolution goes through the same lane a
    // real placement uses, blob and all.
    let resolved = library
        .resolve_source(&onecad_library::resolve::ResolveRequest {
            component_id: "acme.bracket",
            component_version: "1.0.0",
            component_revision: &saved.revision,
        })
        .expect("resolve the authored package");
    let (sha256, bytes, codec, format) = match resolved {
        onecad_library::resolve::ResolvedSource::Document { blob, .. } => {
            (blob.sha256, blob.bytes, blob.codec, blob.format)
        }
        other => panic!("expected a document source, got {other:?}"),
    };

    let mut fresh = runtime_over(&wm);
    fresh
        .stage_component_blob(
            &sha256,
            onecad_core::document::record::ImportSourceCodec::from_extension(&codec).unwrap(),
            &bytes,
        )
        .expect("stage the authored blob");
    let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id: "acme.bracket".to_string(),
        component_version: "1.0.0".to_string(),
        component_revision: saved.revision.clone(),
        params: Default::default(),
        source: ComponentSourceRef::Document {
            // The record's provenance pointer is the frozen document's own
            // digest; this test does not read it back, so any well-formed
            // digest stands in (`validate` requires bare 64-hex).
            document_sha256: "0".repeat(64),
            sha256,
            codec: onecad_core::document::record::ImportSourceCodec::from_extension(&codec)
                .unwrap(),
            brep_format: format,
            params: Default::default(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }));
    add_op(
        &mut fresh,
        OperationRecord::new(RecordId(Uuid::from_u128(0xa2)), 0, "Place Component", op),
    );
    let report = regen_all(&mut fresh).await;
    let snap = published(&report, "place the authored component");
    assert_eq!(
        snap.bodies.len(),
        1,
        "the authored package publishes one body"
    );
    let replaced_volume = exact_volume(&wm, snap.bodies[0].body).await;
    assert!(
        (replaced_volume - authored_volume).abs() < 1e-6,
        "authored {authored_volume} vs placed {replaced_volume} — the bake must be the same solid"
    );

    wm.shutdown().await;
}

/// Union-at-bake (WP-F1.2, spec §9): a body that flattens to SEVERAL solids is
/// refused with the machine-readable marker the dialog keys its offer on, and
/// the very same save succeeds once the author opts into the fuse.
///
/// Both halves matter. Without the marker the dialog would have to match prose
/// to know whether to offer the fuse; without the refusal a multi-solid
/// component would reach someone else's machine, which is the failure spec §9
/// exists to prevent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_solid_body_is_refused_and_the_union_opt_in_saves_it() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;

    // A V1 LinearPattern with `fuseResult:false` gathers source + instance into
    // ONE compound body — the everyday way an author ends up holding something
    // that is not one solid. Spacing 4mm keeps the two M6 screws OVERLAPPING
    // (head radius 5), so the fuse can actually connect them.
    let runtime = tokio::sync::Mutex::new(Some(runtime_over(&wm)));
    let (pattern_body, source_volume) = {
        let mut guard = runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        add_op(rt, place_component_record(0xc1));
        let report = regen_all(rt).await;
        let source = published(&report, "pattern source").bodies[0].body;
        add_op(
            rt,
            OperationRecord::new(
                RecordId(Uuid::from_u128(0xc2)),
                0,
                "LinearPattern",
                Operation::Known(KnownOperation::LinearPattern(LinearPatternParams {
                    source_body: Some(source),
                    direction: Vec3::new_unchecked(1.0, 0.0, 0.0),
                    spacing: Scalar::new(4.0),
                    count: 2,
                    fuse_result: false,
                    result_policy_version: None,
                    extra: Default::default(),
                })),
            ),
        );
        let report = regen_all(rt).await;
        let snap = published(&report, "linear pattern");
        let pattern = snap
            .bodies
            .iter()
            .map(|b| b.body)
            .find(|b| *b != source)
            .expect("the pattern publishes its own body");
        (pattern, exact_volume(&wm, source).await)
    };

    let exporter: Arc<dyn onecad_lib::export::GeometryExporter> = Arc::new(wm.clone());
    let spec = || onecad_lib::library::NewComponentSpec {
        id: "acme.pair".to_string(),
        version: "1.0.0".to_string(),
        name: "Pair".to_string(),
        category: vec!["mine".to_string()],
        tags: vec![],
        designation: None,
        attachments: Default::default(),
        parameters: Default::default(),
    };

    let refused = onecad_lib::library::save_as_component_at(
        library_dir.path(),
        &runtime,
        exporter.clone(),
        pattern_body.to_string(),
        spec(),
        None,
        false,
    )
    .await
    .expect_err("a two-solid body must not become a component");
    let message = refused.to_string();
    assert!(
        message.contains(onecad_lib::library::MULTI_SOLID_REFUSAL),
        "the refusal must carry the marker the dialog keys on, got {message:?}"
    );

    let saved = onecad_lib::library::save_as_component_at(
        library_dir.path(),
        &runtime,
        exporter,
        pattern_body.to_string(),
        spec(),
        None,
        true,
    )
    .await
    .expect("the union opt-in saves the same body");
    assert_eq!(saved.source_kind, "document");

    // The package holds the FUSED pair: more than one screw, less than two
    // (they overlap). Placing it back proves the bytes are one real solid.
    let library = onecad_library::Library::open(library_dir.path()).expect("open library");
    let resolved = library
        .resolve_source(&onecad_library::resolve::ResolveRequest {
            component_id: "acme.pair",
            component_version: "1.0.0",
            component_revision: &saved.revision,
        })
        .expect("resolve the fused package");
    let (sha256, bytes, codec, format) = match resolved {
        onecad_library::resolve::ResolvedSource::Document { blob, .. } => {
            (blob.sha256, blob.bytes, blob.codec, blob.format)
        }
        other => panic!("expected a document source, got {other:?}"),
    };
    let mut fresh = runtime_over(&wm);
    fresh
        .stage_component_blob(
            &sha256,
            onecad_core::document::record::ImportSourceCodec::from_extension(&codec).unwrap(),
            &bytes,
        )
        .expect("stage the fused blob");
    let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id: "acme.pair".to_string(),
        component_version: "1.0.0".to_string(),
        component_revision: saved.revision.clone(),
        params: Default::default(),
        source: ComponentSourceRef::Document {
            document_sha256: "0".repeat(64),
            sha256,
            codec: onecad_core::document::record::ImportSourceCodec::from_extension(&codec)
                .unwrap(),
            brep_format: format,
            params: Default::default(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }));
    add_op(
        &mut fresh,
        OperationRecord::new(RecordId(Uuid::from_u128(0xc3)), 0, "Place Component", op),
    );
    let report = regen_all(&mut fresh).await;
    let snap = published(&report, "place the fused component");
    assert_eq!(snap.bodies.len(), 1, "the fused package publishes one body");
    let placed = exact_volume(&wm, snap.bodies[0].body).await;
    assert!(
        placed > source_volume && placed < 2.0 * source_volume,
        "the fused pair ({placed}) must be more than one screw ({source_volume}) and less than \
         two — the overlap is what makes it a single solid"
    );

    wm.shutdown().await;
}

/// Every seeded component MESHES for the library UI (WP-B6), with no document
/// open — which is the case that matters, because the most useful place to
/// browse a catalog is the start screen, before any project exists.
///
/// A preview is a `PlaceComponent` candidate against a throwaway copy of the
/// worker's session head (SCHEMA §7.6), and an empty head is a perfectly good
/// thing to place into. This drives the same command the cards call and asserts
/// a real MESH1 with vertices comes back — a card that silently falls back to
/// its icon for the whole catalog would otherwise look exactly like the
/// pre-WP-B6 behaviour.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_seeded_component_meshes_for_the_library_ui() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    onecad_lib::library_seed::seed_library(dir.path()).expect("seed");
    let mut library = onecad_library::Library::open(dir.path()).expect("open");
    library.reindex().expect("reindex");

    let wm = spawn_worker(bin).await;
    let preview: Arc<dyn onecad_lib::worker::PreviewEngine> = Arc::new(wm.clone());

    let ids: Vec<(String, String)> = library
        .index()
        .components
        .iter()
        .flat_map(|(id, versions)| versions.keys().map(move |v| (id.clone(), v.clone())))
        .collect();
    assert!(
        ids.len() >= 10,
        "the seed catalog should be complete: {ids:?}"
    );

    for (id, version) in ids {
        let result = onecad_lib::library::component_preview_mesh_at(
            dir.path(),
            preview.clone(),
            id.clone(),
            version.clone(),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{id}@{version} did not preview: {e:?}"));

        assert_eq!(
            result.bodies.len(),
            1,
            "{id}: a component previews as exactly one body"
        );
        // MESH1 magic + a non-zero vertex count: the bytes are a mesh the
        // frontend's own parser will accept, not an empty success. The magic is
        // the LE u32 0x4d455348, i.e. the bytes `HSEM` on the wire, and the
        // vertex count sits at 0x08 (see `parseMeshPayload.ts`).
        let mesh = &result.bodies[0].mesh;
        assert!(mesh.len() > 64, "{id}: mesh is shorter than a MESH1 header");
        assert_eq!(&mesh[0..4], b"HSEM", "{id}: not a MESH1 blob");
        let vertex_count = u32::from_le_bytes([mesh[8], mesh[9], mesh[10], mesh[11]]);
        assert!(vertex_count > 0, "{id}: meshed to zero vertices");
    }

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// WP-F2b: the seeded project starters (spec §8)
// ─────────────────────────────────────────────────────────────────────────────

/// The exact analytic volume of the NEMA 17 the `nema17-mount` starter places:
/// the 42.3 mm frame block at the frame's own 40 mm default length, plus the
/// Ø22 × 2 pilot boss and the Ø5 × 24 shaft (which overlaps the boss for its
/// first 2 mm), minus the four Ø3 blind mounting holes 4.5 mm deep. Spelled
/// from the source dimensions, exactly as `motor_volume` does on the C++ side —
/// a divergence between the two shows up on both.
fn nema17_default_volume() -> f64 {
    let cyl = |d: f64, h: f64| PI * (d / 2.0) * (d / 2.0) * h;
    42.3 * 42.3 * 40.0 + cyl(22.0, 2.0) + cyl(5.0, 24.0 - 2.0) - 4.0 * cyl(3.0, 4.5)
}

/// A seeded starter must OPEN and REGENERATE through the real worker — a
/// template that lists on the start screen but cannot be instantiated is worse
/// than no template at all. The motor-mount starter is the one that can fail:
/// it carries a `PlaceComponent` record authored with no worker in sight, so
/// this is the only place the authored record meets the generator it names.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seeded_nema17_starter_regenerates_into_a_motor() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    onecad_lib::library_seed::seed_library(library_dir.path()).expect("seed");
    let entry =
        onecad_library::template::get(library_dir.path(), "onecad.std.template.nema17-mount")
            .expect("the motor-mount starter seeds");

    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &entry.document_path);
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "nema17 starter");
    assert_eq!(
        snap.bodies.len(),
        1,
        "the starter opens with exactly the motor"
    );

    let vol = exact_volume(&wm, snap.bodies[0].body).await;
    assert!(
        (vol - nema17_default_volume()).abs() < 1.0,
        "starter volume: got {vol}, want {} (NEMA 17 at its default 40 mm body)",
        nema17_default_volume()
    );

    wm.shutdown().await;
}

/// The other two starters must open and regenerate too — to an EMPTY published
/// snapshot, which is the honest content of both (`blank` is empty; the
/// printed-part starter carries one datum and no geometry).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_seeded_geometry_free_starters_open_and_publish_nothing() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    onecad_lib::library_seed::seed_library(library_dir.path()).expect("seed");
    let wm = spawn_worker(bin).await;

    for id in [
        "onecad.std.template.blank",
        "onecad.std.template.printed-part",
    ] {
        let entry = onecad_library::template::get(library_dir.path(), id)
            .unwrap_or_else(|| panic!("{id} seeds"));
        let mut rt = open_over(&wm, &entry.document_path);
        let report = regen_all(&mut rt).await;
        // `NoOp`, not an empty `Published`: a timeline with no step has nothing
        // to publish. What matters here is that the container OPENED and the
        // regen did not fail — the starter is honestly empty.
        assert!(
            matches!(report.outcome, Outcome::NoOp),
            "{id}: a geometry-free starter regenerates to NoOp, got {:?}",
            report.outcome
        );
    }

    wm.shutdown().await;
}
