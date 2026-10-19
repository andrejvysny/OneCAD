//! Native Arc, Slot and Line->tangent-arc authoring through the real application
//! services and production PlaneGCS/OCCT worker. Service evidence, not native input.
#[path = "real_arc_slot/flows.rs"]
mod flows;
#[path = "real_arc_slot/geom.rs"]
mod geom;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "real_point_center/support.rs"]
mod support;

use flows::{arc_flow, assert_arcs, assert_chain, chain_flow};
use geom::{arc, assert_no_orphans, line, line_gap, tangents, welds};
use host::Harness;
use onecad_app::{
    api,
    dto::{DocumentProjection, FinishSketchDto, SketchSolveStatus},
};
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
        },
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::{EditCommand, HistoryDirection},
    ids::{ConstraintId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Constraint, Sketch, SketchEntity, WorldPlane},
};
use onecad_interaction::{sketch::*, DocumentContext};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, ViewSnapshot, Viewport};
use std::path::{Path, PathBuf};
use support::*;
use uuid::Uuid;

fn sketch_id() -> SketchId {
    SketchId(Uuid::from_u128(0x2a000))
}
fn extrude_id() -> RecordId {
    RecordId(Uuid::from_u128(0x2a001))
}
fn xy(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}

#[test]
fn slot_dof_residuals_width_history_extrude_and_fresh_reopen() {
    isolated(
        "slot_dof_residuals_width_history_extrude_and_fresh_reopen",
        "slot",
        |r, p| {
            Box::pin(async move {
                if p == "slot-reopen" {
                    reopen(&r, "slot", 2, 6.0, assert_slot).await;
                } else {
                    slot_flow(&r).await;
                }
            })
        },
    );
}

#[test]
fn center_arc_radius_edit_history_numeric_and_fresh_reopen() {
    isolated(
        "center_arc_radius_edit_history_numeric_and_fresh_reopen",
        "arc",
        |r, p| {
            Box::pin(async move {
                if p == "arc-reopen" {
                    reopen(&r, "arc", 1, 14.0, assert_arcs).await;
                } else {
                    arc_flow(&r).await;
                }
            })
        },
    );
}

#[test]
fn tangent_chain_profile_extrude_and_fresh_reopen() {
    isolated(
        "tangent_chain_profile_extrude_and_fresh_reopen",
        "chain",
        |r, p| {
            Box::pin(async move {
                if p == "chain-reopen" {
                    reopen(&r, "chain", 2, 0.0, |s, _| assert_chain(s)).await;
                } else {
                    chain_flow(&r).await;
                }
            })
        },
    );
}

fn isolated<F>(name: &str, phase: &str, run: F)
where
    F: FnOnce(PathBuf, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>,
{
    host::required_worker();
    if let Ok(current) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(run(root, current));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-arc-slot-")
        .tempdir()
        .unwrap();
    host::run_isolated_phase(root.path(), phase, name);
    host::run_isolated_phase(root.path(), &format!("{phase}-reopen"), name);
}

async fn new(root: &Path, name: &str) -> Harness {
    let harness = Harness::new(&root.join(name)).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    ready(&harness).await;
    harness
}

async fn ready(harness: &Harness) {
    let worker = harness.state.live_worker().unwrap();
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
}

async fn depth(harness: &Harness) -> usize {
    harness.projection().await.undo_depth
}

/// Submit one primitive/constraint batch; exactly one history entry results.
async fn once(
    harness: &Harness,
    state: &mut SketchInteraction,
    effects: Vec<SketchEffect>,
) -> onecad_app::dto::SketchUpsertDto {
    let before = depth(harness).await;
    let solve = submit(harness, state, effects).await;
    assert!(solve.conflicting.is_empty(), "{:?}", solve.conflicting);
    assert_eq!(depth(harness).await, before + 1, "one history entry");
    solve
}

fn click_at(state: &mut SketchInteraction, p: Vec2, seed: u128) -> Vec<SketchEffect> {
    let point = captured(state, p);
    state.dispatch(SketchAction::Click {
        point,
        ids: ids(seed),
    })
}

async fn add(harness: &Harness, state: &mut SketchInteraction, c: Vec<Constraint>, dof: u32) {
    let key = state.session.as_ref().unwrap().key.clone();
    let effects = state.dispatch(SketchAction::AddConstraints {
        key,
        constraints: c,
    });
    let solve = once(harness, state, effects).await;
    assert_eq!(solve.dof, dof);
}

/// Edit a dimension, check geometry, Undo restores bit-exact, Redo reapplies.
async fn edit_with_history(
    harness: &Harness,
    state: &mut SketchInteraction,
    constraint: ConstraintId,
    value: f64,
    check: impl Fn(&Sketch),
) {
    let before = sketch(harness).await;
    let key = state.session.as_ref().unwrap().key.clone();
    let value = Scalar::new(value);
    let effects = state.dispatch(SketchAction::SetDimension {
        key,
        constraint,
        value,
    });
    once(harness, state, effects).await;
    let edited = sketch(harness).await;
    check(&edited);
    history(harness, state, HistoryDirection::Undo).await;
    assert_eq!(sketch(harness).await, before);
    history(harness, state, HistoryDirection::Redo).await;
    let redone = sketch(harness).await;
    check(&redone);
    let (a, b) = (serde_json::to_value(&redone), serde_json::to_value(&edited));
    assert!(geom::json_close(&a.unwrap(), &b.unwrap(), 1e-9));
}

fn assert_slot(sketch: &Sketch, radius: f64) {
    let p = ids(1000);
    assert_eq!(sketch.entities().len(), 10);
    assert_eq!(welds(sketch), 4);
    let t = tangents(sketch);
    assert_eq!(t.len(), 4);
    assert_no_orphans(sketch);
    let (a, b) = (arc(sketch, p.entities[8]), arc(sketch, p.entities[9]));
    assert!(
        (a.1 - radius).abs() <= 1e-6 && (b.1 - radius).abs() <= 1e-6,
        "{a:?} {b:?}"
    );
    assert!((a.1 - b.1).abs() <= 1e-6);
    let (w0, w1) = (line(sketch, p.entities[6]), line(sketch, p.entities[7]));
    let gap = line_gap(w0.0, w0.1, w1.0);
    assert!((gap - 2.0 * radius).abs() <= 1e-6, "walls {gap}");
}

async fn slot_flow(root: &Path) {
    let harness = new(root, "host").await;
    let mut state = begin(&harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::Slot));
    assert!(click_at(&mut state, xy(0.0, 0.0), 500).is_empty());
    assert!(click_at(&mut state, xy(30.0, 0.0), 500).is_empty());
    let effects = click_at(&mut state, xy(15.0, 5.0), 1000);
    let solve = once(&harness, &mut state, effects).await;
    eprintln!("slot bare dof {} {:?}", solve.dof, solve.status);
    assert_eq!(solve.dof, 5);
    assert_eq!(solve.status, SketchSolveStatus::UnderConstrained);
    assert_slot(&sketch(&harness).await, 5.0);
    regions(&harness, 1).await;
    let diameter = Constraint::Diameter {
        id: ids(1000).constraints[9],
        entity: ids(1000).entities[8],
        value: Scalar::new(10.0),
    };
    add(&harness, &mut state, vec![diameter], 4).await;
    assert_slot(&sketch(&harness).await, 5.0);
    let c = ids(1000).constraints[9];
    edit_with_history(&harness, &mut state, c, 12.0, |s| assert_slot(s, 6.0)).await;
    finish_extrude_save(&harness, &mut state, root, "slot").await;
    harness.close().await;
}

async fn finish(harness: &Harness, state: &mut SketchInteraction) -> FinishSketchDto {
    let mut effects = state.dispatch(SketchAction::Finish);
    let SketchEffect::FinishSketch { tag, .. } = effects.remove(0) else {
        panic!("expected finish effect")
    };
    close_visit(harness).await;
    let key = tag.key.sketch_id.to_string();
    let dto = api::finish_sketch(&harness.state, harness.host.clone(), key)
        .await
        .unwrap();
    harness.settled(&harness.projection().await).await;
    assert_eq!(dto.regions.len(), 1);
    assert!(dto.regions[0].holes.is_empty());
    state.dispatch(SketchAction::Finished {
        tag,
        result: Box::new(dto.clone()),
    });
    dto
}

fn extrude(regions: &FinishSketchDto, depth: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id(),
            region: RegionId::new(&regions.regions[0].region_id),
            region_identity_version: Some(regions.region_identity_version),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(depth),
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
    }))
}

async fn commit(harness: &Harness, operation: Operation) -> DocumentProjection {
    let record = OperationRecord::new(extrude_id(), 1, "Arc/slot profile extrusion", operation);
    let command = EditCommand::AddOperation {
        record,
        at_cursor: true,
    };
    let effects = Some(api::OperationEffects {
        hide_sketch_ids: vec![sketch_id().to_string()],
    });
    let response =
        api::apply_operation(&harness.state, harness.host.clone(), vec![command], effects)
            .await
            .unwrap();
    harness.settled(&response.projection).await
}

/// Finish, re-enter, extrude 4 mm, record the sketch, Save.
async fn finish_extrude_save(
    harness: &Harness,
    state: &mut SketchInteraction,
    root: &Path,
    name: &str,
) {
    let saved = sketch(harness).await;
    let regions = finish(harness, state).await;
    let again = begin(harness, false).await;
    let entered = again.session.as_ref().unwrap().authoritative.clone();
    let (a, b) = (serde_json::to_value(&entered), serde_json::to_value(&saved));
    assert!(geom::json_close(&a.unwrap(), &b.unwrap(), 1e-9));
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    assert!(solve.solve_current);
    close_visit(harness).await;
    let before = harness.projection().await;
    let after = commit(harness, extrude(&regions, 4.0)).await;
    assert_eq!(after.undo_depth, before.undo_depth + 1);
    assert_eq!(after.bodies.len(), 1, "extrusion committed a body");
    record_and_save(harness, root, name).await;
}

/// Persist the solved sketch + DOF for the fresh-process phase, then Save.
async fn record_and_save(harness: &Harness, root: &Path, name: &str) {
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    let record = serde_json::json!({ "sketch": sketch(harness).await, "dof": solve.dof });
    std::fs::write(root.join(format!("{name}.json")), record.to_string()).unwrap();
    let path = root.join(format!("{name}.onecad"));
    let done = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(done.clean);
}

/// Fresh worker: same IDs and geometry <= 1e-9, same DOF, same body.
async fn reopen(root: &Path, name: &str, features: usize, radius: f64, check: fn(&Sketch, f64)) {
    let harness = Harness::new(&root.join("reopen-host")).await;
    let path = root.join(format!("{name}.onecad"));
    api::open_document(
        &harness.state,
        harness.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    ready(&harness).await;
    let projection = harness.projection().await;
    assert!(!projection.dirty);
    assert_eq!(projection.features.len(), features);
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap()).unwrap();
    let state = begin(&harness, false).await;
    let reopened = state.session.as_ref().unwrap().authoritative.clone();
    assert!(geom::json_close(
        &serde_json::to_value(&reopened).unwrap(),
        &record["sketch"],
        1e-9
    ));
    check(&reopened, radius);
    let solve = api::get_sketch(&harness.state, sketch_id().to_string())
        .await
        .unwrap();
    assert!(solve.solve_current);
    assert_eq!(u64::from(solve.dof), record["dof"].as_u64().unwrap());
    close_visit(&harness).await;
    harness.close().await;
}
