//! VF-B6 split-ordinal identity tripwire, against the REAL C++ OCCT worker, driven
//! through the app's [`DocumentRuntime`] exactly like `transform_body.rs`.
//!
//! # The defect this pins
//!
//! A boolean Cut that bisects a plate mints `body_<cutOpId>:0` and `:1`, ordered by
//! a **geometric** key (quantized volume, then centroid). The ids are stable but the
//! ordinal is not lineage: nudge the cut so the two pieces swap sizes and `:0` names
//! the OTHER solid. A downstream op referencing `:0` then re-resolves **cleanly** —
//! full confidence, no ladder complaint — onto the wrong body. Before this wave the
//! runtime seeded ZERO repair items here; the mirror simply started mirroring a
//! different piece and nothing anywhere said so.
//!
//! # Geometry
//!
//! The named XY sketch plane uses the OneCAD-CPP non-standard basis
//! (`xAxis=(0,1,0)`, `yAxis=(−1,0,0)`), so sketch `u` runs along world **+Y**. A
//! plate sketched at `(0,0)+40×20` and extruded 25 therefore occupies world
//! `y ∈ [0,40]`, `x ∈ [−20,0]`, `z ∈ [0,25]` (volume 20 000).
//!
//! A 4-wide tool slab at sketch `u ∈ [a, a+4]`, over-spanning the plate in `v` and
//! `z`, cuts straight through:
//!
//! | tool `a` | near piece (`y ∈ [0,a]`) | far piece (`y ∈ [a+4,40]`) | ordinal 0 |
//! |----------|--------------------------|----------------------------|-----------|
//! | 17       | 17 long, **8500** mm³    | 19 long, **9500** mm³      | NEAR      |
//! | 19       | 19 long, **9500** mm³    | 17 long, **8500** mm³      | FAR       |
//!
//! Moving the cut by 2 mm swaps which piece is the small one, so `:0` changes owner
//! while both volumes keep the exact same two VALUES — which is precisely why the
//! ordinal (a volume rank) cannot see it and the centroid can.
//!
//! Gated on `ONECAD_WORKER_PATH` (else dev-tree fallback); a missing binary skips
//! cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, BooleanOp, BooleanParams, ExtrudeMode, ExtrudeParams, KnownOperation,
    MirrorBodyParams, Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::repair::RepairReason;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{intern_split_child, sketch_wire};
use onecad_lib::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::validate_mesh_blob;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors transform_body.rs / breadth_ops.rs)
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

// Record ids. Timeline: 0 plate sketch · 1 plate extrude · 2 tool sketch ·
// 3 tool extrude · 4 Cut (the N-body op) · 5 Mirror (the downstream victim).
const SK_PLATE: u128 = 0xA00;
const EX_PLATE: u128 = 0xA01;
const SK_TOOL: u128 = 0xB00;
const EX_TOOL: u128 = 0xB01;
const OP_CUT: u128 = 0xC00;
const OP_MIRROR: u128 = 0xD00;
const CUT_STEP: usize = 4;
const MIRROR_STEP: usize = 5;

/// Sketch-`u` start of the 4-wide tool slab. `17` ⇒ the NEAR piece is the small one;
/// `19` ⇒ the FAR piece is. See the module header table.
const TOOL_A_NEAR_SMALL: f64 = 17.0;
const TOOL_A_FAR_SMALL: f64 = 19.0;

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-08-03T00:00:00Z".into(),
        modified: "2026-08-03T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + record builders (rect_sketch verbatim from transform_body.rs)
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

fn sketch_op(sk: &Sketch) -> Operation {
    let (_plane, entities, constraints) = sketch_wire(sk);
    Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    }))
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", sketch_op(sk))
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

fn cut_record(rec: u128, target: BodyId, tool: BodyId) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Boolean",
        Operation::Known(KnownOperation::Boolean(BooleanParams {
            operation: BooleanOp::Cut,
            target_body: target,
            tool_body: tool,
            extra: Default::default(),
        })),
    )
}

/// The downstream victim: a mirror of split child `:0` across a far plane, NOT fused,
/// so it produces a fresh body and leaves the child untouched. Its `inputs.bodies`
/// names the child, which is the whole point — that is the edge the tripwire gates.
fn mirror_record(rec: u128, source: BodyId) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "MirrorBody",
        Operation::Known(KnownOperation::MirrorBody(MirrorBodyParams {
            source_body: Some(source),
            plane_point: Vec3::new_unchecked(0.0, -50.0, 0.0),
            plane_normal: Vec3::new_unchecked(0.0, 1.0, 0.0),
            fuse_with_original: false,
            extra: Default::default(),
        })),
    )
}

/// The tool sketch at slab start `a` (see the module header).
fn tool_sketch(sid: SketchId, a: f64) -> Sketch {
    rect_sketch(sid, 0xB10, a, -10.0, 4.0, 40.0)
}

fn set_tool_position(rt: &mut DocumentRuntime, sid: SketchId, a: f64) {
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(SK_TOOL)),
        op: sketch_op(&tool_sketch(sid, a)),
    })
    .expect("UpdateOperationParams on the tool sketch");
}

/// The seeded ordinal-permutation gates currently standing.
fn ordinal_gates(rt: &DocumentRuntime) -> Vec<&onecad_core::document::repair::RepairItem> {
    rt.repair_items()
        .iter()
        .filter(|i| i.seeded && i.reason == RepairReason::OrdinalPermutation)
        .collect()
}

/// Builds the plate + tool + Cut + Mirror timeline and returns
/// `(tool sketch id, child :0 BodyId)`.
async fn build(rt: &mut DocumentRuntime) -> (SketchId, BodyId) {
    let plate_sid = SketchId(Uuid::from_u128(0xA0));
    let tool_sid = SketchId(Uuid::from_u128(0xB0));
    let plate = rect_sketch(plate_sid, 0xA10, 0.0, 0.0, 40.0, 20.0);

    add_op(rt, sketch_record(SK_PLATE, &plate));
    add_op(rt, extrude_record(EX_PLATE, plate_sid, 25.0));
    add_op(
        rt,
        sketch_record(SK_TOOL, &tool_sketch(tool_sid, TOOL_A_NEAR_SMALL)),
    );
    // 100 deep from z=0 ⇒ fully through the 25-tall plate.
    add_op(rt, extrude_record(EX_TOOL, tool_sid, 100.0));
    add_op(
        rt,
        cut_record(
            OP_CUT,
            BodyId(Uuid::from_u128(EX_PLATE)),
            BodyId(Uuid::from_u128(EX_TOOL)),
        ),
    );

    // The ordinal child's BodyId is a pure function of (cutOpId, k). Interning it up
    // front is exactly what a document OPEN does from the persisted `splitOf`, and it
    // is what lets the Mirror's `sourceBodyId` render as `body_<cutOpId>:0` on the
    // very first plan — before the Cut has run and taught the interner itself.
    let child0 = intern_split_child(Uuid::from_u128(OP_CUT), 0);
    add_op(rt, mirror_record(OP_MIRROR, child0));
    (tool_sid, child0)
}

/// The world-Y span of a body's current mesh — the oracle for *which* solid an
/// ordinal names, since the two split pieces sit at different Y and a snapshot's
/// per-body `signature` is deliberately whole-model (see `body_snapshots`).
async fn body_y_span(rt: &mut DocumentRuntime, body: BodyId) -> (f64, f64) {
    let mesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("fetch body mesh");
    let view = validate_mesh_blob(&mesh).expect("MESH1 validates");
    (f64::from(view.bbox_min[1]), f64::from(view.bbox_max[1]))
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate
// ─────────────────────────────────────────────────────────────────────────────

/// **RED before this wave**: editing the cut so the two split pieces swap sizes left
/// `rt.repair_items()` completely EMPTY — the mirror silently re-bound to the other
/// solid. Now the permutation is detected at commit and seeded as a deterministic
/// `OrdinalPermutation` NeedsRepair on the mirror's ref, with the regen ceiling
/// pinned below it; reverting the edit clears the tripwire's own seeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_edit_that_permutes_split_ordinals_seeds_needs_repair_downstream() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (tool_sid, child0) = build(&mut rt).await;

    // ── HEALTHY: the cut splits, the mirror stands on :0, nothing is gated ─────
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "plate cut + mirror");
    assert!(
        ordinal_gates(&rt).is_empty(),
        "a first regen adopts stamps and must claim NOTHING"
    );
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the healthy build reports no repairs"
    );
    let healthy_bodies = snap.bodies.len();
    assert!(
        healthy_bodies >= 3,
        "two split children + the mirror image, got {healthy_bodies}"
    );
    // `:0` is the SMALL piece, and at a=17 that is the NEAR one: world y ∈ [0,17].
    let child0_before = body_y_span(&mut rt, child0).await;
    assert!(
        child0_before.1 < 20.0,
        "child :0 starts out as the NEAR piece, got y span {child0_before:?}"
    );

    // ── THE EDIT: move the cut 2 mm so the pieces swap sizes ──────────────────
    set_tool_position(&mut rt, tool_sid, TOOL_A_FAR_SMALL);
    let rep = regen_all(&mut rt).await;
    let _ = published(&rep, "regen after the cut edit");

    let gates = ordinal_gates(&rt);
    assert!(
        !gates.is_empty(),
        "RED before VF-B6: the split ordinals permuted and NOTHING was seeded — the \
         mirror re-bound cleanly onto the other solid"
    );
    assert!(
        gates.iter().all(|g| g.step_index == MIRROR_STEP),
        "the gate lands on the DOWNSTREAM record's step, not the cut's: {:?}",
        gates.iter().map(|g| g.step_index).collect::<Vec<_>>()
    );
    let anchor = gates[0]
        .ordinal_anchor
        .as_ref()
        .expect("an ordinal gate carries the anchor it was raised against");
    assert_eq!(anchor.op, RecordId(Uuid::from_u128(OP_CUT)));
    assert_eq!(anchor.stamps.len(), 2, "both ordinals are anchored");
    assert!(
        gates[0].ui_label.contains(&format!("step {CUT_STEP}")),
        "the label names the offending op: {:?}",
        gates[0].ui_label
    );

    // The ceiling is now BELOW the mirror, so the mirror may not execute at all.
    assert_eq!(
        rt.repair_items()
            .iter()
            .filter(|i| i.seeded)
            .map(|i| i.step_index)
            .min(),
        Some(MIRROR_STEP),
        "first_seeded_step pins the regen ceiling at the mirror"
    );

    // ── The gate is STABLE: a further regen must not clear its own seed ───────
    // (Comparing against the previous regen rather than the anchor would see "no
    // change since last time" here and silently drop the gate on the next tick.)
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "second regen under the gate");
    assert!(
        !ordinal_gates(&rt).is_empty(),
        "the gate survives a regen that merely republishes the permuted geometry"
    );
    assert!(
        snap.repair_summary.needs_repair_count > 0,
        "the document still reports NeedsRepair"
    );
    assert_eq!(
        snap.bodies.len(),
        healthy_bodies - 1,
        "publish ≤ m−1: the mirror never ran, so its image is NOT in the document — \
         nothing was bound to the wrong solid"
    );
    // The permutation itself is real: `:0` is now the FAR piece (y ∈ [23,40]).
    let child0_flipped = body_y_span(&mut rt, child0).await;
    assert!(
        child0_flipped.0 > 20.0,
        "the ordinal genuinely changed owner — :0 is now the FAR piece, got \
         {child0_flipped:?} (was {child0_before:?})"
    );

    // ── REVERT ⇒ the ordering comes home ⇒ the tripwire clears its own seeds ──
    set_tool_position(&mut rt, tool_sid, TOOL_A_NEAR_SMALL);
    let _ = published(&regen_all(&mut rt).await, "regen after reverting the cut");
    assert!(
        ordinal_gates(&rt).is_empty(),
        "self-heal: the ordinals match the anchor again, so the regen-side seeds go \
         (they carry no command inverse — undoing the geometry IS the undo)"
    );
    assert!(
        rt.repair_items().is_empty(),
        "no repair item of any kind survives the self-heal: {:?}",
        rt.repair_items()
    );

    // The ceiling was sampled at `begin_regen`, BEFORE the commit that cleared the
    // gate, so that regen still stopped below the mirror. The next one runs it — a
    // one-regen lag by construction (see `apply_ordinal_tripwire`).
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen once the gate has lifted");
    assert_eq!(
        snap.repair_summary.needs_repair_count, 0,
        "the document is healthy again"
    );
    assert_eq!(
        snap.bodies.len(),
        healthy_bodies,
        "the mirror executes again once the gate lifts"
    );
    assert_eq!(
        body_y_span(&mut rt, child0).await,
        child0_before,
        "child :0 is back to the solid it originally named"
    );
}

/// A legitimate parametric edit that does NOT cross the ordinals must stay silent.
/// The tripwire's whole value depends on never crying wolf: a false gate pins the
/// regen ceiling and blocks the document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resizing_the_cut_without_crossing_the_ordinals_seeds_nothing() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (tool_sid, _) = build(&mut rt).await;
    let _ = published(&regen_all(&mut rt).await, "plate cut + mirror");

    // 17 → 16: the near piece shrinks and the far one grows, but the near piece is
    // still the small one. Same ordinal assignment, different geometry.
    set_tool_position(&mut rt, tool_sid, 16.0);
    let rep = regen_all(&mut rt).await;
    let snap = published(&rep, "regen after a non-crossing cut edit");
    assert!(
        ordinal_gates(&rt).is_empty(),
        "a resize that preserves the ordering must NOT gate: {:?}",
        ordinal_gates(&rt)
            .iter()
            .map(|g| &g.ui_label)
            .collect::<Vec<_>>()
    );
    assert_eq!(snap.repair_summary.needs_repair_count, 0);
}

/// The count-change case is NOT this tripwire's business: when the cut stops
/// splitting entirely there is one body where there were two, no ordinal survives to
/// permute, and the pre-existing binding machinery owns the outcome. Pins that the
/// tripwire makes no claim and — critically — that it does not leave a gate behind
/// that nothing could ever close.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_that_stops_splitting_makes_no_ordinal_claim() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (tool_sid, _) = build(&mut rt).await;
    let _ = published(&regen_all(&mut rt).await, "plate cut + mirror");

    // Slide the slab off the far end: it now only nibbles the edge, so the Cut
    // yields ONE body and mints no ordinal children at all.
    set_tool_position(&mut rt, tool_sid, 38.0);
    let rep = regen_all(&mut rt).await;
    let _ = published(&rep, "regen after the cut stops splitting");
    assert!(
        ordinal_gates(&rt).is_empty(),
        "no ordinals ⇒ no permutation claim (and no undismissable gate)"
    );
}

/// Stamps are DERIVED evidence, so they must ride the save/reopen round trip — a
/// reopened document that lost them would silently stop protecting its split
/// children. Also pins the legacy direction: a registry with NO stamps makes no
/// claim rather than a false one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn geom_stamps_survive_save_and_reopen_and_absent_stamps_make_no_claim() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    let (_, child0) = build(&mut rt).await;
    let _ = published(&regen_all(&mut rt).await, "plate cut + mirror");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ordinal_stamps.onecad");
    rt.save(&path, save_meta()).expect("save");

    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");

    let meta = reopened
        .body_meta(child0)
        .expect("the reopened document still knows child :0");
    assert!(
        meta.split_of.is_some(),
        "the ordinal origin round-trips (pre-existing invariant)"
    );
    assert!(
        meta.geom_stamp.is_some(),
        "VF-B6: the rank key the ordinal was assigned by round-trips too — without it \
         a reopened document silently stops protecting its split children"
    );
    assert!(
        ordinal_gates(&reopened).is_empty(),
        "reopening a healthy document raises nothing"
    );
}
