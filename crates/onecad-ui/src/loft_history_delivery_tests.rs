//! Reopened nonnumeric history records must reach the native editor.
use super::*;
use crate::sketch_services::test_support::Harness;
use onecad_app::{api, dto::FeatureStatus, worker::wire::body_id_wire};
use onecad_core::{
    document::{
        datum::DatumPlane,
        record::*,
        refs::{ElementKind, SketchRegionRef},
    },
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, DatumPlaneId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    sketch::{Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use onecad_interaction::modeling_records;
use std::path::Path;
use uuid::Uuid;
fn loft_id() -> RecordId {
    RecordId(Uuid::from_u128(0xa0f7))
}
fn body() -> BodyId {
    BodyId(loft_id().0)
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn reopened_loft_history_edits_choices_and_preserves_downstream_hole() {
    let root = tempfile::tempdir().unwrap();
    let saved = root.path().join("loft.onecad");
    fixture(root.path(), &saved).await;
    let mut host = boxed(&root.path().join("native")).await;
    host.app
        .document_request(crate::document_flow::Request::Open(Some(saved)));
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.doc.documents.running
            && app.doc.display.as_ref().is_some_and(|d| {
                d.projection
                    .features
                    .iter()
                    .any(|f| f.id == loft_id().to_string())
            })
    })
    .await;
    let display = host.app.doc.display.clone().unwrap();
    assert_eq!(
        display.records.len(),
        display.projection.features.len(),
        "capture must retain every admitted authored history record, including Loft without numeric fields"
    );
    let hole = display.records.last().unwrap().clone();
    host.app.ui.inspector_open = true;
    host.app.ui.inspector_tab = crate::ui_state::InspectorTab::History;
    tap(&mut host, &id::history(2));
    host.control("Edit feature");
    assert!(
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.editing_existing && tool.tag.record_id == loft_id()),
        "selected Loft must enter its history editor"
    );
    host.control("Ruled");
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| app.doc.interaction.tool.as_ref().is_some_and(|tool| tool.ready() && matches!(tool.effective_op(), Operation::Known(KnownOperation::Loft(p)) if p.is_ruled))).await;
    tap(&mut host, id::OP_CONFIRM);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.is_none()
            && !app.busy()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.undo_depth == display.projection.undo_depth + 1)
    })
    .await;
    let edited = host.app.doc.display.clone().unwrap();
    assert!(
        edited
            .projection
            .features
            .iter()
            .all(|f| f.status == FeatureStatus::Ok)
    );
    assert_eq!(edited.projection.bodies.len(), 1);
    assert_eq!(
        edited.records.last().unwrap(),
        &hole,
        "Loft choice edit must preserve the later Hole binding and parameters"
    );
    assert!(
        matches!(&edited.records[2].op, Operation::Known(KnownOperation::Loft(p)) if p.is_ruled)
    );
    host.stop().await;
}
async fn fixture(root: &Path, saved: &Path) {
    let h = Harness::new(&root.join("fixture")).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let profiles = vec![
        section(&h, 1, 0.0, 5.0).await,
        section(&h, 2, 20.0, 8.0).await,
    ];
    let record = modeling_records::loft(profiles, loft_id(), 2).unwrap();
    add(&h, record).await;
    let scope = api::preview_context(&h.state).await.unwrap();
    let mut top = None;
    for ordinal in 1..=16 {
        let key = format!("f:{ordinal}");
        let Some(info) = api::element_info(
            &h.state,
            scope.snapshot_id.0,
            body_id_wire(body()),
            String::new(),
            Some(key.clone()),
            None,
        )
        .await
        .unwrap() else {
            break;
        };
        if info.has_normal && info.normal[2] > 0.99 && (info.center[2] - 20.0).abs() < 1e-5 {
            top = Some(key);
            break;
        }
    }
    let (face, point) = api::stage_hole(
        &h.state,
        &scope,
        api::StagePick {
            body: body(),
            kind: ElementKind::Face,
            address: api::StageAddress::TopoKey(top.expect("planar Loft top")),
        },
    )
    .await
    .unwrap();
    add(
        &h,
        modeling_records::hole(face, point, RecordId(Uuid::from_u128(0xa0f8)), 3).unwrap(),
    )
    .await;
    api::save_document(
        &h.state,
        h.host.clone(),
        Some(saved.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    h.close().await;
}
async fn add(h: &Harness, record: OperationRecord) {
    let response = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::AddOperation {
            record,
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&response.projection).await;
}
async fn section(h: &Harness, n: u128, offset: f64, radius: f64) -> SketchRegionRef {
    let sketch = SketchId(Uuid::from_u128(0xa100 + n));
    let datum = DatumPlaneId(Uuid::from_u128(0xa200 + n));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, format!("Section {n}"), "XY", offset),
        },
    )
    .await
    .unwrap();
    let mut empty = Sketch::on_world_plane(sketch, format!("Section {n}"), WorldPlane::XY);
    empty.attachment = SketchAttachment::Datum { datum };
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddSketch { sketch: empty },
    )
    .await
    .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sketch.to_string(), Some(true))
        .await
        .unwrap();
    let point = EntityId(Uuid::from_u128(0xa300 + n));
    let circle = EntityId(Uuid::from_u128(0xa400 + n));
    api::sketch_upsert(
        &h.state,
        h.host.clone(),
        sketch.to_string(),
        vec![
            SketchEditOp::AddEntity {
                entity: SketchEntity::point(point, Vec2::new_unchecked(0.0, 0.0), false, false),
            },
            SketchEditOp::AddEntity {
                entity: SketchEntity::circle(circle, point, radius, false).unwrap(),
            },
        ],
    )
    .await
    .unwrap();
    let finished = api::finish_sketch(&h.state, h.host.clone(), sketch.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    SketchRegionRef {
        sketch,
        region: RegionId::new(&finished.regions[0].region_id),
        region_identity_version: Some(finished.region_identity_version),
        region_anchor: Some([1.0, 0.0]),
        extra: Default::default(),
    }
}
