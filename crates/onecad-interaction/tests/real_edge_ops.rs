//! Fillet, Chamfer, Offset Face and Move / Rotate creation through the shared
//! binder and reducer with the real worker (ONECAD-316/319; IP13–IP15) on the
//! 60 × 40 × 4 mm reference plate with one Ø6 mm through hole, then parameter
//! edit, undo/redo and save/reopen in a fresh AppState. Automated service
//! evidence, not native input.
#[path = "real_edge_ops/drive.rs"]
mod drive;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;
#[path = "real_edge_ops/plate.rs"]
mod plate;

use drive::close;
use host::Harness;
use onecad_app::{api, dto::DocumentProjection, worker::wire::body_id_wire};
use onecad_core::{
    document::{record::*, variables::Scalar},
    ids::{BodyId, RecordId},
};
use onecad_interaction::{
    operand_binding::{self, EdgeOpKind, DEFAULT_OPERATION_SIZE_MM},
    *,
};
use std::{
    f64::consts::PI,
    path::{Path, PathBuf},
};

/// Undo/redo, rigid motion and reopen rebuild the same solid.
const SAME: f64 = 1e-9;
/// Feature volume deltas against their closed forms.
const DELTA: f64 = 1e-4;
/// Mesh bounds are f32 vertex positions.
const MESH: f64 = 1e-5;

#[test]
fn edge_face_and_body_operations_create_edit_undo_and_reopen() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("edge-ops") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(workflow(&root));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-edge-ops-")
        .tempdir()
        .unwrap();
    host::run_isolated_phase(
        root.path(),
        "edge-ops",
        "edge_face_and_body_operations_create_edit_undo_and_reopen",
    );
}

/// What the reopened document must reproduce.
struct Authored {
    body: BodyId,
    fillet: RecordId,
    chamfer: RecordId,
    offset: RecordId,
    moved: RecordId,
    rotated: RecordId,
    volume: f64,
    op_types: Vec<String>,
}

async fn workflow(root: &Path) {
    let saved = root.join("edge-ops.onecad");
    let authored = author(&root.join("author-host"), &saved).await;
    reopen(&root.join("reopen-host"), &saved, &authored).await;
}

async fn author(root: &Path, saved: &Path) -> Authored {
    let harness = Harness::new(root).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let worker = harness.state.live_worker().unwrap();
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let (body, _) = plate::build(&harness).await;
    let base = drive::volume(&harness, body).await;
    let expected = plate::volume();
    close(
        base,
        expected,
        expected,
        1e-6,
        "plate volume 9600 − 36π mm³",
    );
    let mut state = Interaction::default();
    let (fillet, filleted) = fillet_edit_and_history(&harness, &mut state, body, base).await;
    let (chamfer, chamfered) = chamfer(&harness, &mut state, body, filleted).await;
    let (offset, volume) = offset_top(&harness, &mut state, body, chamfered).await;
    let (moved, rotated) = move_and_rotate(&harness, &mut state, body, volume).await;
    let op_types = harness
        .projection()
        .await
        .features
        .iter()
        .map(|feature| feature.op_type.clone())
        .collect();
    save(&harness, saved).await;
    harness.close().await;
    Authored {
        body,
        fillet,
        chamfer,
        offset,
        moved,
        rotated,
        volume,
        op_types,
    }
}

/// The vertical plate edge at corner (x, y), by its snapshot TopoKey.
async fn corner_edge(harness: &Harness, body: BodyId, x: f64, y: f64) -> String {
    let edges = drive::elements(harness, body, "e").await;
    let mut matching = edges.iter().filter(|edge| {
        edge.curve_type == 0
            && (edge.magnitude - plate::THICKNESS).abs() < 1e-6
            && (edge.center[0] - x).abs() < 1e-6
            && (edge.center[1] - y).abs() < 1e-6
    });
    let edge = matching.next().expect("vertical corner edge");
    assert!(matching.next().is_none(), "one edge per corner");
    edge.topo_key.clone()
}

/// Staging mints identity without dirtying or moving the head.
async fn staged_cleanly(harness: &Harness, before: &DocumentProjection) {
    let after = harness.projection().await;
    assert_eq!(
        (after.revision, after.dirty, after.undo_depth),
        (before.revision, before.dirty, before.undo_depth)
    );
}

/// Begins a bound record at its default value and runs it to a ready preview.
async fn begin(
    harness: &Harness,
    state: &mut Interaction,
    record: OperationRecord,
    gesture: u64,
    context: DocumentContext,
) -> RecordId {
    let id = record.record_id;
    let action = operand_binding::begin_bound(record, gesture, context).unwrap();
    let effects = state.dispatch(action);
    drive::drive(harness, state, effects).await;
    let tool = state
        .tool
        .as_ref()
        .expect("the reducer accepted the record");
    assert_eq!(tool.preview, PreviewStatus::Ready);
    id
}

/// Stages one picked edge the way the native host does, binds it and begins.
async fn begin_edge_op(
    harness: &Harness,
    state: &mut Interaction,
    kind: EdgeOpKind,
    (body, key): (BodyId, String),
    gesture: u64,
) -> RecordId {
    let context = drive::adopt(harness, state).await;
    let head = harness.projection().await;
    let scope = api::preview_context(&harness.state).await.unwrap();
    let pick = api::EdgeOpPickInput {
        body_id: Some(body_id_wire(body)),
        topo_key: Some(key),
        element_id: None,
    };
    let staged = api::stage_edge_op(&harness.state, &scope, kind.mode(), vec![pick], true)
        .await
        .unwrap();
    assert_eq!(
        staged.refs.len(),
        1,
        "a plate corner edge chains to nothing"
    );
    staged_cleanly(harness, &head).await;
    let size = Scalar::new(DEFAULT_OPERATION_SIZE_MM);
    let step = head.applied_ops as u32;
    let record = operand_binding::bind_edge_op(kind, &staged, size, RecordId::new(), step).unwrap();
    begin(harness, state, record, gesture, context).await
}

async fn fillet_edit_and_history(
    harness: &Harness,
    state: &mut Interaction,
    body: BodyId,
    base: f64,
) -> (RecordId, f64) {
    let key = corner_edge(harness, body, 0.0, 0.0).await;
    let id = begin_edge_op(harness, state, EdgeOpKind::Fillet, (body, key), 1).await;
    let created = drive::confirm(harness, state).await;
    let one = drive::volume(harness, body).await;
    let removed = (1.0 - PI / 4.0) * plate::THICKNESS;
    close(
        base - one,
        removed,
        removed,
        DELTA,
        "1 mm fillet removes (1 − π/4)·4",
    );
    unchanged_edit_emits_nothing(harness, state, id, &created).await;
    let edited = edit_fillet(harness, state, id).await;
    assert_eq!(edited.undo_depth, created.undo_depth + 1);
    let half = drive::volume(harness, body).await;
    let quarter = removed / 4.0;
    close(
        base - half,
        quarter,
        quarter,
        DELTA,
        "0.5 mm fillet removes (1 − π/4)·1",
    );
    for (redo, expected, what) in [
        (false, one, "undo restores the 1 mm fillet"),
        (false, base, "undo removes the fillet"),
        (true, one, "redo restores the 1 mm fillet"),
        (true, half, "redo restores the 0.5 mm edit"),
    ] {
        history(harness, redo).await;
        close(
            drive::volume(harness, body).await,
            expected,
            expected,
            SAME,
            what,
        );
    }
    (id, half)
}

async fn history(harness: &Harness, redo: bool) -> DocumentProjection {
    let projection = if redo {
        api::redo(&harness.state, harness.host.clone()).await
    } else {
        api::undo(&harness.state, harness.host.clone()).await
    };
    harness.settled(&projection.unwrap()).await
}

/// Re-edit through the history path, starting from the stored record.
async fn begin_fillet_edit(harness: &Harness, state: &mut Interaction, id: RecordId, gesture: u64) {
    let context = drive::adopt(harness, state).await;
    let record = api::get_operation_record(&harness.state, id).await.unwrap();
    let effects = state.dispatch(Action::BeginParameterEdit {
        record: Box::new(record),
        parameter: NumericParameter::FilletRadius,
        gesture,
        context,
    });
    drive::drive(harness, state, effects).await;
    assert!(state.tool.as_ref().is_some_and(ModelSession::ready));
}

async fn unchanged_edit_emits_nothing(
    harness: &Harness,
    state: &mut Interaction,
    id: RecordId,
    head: &DocumentProjection,
) {
    begin_fillet_edit(harness, state, id, 2).await;
    let effects = state.dispatch(Action::Confirm);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ApplyOperation { .. })),
        "{effects:?}"
    );
    assert!(state.tool.is_none());
    let after = harness.projection().await;
    assert_eq!(
        (after.revision, after.undo_depth),
        (head.revision, head.undo_depth)
    );
}

async fn edit_fillet(
    harness: &Harness,
    state: &mut Interaction,
    id: RecordId,
) -> DocumentProjection {
    begin_fillet_edit(harness, state, id, 3).await;
    drive::typed(harness, state, NumericParameter::FilletRadius.into(), "0.5").await;
    let edited = drive::confirm(harness, state).await;
    let stored = api::get_operation_record(&harness.state, id).await.unwrap();
    let Operation::Known(KnownOperation::Fillet(params)) = &stored.op else {
        panic!("fillet record expected")
    };
    assert_eq!(params.radius.value, 0.5);
    assert_eq!(params.tangent_closure_version, Some(1));
    edited
}

async fn chamfer(
    harness: &Harness,
    state: &mut Interaction,
    body: BodyId,
    before: f64,
) -> (RecordId, f64) {
    let key = corner_edge(harness, body, plate::WIDTH, 0.0).await;
    let id = begin_edge_op(harness, state, EdgeOpKind::Chamfer, (body, key), 4).await;
    drive::confirm(harness, state).await;
    let after = drive::volume(harness, body).await;
    close(
        before - after,
        2.0,
        2.0,
        DELTA,
        "1 mm equal chamfer removes ½·1·1·4",
    );
    (id, after)
}

async fn offset_top(
    harness: &Harness,
    state: &mut Interaction,
    body: BodyId,
    before: f64,
) -> (RecordId, f64) {
    let faces = drive::elements(harness, body, "f").await;
    let top = faces
        .iter()
        .filter(|face| face.has_normal && face.normal[2].abs() > 1.0 - 1e-9)
        .max_by(|a, b| a.center[2].total_cmp(&b.center[2]))
        .expect("planar top face")
        .clone();
    // The 0.5 mm fillet and the 1 mm chamfer each trim one top corner.
    let trimmed = (1.0 - PI / 4.0) * 0.25 + 0.5;
    let hole = PI * plate::HOLE_RADIUS * plate::HOLE_RADIUS;
    let area = plate::WIDTH * plate::HEIGHT - hole - trimmed;
    close(top.magnitude, area, area, 1e-9, "top face area");
    let context = drive::adopt(harness, state).await;
    let head = harness.projection().await;
    let scope = api::preview_context(&harness.state).await.unwrap();
    let pick = api::OffsetFacePickInput {
        body_id: Some(body_id_wire(body)),
        topo_key: Some(top.topo_key.clone()),
        element_id: None,
    };
    let staged = api::stage_offset_face(&harness.state, &scope, vec![pick], true)
        .await
        .unwrap();
    assert!(staged.prepared.refusal.is_none(), "{staged:?}");
    assert_eq!(staged.refs.len(), 1, "a plate top face chains to nothing");
    staged_cleanly(harness, &head).await;
    let size = Scalar::new(DEFAULT_OPERATION_SIZE_MM);
    let step = head.applied_ops as u32;
    let record = operand_binding::bind_offset_face(&staged, size, RecordId::new(), step).unwrap();
    let id = begin(harness, state, record, 5, context).await;
    drive::confirm(harness, state).await;
    let after = drive::volume(harness, body).await;
    close(
        after - before,
        area,
        area,
        DELTA,
        "+1 mm offset adds the top area",
    );
    (id, after)
}

type Bounds = ([f64; 3], [f64; 3]);

/// A new Move / Rotate about the bounds centre with one typed value.
async fn transform(
    harness: &Harness,
    state: &mut Interaction,
    (body, bounds): (BodyId, Bounds),
    (parameter, text): (NumericParameter, &str),
    gesture: u64,
) -> RecordId {
    let context = drive::adopt(harness, state).await;
    let step = harness.projection().await.applied_ops as u32;
    let center = [0, 1, 2].map(|axis| (bounds.0[axis] + bounds.1[axis]) * 0.5);
    let record = operand_binding::transform_record(body, center, RecordId::new(), step);
    let id = record.record_id;
    let fields = operand_binding::transform_fields();
    let effects = state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        focus: fields.first().copied(),
        fields,
        gesture,
        context,
    });
    drive::drive(harness, state, effects).await;
    drive::typed(harness, state, parameter.into(), text).await;
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
    drive::confirm(harness, state).await;
    id
}

async fn move_and_rotate(
    harness: &Harness,
    state: &mut Interaction,
    body: BodyId,
    volume: f64,
) -> (RecordId, RecordId) {
    let start = drive::bounds(harness, body).await;
    let x = NumericParameter::TransformTranslation(ParameterAxis::X);
    let moved = transform(harness, state, (body, start), (x, "10"), 6).await;
    close(
        drive::volume(harness, body).await,
        volume,
        volume,
        SAME,
        "move keeps volume",
    );
    let shifted = drive::bounds(harness, body).await;
    for axis in 0..3 {
        let shift = if axis == 0 { 10.0 } else { 0.0 };
        assert!(
            (shifted.0[axis] - start.0[axis] - shift).abs() < MESH,
            "{shifted:?}"
        );
        assert!(
            (shifted.1[axis] - start.1[axis] - shift).abs() < MESH,
            "{shifted:?}"
        );
    }
    let angle = NumericParameter::TransformRotationAngle;
    let rotated = transform(harness, state, (body, shifted), (angle, "90"), 7).await;
    close(
        drive::volume(harness, body).await,
        volume,
        volume,
        SAME,
        "rotate keeps volume",
    );
    let turned = drive::bounds(harness, body).await;
    let extent = |bounds: &Bounds, axis: usize| bounds.1[axis] - bounds.0[axis];
    let centre = |bounds: &Bounds, axis: usize| (bounds.1[axis] + bounds.0[axis]) * 0.5;
    for (axis, swapped) in [(0, 1), (1, 0), (2, 2)] {
        assert!((extent(&turned, axis) - extent(&shifted, swapped)).abs() < MESH);
        assert!((centre(&turned, axis) - centre(&shifted, axis)).abs() < MESH);
    }
    (moved, rotated)
}

async fn save(harness: &Harness, path: &Path) {
    let saved = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean, "save persists the current revision");
    assert!(path.is_file());
}

async fn reopen(root: &Path, path: &Path, authored: &Authored) {
    let harness = Harness::new(root).await;
    api::open_document(
        &harness.state,
        harness.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let initial = harness.projection().await;
    let reopened = harness.settled(&initial).await;
    assert!(!reopened.dirty, "replay does not create an unsaved edit");
    let op_types: Vec<_> = reopened
        .features
        .iter()
        .map(|f| f.op_type.clone())
        .collect();
    assert_eq!(op_types, authored.op_types);
    assert_eq!(
        op_types,
        [
            "Sketch",
            "Extrude",
            "Sketch",
            "Extrude",
            "Fillet",
            "Chamfer",
            "OffsetFace",
            "TransformBody",
            "TransformBody"
        ]
    );
    reopened_params(&harness, authored).await;
    let volume = drive::volume(&harness, authored.body).await;
    close(
        volume,
        authored.volume,
        authored.volume,
        SAME,
        "reopened volume",
    );
    harness.close().await;
}

async fn stored(harness: &Harness, id: RecordId) -> Operation {
    api::get_operation_record(&harness.state, id)
        .await
        .unwrap()
        .op
}

async fn reopened_params(harness: &Harness, authored: &Authored) {
    let Operation::Known(KnownOperation::Fillet(fillet)) = stored(harness, authored.fillet).await
    else {
        panic!("fillet survives reopen")
    };
    assert_eq!((fillet.radius.value, fillet.edge_ids.len()), (0.5, 1));
    let Operation::Known(KnownOperation::Chamfer(chamfer)) =
        stored(harness, authored.chamfer).await
    else {
        panic!("chamfer survives reopen")
    };
    assert_eq!(chamfer.radius.value, 1.0);
    assert!(!chamfer.is_asymmetric());
    let Operation::Known(KnownOperation::OffsetFace(offset)) =
        stored(harness, authored.offset).await
    else {
        panic!("offset face survives reopen")
    };
    assert_eq!((offset.distance.value, offset.face_ids.len()), (1.0, 1));
    let Operation::Known(KnownOperation::TransformBody(moved)) =
        stored(harness, authored.moved).await
    else {
        panic!("move survives reopen")
    };
    assert_eq!(
        moved.translate.each_ref().map(|s| s.value),
        [10.0, 0.0, 0.0]
    );
    let Operation::Known(KnownOperation::TransformBody(turned)) =
        stored(harness, authored.rotated).await
    else {
        panic!("rotation survives reopen")
    };
    assert_eq!(turned.rotate.angle_deg.value, 90.0);
    assert_eq!(turned.rotate.axis.z, 1.0);
}
