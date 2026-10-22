//! Real-kernel coverage of the records the desktop/iPad shared tool routes author.
#[allow(dead_code)]
#[path = "real_edge_ops/drive.rs"]
mod drive;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "real_edge_ops/plate.rs"]
mod plate;
use host::Harness;
use onecad_app::api;
use onecad_core::{
    document::{
        record::*,
        refs::{ElementKind, ElementRef},
    },
    ids::{BodyId, RecordId},
    math::Vec3,
};
use onecad_interaction::*;
use std::path::Path;

#[test]
fn shared_modeling_creation_preview_commit_undo_and_reopen() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("modeling-creation") {
        let root =
            std::path::PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(workflow(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    host::run_isolated_phase(
        root.path(),
        "modeling-creation",
        "shared_modeling_creation_preview_commit_undo_and_reopen",
    );
}

async fn begin(h: &Harness, state: &mut Interaction, record: OperationRecord) {
    eprintln!(
        "Beginning {} {} at revision {}",
        record.op.op_type(),
        record.record_id,
        h.projection().await.revision
    );
    let context = drive::adopt(h, state).await;
    let action = operand_binding::begin_bound(record, 1, context).unwrap();
    let effects = state.dispatch(action);
    drive::drive(h, state, effects).await;
    assert!(
        state.tool.as_ref().is_some_and(|tool| tool.ready()),
        "{:?}",
        state.tool
    );
}

async fn undo(h: &Harness, body: BodyId, base: f64) {
    let projection = api::undo(&h.state, h.host.clone()).await.unwrap();
    let head = h.settled(&projection).await;
    assert_eq!(head.bodies.len(), 1);
    assert!((drive::volume(h, body).await - base).abs() < 1e-5);
}

async fn stage(h: &Harness, body: BodyId, kind: ElementKind, key: String) -> ElementRef {
    let scope = api::preview_context(&h.state).await.unwrap();
    api::stage_elements(
        &h.state,
        &scope,
        vec![api::StagePick {
            body,
            kind,
            address: api::StageAddress::TopoKey(key),
        }],
    )
    .await
    .unwrap()
    .remove(0)
}

async fn workflow(root: &Path) {
    let h = Harness::new(&root.join("author")).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let (body, projection) = plate::build_solid(&h).await;
    let base = drive::volume(&h, body).await;
    let step = projection.applied_ops as u32;
    let mut state = Interaction::default();
    for (record, count) in [
        (
            modeling_records::linear_pattern(body, 100.0, RecordId::new(), step),
            2,
        ),
        (
            modeling_records::circular_pattern(body, RecordId::new(), step),
            3,
        ),
        (modeling_records::mirror(body, RecordId::new(), step), 2),
    ] {
        begin(&h, &mut state, record).await;
        let head = drive::confirm(&h, &mut state).await;
        assert_eq!(head.bodies.len(), count);
        for id in head.bodies.keys() {
            assert!((drive::volume(&h, id.parse().unwrap()).await - base).abs() < 1e-5);
        }
        undo(&h, body, base).await;
    }
    face_operations(&h, &mut state, body, step, base).await;
    begin(
        &h,
        &mut state,
        modeling_records::gear(RecordId::new(), step),
    )
    .await;
    let gears = drive::confirm(&h, &mut state).await;
    assert_eq!(gears.bodies.len(), 2);
    let gear_body = gears
        .bodies
        .keys()
        .map(|id| id.parse::<BodyId>().unwrap())
        .find(|id| *id != body)
        .unwrap();
    assert!(drive::volume(&h, gear_body).await > 0.0);
    undo(&h, body, base).await;
    revolve(&h, &mut state, body, step, base).await;
    boolean(&h, &mut state, body, step, root).await;
    h.close().await;
    let reopened = Harness::new(&root.join("reopened")).await;
    api::open_document(
        &reopened.state,
        reopened.host.clone(),
        root.join("modeled.onecad").to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let head = reopened.settled(&reopened.projection().await).await;
    assert_eq!(head.features.last().unwrap().op_type, "Boolean");
    assert_eq!(head.bodies.len(), 1);
    assert!(drive::volume(&reopened, body).await < base);
    reopened.close().await;
}

async fn face_operations(h: &Harness, state: &mut Interaction, body: BodyId, step: u32, base: f64) {
    for is_hole in [false, true] {
        let faces = drive::elements(h, body, "f").await;
        let top = faces
            .iter()
            .filter(|f| f.has_normal && f.normal[2] > 0.99)
            .max_by(|a, b| a.center[2].total_cmp(&b.center[2]))
            .unwrap();
        let face = stage(h, body, ElementKind::Face, top.topo_key.clone()).await;
        let record = if is_hole {
            modeling_records::hole(
                face,
                Vec3::new_unchecked(-10.0, 10.0, 4.0),
                RecordId::new(),
                step,
            )
            .unwrap()
        } else {
            modeling_records::shell(vec![face], RecordId::new(), step).unwrap()
        };
        begin(h, state, record).await;
        drive::confirm(h, state).await;
        let volume = drive::volume(h, body).await;
        assert!(volume > 0.0 && volume < base);
        if is_hole {
            assert!((base - volume - 16.0 * std::f64::consts::PI).abs() < 1e-5);
        }
        undo(h, body, base).await;
    }
}

async fn revolve(h: &Harness, state: &mut Interaction, body: BodyId, step: u32, base: f64) {
    let edges = drive::elements(h, body, "e").await;
    let edge = edges
        .iter()
        .find(|e| {
            e.center[0].abs() < 1e-6
                && e.center[2].abs() < 1e-6
                && (e.magnitude - 60.0).abs() < 1e-6
        })
        .unwrap();
    let axis = stage(h, body, ElementKind::Edge, edge.topo_key.clone()).await;
    let source = api::get_operation_record(&h.state, plate::plate_record())
        .await
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(p)) = source.op else {
        panic!()
    };
    let record =
        modeling_records::revolve(p.profile.unwrap(), axis, RecordId::new(), step).unwrap();
    begin(h, state, record).await;
    let head = drive::confirm(h, state).await;
    assert_eq!(head.bodies.len(), 2);
    let new_body = head
        .bodies
        .keys()
        .map(|s| s.parse::<BodyId>().unwrap())
        .find(|id| *id != body)
        .unwrap();
    assert!(
        (drive::volume(h, new_body).await - std::f64::consts::PI * 40.0 * 40.0 * 60.0).abs() < 1e-4
    );
    undo(h, body, base).await;
    // A sketch line also works before there is any other solid to pick an edge on.
    let source = api::get_operation_record(&h.state, plate::plate_record())
        .await
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(p)) = source.op else {
        panic!()
    };
    let profile = p.profile.unwrap();
    let sketch = api::get_sketch_snapshot(&h.state, profile.sketch)
        .await
        .unwrap();
    let line = sketch
        .entities()
        .iter()
        .find_map(|entity| match entity {
            onecad_core::sketch::SketchEntity::Line { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let axis = onecad_core::document::refs::AxisRef::SketchLine {
        sketch: profile.sketch,
        line,
        extra: Default::default(),
    };
    let record = modeling_records::revolve_axis(profile, axis, RecordId::new(), step);
    begin(h, state, record).await;
    assert_eq!(drive::confirm(h, state).await.bodies.len(), 2);
    undo(h, body, base).await;
}

async fn boolean(h: &Harness, state: &mut Interaction, body: BodyId, step: u32, root: &Path) {
    let mut copy = operand_binding::transform_record(body, [0.0; 3], RecordId::new(), step);
    if let Operation::Known(KnownOperation::TransformBody(p)) = &mut copy.op {
        p.copy = true;
        p.translate[0].value = 5.0;
    }
    begin(h, state, copy).await;
    let head = drive::confirm(h, state).await;
    let tool_body = head
        .bodies
        .keys()
        .map(|s| s.parse::<BodyId>().unwrap())
        .find(|id| *id != body)
        .unwrap();
    let mut record =
        modeling_records::boolean(&[body, tool_body], RecordId::new(), step + 1).unwrap();
    if let Operation::Known(KnownOperation::Boolean(p)) = &mut record.op {
        p.operation = BooleanOp::Cut;
    }
    begin(h, state, record).await;
    let head = drive::confirm(h, state).await;
    assert_eq!(head.bodies.len(), 1);
    let saved = api::save_document(
        &h.state,
        h.host.clone(),
        Some(root.join("modeled.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
}
