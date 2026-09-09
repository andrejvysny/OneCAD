//! **`FILLET_BLEND_APPROXIMATED`** (SCHEMA §7.2 warning row / §7.3 Fillet "Two
//! blend classes, two budgets", kernel-hardening WP-G) survives the wire into
//! the document's step diagnostics, driven through the app's [`DocumentRuntime`]
//! against the REAL C++ OCCT worker — mirroring `topology_rebind.rs` /
//! `chamfer_reference.rs` and the canonical NDJSON fixture
//! `protocol/fixtures/fillet_approximated_blend.ndjson`.
//!
//! * `approximated_blend_warning_reaches_the_document` — the same tee-seam
//!   geometry as the fixture: a Ø40 shaft along X (a r=20 circle on XZ,
//!   extruded 60 symmetric) fused with a Ø20 boss along +Z (a r=10 circle on
//!   XY, extruded 40 blind, `Add` onto the shaft). The boss's base sits on the
//!   shaft's axis and stands 20 mm proud. The tee seam is a cylinder/cylinder
//!   tangent pair OCCT splits into `e:1` + `e:4`; a chain-tangent fillet r=2
//!   there is APPROXIMATED and must publish with exactly one
//!   `FILLET_BLEND_APPROXIMATED` warning carrying the SCHEMA-pinned evidence
//!   keys.
//! * `analytic_blend_carries_no_class_warning` — a 40×20×10 box, fillet r=2 on
//!   a vertical (plane/plane) edge. ANALYTIC: publishes silent.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing
//! binary skips cleanly (CI sets `ONECAD_REQUIRE_WORKER=1` to make that a hard
//! failure).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ElementId, EntityId, RecordId, RegionId, SketchId, SnapshotId};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{ElementQuery, MeshProvider, SolverEngine, WorkerManager};

use onecad_protocol::mesh::validate_mesh_blob;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors chamfer_reference.rs / topology_rebind.rs)
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
    if let Some(path) = onecad_lib::worker::resolve_worker_path() {
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

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op record builders
// ─────────────────────────────────────────────────────────────────────────────

fn plane_ref(kind: PlaneKind, plane: &onecad_core::sketch::SketchPlane) -> SketchPlaneRef {
    SketchPlaneRef {
        kind,
        origin: plane.origin,
        x_axis: plane.x_axis,
        y_axis: plane.y_axis,
        normal: plane.normal,
        extra: Default::default(),
    }
}

/// A single-circle sketch (radius `r`, centred at the plane origin) on the
/// given world plane — same-shape geometry as `protocol/fixtures/fillet_approximated_blend.ndjson`.
fn circle_sketch(sid: SketchId, world: WorldPlane, r: f64) -> Sketch {
    let mut sk = Sketch::on_world_plane(sid, "Circle", world);
    let centre = EntityId(Uuid::new_v4());
    sk.add_entity(SketchEntity::point(
        centre,
        Vec2::new_unchecked(0.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    let circ = EntityId(Uuid::new_v4());
    sk.add_entity(SketchEntity::circle(circ, centre, r, false).expect("circle"))
        .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch, kind: PlaneKind) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let plane = plane_ref(kind, &sk.plane);
    let params = SketchOpParams {
        sketch: sk.id,
        plane,
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

#[allow(clippy::too_many_arguments)]
fn extrude_record(
    rec: u128,
    sketch: SketchId,
    dist: f64,
    mode: ExtrudeMode,
    boolean_mode: BooleanMode,
    target_body: Option<BodyId>,
) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        1,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            // First-region fallback (V1) — each sketch here has exactly one
            // circle/rectangle region.
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(dist),
            draft_angle_deg: Scalar::new(0.0),
            mode,
            boolean_mode,
            target_body,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        })),
    )
}

/// A fillet on one or more edges, each carried as a typed [`ElementRef`]
/// (Rust-minted `ElementId` + a world-point anchor) — SCHEMA §7.3 `inputs[]`
/// shape.
fn fillet_record(
    rec: u128,
    body: BodyId,
    edges: &[(ElementId, Vec3)],
    radius: f64,
    chain_tangent: bool,
) -> OperationRecord {
    let refs: Vec<ElementRef> = edges
        .iter()
        .map(|(el, anchor)| ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: el.clone(),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: *anchor,
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        })
        .collect();
    let edge_ids: Vec<ElementId> = edges.iter().map(|(el, _)| el.clone()).collect();
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        2,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids,
            edges: refs,
            chain_tangent_edges: chain_tangent,
            tangent_closure_version: chain_tangent.then_some(1),
            extra: Default::default(),
        })),
    )
}

/// Promotes a snapshot-scoped `TopoKey` (e.g. `"e:1"`) into a Rust-minted
/// `ElementId`, using the worker's own reported centre as the anchor — same
/// pattern as `topology_rebind.rs::build_filleted_box`'s `ref_face` pick.
async fn promote_edge(
    rt: &mut DocumentRuntime,
    wm: &WorkerManager,
    snap: SnapshotId,
    body: BodyId,
    topo_key: &str,
) -> (ElementId, Vec3) {
    let info = ElementQuery::query_element_by_topo_key(wm, snap, body, topo_key)
        .await
        .expect("QueryElement by topoKey")
        .unwrap_or_else(|| panic!("edge {topo_key} resolves on its own snapshot"));
    let anchor = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
    let promoted = rt
        .promote_selection(
            snap,
            body,
            vec![(
                onecad_core::ids::TopoKey::new(topo_key),
                Some(AnchorIntent {
                    world_point: anchor,
                    surface_uv: None,
                    local_frame: None,
                    adjacency_hint: None,
                    extra: Default::default(),
                }),
            )],
        )
        .await
        .unwrap_or_else(|e| panic!("promote {topo_key}: {e}"));
    (ElementId::new(promoted[0].element_id.clone()), anchor)
}

// ─────────────────────────────────────────────────────────────────────────────
// (1) APPROXIMATED — the cylinder/cylinder tee seam.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn approximated_blend_warning_reaches_the_document() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary resolved");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sk1 = SketchId(Uuid::from_u128(0x1));
    let sk2 = SketchId(Uuid::from_u128(0x2));

    // op0/op1 — Ø40 shaft along X: r=20 circle on XZ, extruded 60 symmetric,
    // NewBody.
    add_op(
        &mut rt,
        sketch_record(
            0xA000,
            &circle_sketch(sk1, WorldPlane::XZ, 20.0),
            PlaneKind::Xz,
        ),
    );
    add_op(
        &mut rt,
        extrude_record(
            0xA001,
            sk1,
            60.0,
            ExtrudeMode::Symmetric,
            BooleanMode::NewBody,
            None,
        ),
    );
    let shaft_report = regen_all(&mut rt).await;
    let shaft_snap = published(&shaft_report, "shaft extrude").clone();
    let body = shaft_report.changed[0].0;

    // op2/op3 — Ø20 boss along +Z: r=10 circle on XY, extruded 40 blind, Add
    // onto the shaft. Its base sits on the shaft's axis (z=0 at x=y=0), so it
    // stands 20 mm proud of the Ø40 shaft's surface.
    add_op(
        &mut rt,
        sketch_record(
            0xA002,
            &circle_sketch(sk2, WorldPlane::XY, 10.0),
            PlaneKind::Xy,
        ),
    );
    add_op(
        &mut rt,
        extrude_record(
            0xA003,
            sk2,
            40.0,
            ExtrudeMode::Blind,
            BooleanMode::Add,
            Some(body),
        ),
    );
    let fused_report = regen_all(&mut rt).await;
    let fused_snap = published(&fused_report, "boss fused onto shaft").clone();
    let snap_id = SnapshotId(fused_report.snapshot_id);
    let _ = shaft_snap; // pre-fuse snapshot kept only for eprintln context below

    let fmesh = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("mesh the fused body");
    let fview = validate_mesh_blob(&fmesh).expect("fused MESH1 validates");
    eprintln!(
        "fillet_blend_class: fused body faces={} snapshotId={}",
        fview.face_count, fused_snap.id.0
    );

    // The tee seam OCCT splits into the tangent pair `e:1` + `e:4` (same as
    // `protocol/fixtures/fillet_approximated_blend.ndjson`) — promote both.
    let (edge_a, anchor_a) = promote_edge(&mut rt, &wm, snap_id, body, "e:1").await;
    let (edge_b, anchor_b) = promote_edge(&mut rt, &wm, snap_id, body, "e:4").await;

    add_op(
        &mut rt,
        fillet_record(
            0xA004,
            body,
            &[(edge_a, anchor_a), (edge_b, anchor_b)],
            2.0,
            true,
        ),
    );
    let fillet_report = regen_all(&mut rt).await;
    let _ = published(&fillet_report, "tee-seam fillet");

    let diagnostics: Vec<_> = rt
        .projection()
        .features
        .iter()
        .flat_map(|f| f.diagnostics.iter())
        .cloned()
        .collect();
    eprintln!("fillet_blend_class: step diagnostics = {diagnostics:#?}");

    let matches: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.code == "FILLET_BLEND_APPROXIMATED")
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one FILLET_BLEND_APPROXIMATED diagnostic, got {matches:#?}"
    );
    let diag = matches[0];
    assert_eq!(diag.severity, onecad_core::regen::engine::Severity::Warning);
    let evidence = diag
        .evidence
        .as_ref()
        .expect("FILLET_BLEND_APPROXIMATED carries evidence");
    eprintln!("fillet_blend_class: evidence = {evidence:#}");

    assert_eq!(
        evidence["blendSurfaceClass"],
        serde_json::json!("approximated")
    );
    let tol = evidence["approximationTolerance"]
        .as_f64()
        .expect("approximationTolerance is a finite number");
    assert!(tol.is_finite() && tol > 0.0, "got {tol}");

    let max_err = evidence["maximumSectionRadiusError"]
        .as_f64()
        .expect("maximumSectionRadiusError is a number");
    let allowed_err = evidence["allowedSectionRadiusError"]
        .as_f64()
        .expect("allowedSectionRadiusError is a number");
    assert!(max_err.is_finite());
    assert!(
        max_err <= allowed_err,
        "the fillet published, so the section budget must have been met: \
         maximumSectionRadiusError={max_err} allowedSectionRadiusError={allowed_err}"
    );

    for key in [
        "maximumTangencyRadians",
        "allowedTangencyRadians",
        "approximatedContours",
        "approximatedBlendFaces",
        "maxContourVertexValence",
    ] {
        assert!(
            !evidence[key].is_null(),
            "evidence must carry {key}, got {evidence:#}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (2) ANALYTIC — a plane/plane box edge.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn analytic_blend_carries_no_class_warning() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary resolved");
        return;
    };
    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);

    let sid = SketchId(Uuid::from_u128(0x10));
    let mut sk = Sketch::on_world_plane(sid, "Box", WorldPlane::XY);
    let base = 0x2000u128;
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let pts = [(0.0, 0.0), (40.0, 0.0), (40.0, 20.0), (0.0, 20.0)];
    for (i, (x, y)) in pts.iter().enumerate() {
        sk.add_entity(SketchEntity::point(
            e(i as u128),
            Vec2::new_unchecked(*x, *y),
            false,
            false,
        ))
        .unwrap();
    }
    for i in 0..4u128 {
        let a = e(i);
        let b = e((i + 1) % 4);
        sk.add_entity(SketchEntity::line(e(0x10 + i), a, b, false))
            .unwrap();
    }
    add_op(&mut rt, sketch_record(0xB000, &sk, PlaneKind::Xy));
    add_op(
        &mut rt,
        extrude_record(
            0xB001,
            sid,
            10.0,
            ExtrudeMode::Blind,
            BooleanMode::NewBody,
            None,
        ),
    );
    let report = regen_all(&mut rt).await;
    let _ = published(&report, "box extrude");
    let body = report.changed[0].0;
    let snap_id = SnapshotId(report.snapshot_id);

    // A vertical (plane/plane) edge of the box — analytic.
    let (edge_el, anchor) = promote_edge(&mut rt, &wm, snap_id, body, "e:1").await;

    add_op(
        &mut rt,
        fillet_record(0xB002, body, &[(edge_el, anchor)], 2.0, false),
    );
    let fillet_report = regen_all(&mut rt).await;
    let _ = published(&fillet_report, "box edge fillet");

    let diagnostics: Vec<_> = rt
        .projection()
        .features
        .iter()
        .flat_map(|f| f.diagnostics.iter())
        .cloned()
        .collect();
    eprintln!("fillet_blend_class: analytic step diagnostics = {diagnostics:#?}");

    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == "FILLET_BLEND_APPROXIMATED"),
        "an analytic blend must not carry FILLET_BLEND_APPROXIMATED, got {diagnostics:#?}"
    );
}
