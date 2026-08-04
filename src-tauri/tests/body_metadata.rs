//! Body metadata durability against the REAL C++ OCCT worker, driven through the
//! app's [`DocumentRuntime`] like `suppression.rs` / `checkpoints.rs`.
//!
//! A body's `name` / `visible` are the only user-authored facts about a body — and
//! before this gate they were all three of unreachable, invisible and non-durable:
//!
//! * a worker-minted body (every extrude output) had NO `document.bodies` row, so
//!   `RenameBody` / `SetVisibility` / `DeleteBody` — which all validate against that
//!   registry — rejected it outright;
//! * the projection read `regen.bodies` FIRST, so even a row that did exist was
//!   shadowed by the regen mirror's generated `"Body N"` / `visible: true`;
//! * `save()` / `write_autosave()` overwrote `doc.bodies` with the regen registry
//!   wholesale, so whatever the user had authored never reached disk.
//!
//! These tests pin the fix end to end: adopt (identity) → overlay (intent) → save.
//!
//! * `set_visibility_on_a_regen_only_body_is_accepted` — the reachability half.
//! * `body_visibility_survives_regen_and_save` / `body_rename_survives_regen_and_save`
//!   — projection, a forced from-0 regen (which rebuilds the registry from scratch and
//!   re-derives the default names), and a save → reopen round trip.
//! * `undo_restores_prior_body_metadata` — `Inverse::RestoreBodies` restores the WHOLE
//!   registry, so an undo erases rows adopted after the edit it reverts; adoption runs
//!   after every mutation precisely to re-register them.
//! * `delete_body_on_regen_body_is_rejected` — adoption must not turn `DeleteBody` into
//!   a silent no-op on timeline geometry.
//! * `suppressed_feature_drops_its_body_from_the_tree` — nor may an adopted row outlive
//!   its geometry as a phantom tree entry.
//! * `rename_survives_save_reopen_while_its_feature_is_suppressed` (TRUST F4) — the
//!   durable half of the one above: the row of a body whose feature is suppressed lives
//!   ONLY in `document.bodies`, and a save that took membership from the regen mirror
//!   wholesale dropped it, losing the rename. It now reaches disk — without becoming a
//!   phantom tree row on reopen.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); `ONECAD_REQUIRE_WORKER=1`
//! hard-fails a missing binary (CI). A missing worker is a quiet local-dev skip.

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
use onecad_core::edit::{EditCommand, VisibilityTarget};
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::BodyDto;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors suppression.rs / checkpoints.rs)
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

fn reopen(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen container")
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

fn rid(rec: u128) -> RecordId {
    RecordId(Uuid::from_u128(rec))
}

/// The deterministic worker-minted body of a NewBody op (D1: `body_<opId>`).
fn body_of(rec: u128) -> BodyId {
    BodyId(Uuid::from_u128(rec))
}

/// The projected body row, which is what the model tree renders.
fn projected(rt: &DocumentRuntime, body: BodyId) -> BodyDto {
    rt.projection()
        .bodies
        .get(&body.to_string())
        .unwrap_or_else(|| panic!("body {body} in the projection"))
        .clone()
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
// Sketch + op record builders (a fully-constrained rectangle, as suppression.rs)
// ─────────────────────────────────────────────────────────────────────────────

const SK_A: u128 = 0xA00;
const EX_A: u128 = 0xA01;
const SK_B: u128 = 0xB00;
const EX_B: u128 = 0xB01;

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
// (1) Reachability: a body the timeline minted is addressable by a body command
// ─────────────────────────────────────────────────────────────────────────────

/// The extrude output is worker-minted (D1 `body_<opId>`) and no edit command ever
/// registered it, so pre-fix `document.bodies` had no row for it and every body
/// command failed validation with "body … not found" — the user could not hide or
/// rename the only geometry the document actually has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_visibility_on_a_regen_only_body_is_accepted() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    let report = regen_all(&mut rt).await;
    assert_eq!(published(&report, "box A").bodies.len(), 1);

    rt.apply(EditCommand::SetVisibility {
        target: VisibilityTarget::Body(body_of(EX_A)),
        visible: false,
    })
    .expect("SetVisibility on a regen-minted body must be ACCEPTED (adoption gave it a row)");
    rt.apply(EditCommand::RenameBody {
        body: body_of(EX_A),
        name: "Housing".into(),
    })
    .expect("RenameBody on a regen-minted body must be ACCEPTED");

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (2) Visibility durability: projection → forced from-0 regen → save → reopen
// ─────────────────────────────────────────────────────────────────────────────

/// `visible: false` must survive (a) the projection, which used to read the regen row
/// first, (b) a from-0 regen, which rebuilds the body registry from
/// `BodyRegistry::new()` and would otherwise reset the flag to the `true` default, and
/// (c) `save()` → reopen, which used to clobber `doc.bodies` with the regen registry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_visibility_survives_regen_and_save() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bodies.onecad");
    let body = body_of(EX_A);
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "box A");
    assert!(projected(&rt, body).visible, "a fresh body starts visible");

    rt.apply(EditCommand::SetVisibility {
        target: VisibilityTarget::Body(body),
        visible: false,
    })
    .expect("SetVisibility");
    assert!(
        !projected(&rt, body).visible,
        "the projection reads the DOCUMENT's visibility, not the regen mirror's"
    );

    // A full from-0 regen: the executor starts from a fresh registry, so the regen
    // row is `visible: true` again — only the overlay keeps the user's intent.
    let _ = published(&regen_all(&mut rt).await, "box A re-regenerated");
    assert!(
        !projected(&rt, body).visible,
        "a from-0 regen must not resurrect a hidden body"
    );

    rt.save(&path, save_meta()).expect("save");
    let reopened = reopen(&wm, &path);
    assert!(
        !projected(&reopened, body).visible,
        "the hidden flag reached disk and came back (save merged, not clobbered)"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (3) Name durability: the same shape for `RenameBody`
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_rename_survives_regen_and_save() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bodies.onecad");
    let body = body_of(EX_A);
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "box A");
    let default_name = projected(&rt, body).name;
    assert!(
        !default_name.is_empty(),
        "the regen row carries a default name"
    );

    rt.apply(EditCommand::RenameBody {
        body,
        name: "Bracket".into(),
    })
    .expect("RenameBody");
    assert_eq!(projected(&rt, body).name, "Bracket");

    let _ = published(&regen_all(&mut rt).await, "box A re-regenerated");
    assert_eq!(
        projected(&rt, body).name,
        "Bracket",
        "a from-0 regen re-derives \"{default_name}\" — the user's name must win"
    );

    rt.save(&path, save_meta()).expect("save");
    let reopened = reopen(&wm, &path);
    assert_eq!(
        projected(&reopened, body).name,
        "Bracket",
        "the name reached disk and came back"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (4) Undo: RestoreBodies restores the WHOLE registry
// ─────────────────────────────────────────────────────────────────────────────

/// Every body command's inverse is `Inverse::RestoreBodies`, which swaps the whole
/// registry back — so undoing a rename also erases every row adopted AFTER it. Here
/// box B's ops are added before the rename but only adopted by the regen that follows
/// it, so the rename's memento predates B's row: without adoption re-running after the
/// undo, B becomes unaddressable again (a body command on it would fail) even though
/// its geometry is live.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undo_restores_prior_body_metadata() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (a, b) = (body_of(EX_A), body_of(EX_B));

    build_box_a(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "box A");
    let default_name = projected(&rt, a).name;

    // Box B's ops land BEFORE the rename; its body is adopted by the regen AFTER it.
    build_box_b(&mut rt);
    rt.apply(EditCommand::RenameBody {
        body: a,
        name: "Bracket".into(),
    })
    .expect("RenameBody");
    let _ = published(&regen_all(&mut rt).await, "boxes A+B");
    assert!(!projected(&rt, b).name.is_empty(), "box B is in the tree");

    // Undo pops the rename (the newest committed edit).
    assert!(rt.undo().is_some(), "the rename is undoable");
    assert_eq!(
        projected(&rt, a).name,
        default_name,
        "undo restored the default name"
    );
    rt.apply(EditCommand::RenameBody {
        body: b,
        name: "Cover".into(),
    })
    .expect("box B stays addressable — adoption re-registers the row the undo erased");
    assert_eq!(projected(&rt, b).name, "Cover");

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (5) DeleteBody on timeline geometry is a loud rejection, not a silent no-op
// ─────────────────────────────────────────────────────────────────────────────

/// Adoption gives a regen body a `document.bodies` row, which is exactly what
/// `DeleteBody` validates against — so without a runtime-level guard the command would
/// now "succeed", drop the row, and change NOTHING the user can see (the projection
/// keeps listing the regen row, and the next regen re-mints the body anyway). The
/// runtime rejects it and names the real remedy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_body_on_regen_body_is_rejected() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body = body_of(EX_A);

    build_box_a(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "box A");
    let before = projected(&rt, body);

    let err = rt
        .apply(EditCommand::DeleteBody { body })
        .expect_err("DeleteBody on timeline geometry must be REJECTED");
    let msg = err.to_string();
    assert!(
        msg.contains("produced by the timeline") && msg.contains("producing feature"),
        "the rejection must name the reason and the remedy, got: {msg}"
    );
    assert_eq!(
        projected(&rt, body),
        before,
        "a rejected DeleteBody leaves the projection untouched"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (6) Adoption must not leave phantom tree rows behind
// ─────────────────────────────────────────────────────────────────────────────

/// A `document.bodies` row is never pruned — it outlives the geometry it describes
/// (suppress the producing feature and the next regen publishes no body at all). The
/// projection therefore takes its MEMBERSHIP from the regen mirror alone: a row with
/// no live body would otherwise render a phantom, mesh-less entry in the tree. The
/// row itself is still the right thing to keep: un-suppressing brings the body back
/// with the user's name and visibility intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppressed_feature_drops_its_body_from_the_tree() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let body = body_of(EX_A);

    build_box_a(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "box A");
    rt.apply(EditCommand::RenameBody {
        body,
        name: "Bracket".into(),
    })
    .expect("RenameBody");

    let suppress = |suppressed| EditCommand::SetOperationSuppression {
        record: rid(EX_A),
        suppressed,
        cascade: false,
    };
    rt.apply(suppress(true)).expect("suppress the extrude");
    let report = regen_all(&mut rt).await;
    assert_eq!(published(&report, "suppressed").bodies.len(), 0);
    assert!(
        rt.projection().bodies.is_empty(),
        "no live body ⇒ no tree row (the adopted document row must not become a phantom)"
    );

    rt.apply(suppress(false)).expect("un-suppress");
    let report = regen_all(&mut rt).await;
    assert_eq!(published(&report, "un-suppressed").bodies.len(), 1);
    assert_eq!(
        projected(&rt, body).name,
        "Bracket",
        "the surviving document row re-overlays the returning body"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (7) TRUST F4: the row of a SUPPRESSED feature's body must reach disk
// ─────────────────────────────────────────────────────────────────────────────

/// The in-memory half of this is test (6): a suppressed feature drops its body from the
/// tree while `document.bodies` keeps the user's name for when it comes back. The
/// DURABLE half was broken. `merge_body_metadata` takes membership from the regen mirror
/// wholesale, so the suppressed body's row — which exists ONLY in `document.bodies` —
/// was never written, and reopening + un-suppressing re-minted the body with the
/// regenerated `"Body N"` default. The rename the user made was silently gone.
///
/// The save path now also appends the document-only rows. The projection still takes
/// membership from the regen mirror alone, so the persisted row must NOT come back as a
/// phantom tree entry on reopen either — both halves are asserted here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rename_survives_save_reopen_while_its_feature_is_suppressed() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("suppressed-rename.onecad");
    let body = body_of(EX_A);
    let mut rt = runtime_over(&wm);

    build_box_a(&mut rt);
    let _ = published(&regen_all(&mut rt).await, "box A");
    let default_name = projected(&rt, body).name;
    rt.apply(EditCommand::RenameBody {
        body,
        name: "Bracket".into(),
    })
    .expect("RenameBody");
    assert_eq!(projected(&rt, body).name, "Bracket");

    // Suppress the producing feature: the body leaves the regen mirror, so its ONLY
    // remaining record is the `document.bodies` row carrying the rename.
    let suppress = |suppressed| EditCommand::SetOperationSuppression {
        record: rid(EX_A),
        suppressed,
        cascade: suppressed,
    };
    rt.apply(suppress(true)).expect("suppress the extrude");
    assert_eq!(
        published(&regen_all(&mut rt).await, "suppressed")
            .bodies
            .len(),
        0
    );
    assert!(
        rt.projection().bodies.is_empty(),
        "no live body ⇒ no tree row"
    );

    rt.save(&path, save_meta()).expect("save");
    let mut reopened = reopen(&wm, &path);
    assert!(
        reopened.projection().bodies.is_empty(),
        "the persisted row of a suppressed feature's body must NOT render as a phantom \
         tree entry on reopen — projection membership stays regen-only"
    );

    // Un-suppress → regen re-mints the SAME deterministic body id (D1 `body_<opId>`),
    // and `adopt_regen_bodies` is insert-only, so the persisted row wins the overlay.
    reopened.apply(suppress(false)).expect("un-suppress");
    let report = regen_all(&mut reopened).await;
    assert_eq!(
        published(&report, "un-suppressed after reopen")
            .bodies
            .len(),
        1
    );
    assert_eq!(
        projected(&reopened, body).name,
        "Bracket",
        "the rename survived save → reopen → un-suppress (it would read the re-derived \
         \"{default_name}\" if the row had been dropped on save)"
    );

    wm.shutdown().await;
    eprintln!(
        "SUPPRESSED-RENAME PASS: \"Bracket\" survived save/reopen with its feature suppressed"
    );
}
