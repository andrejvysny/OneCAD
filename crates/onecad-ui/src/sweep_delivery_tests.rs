//! Bounded Sweep through native controls; synthetic input is not physical acceptance.
use super::*;
use crate::sketch_services::test_support::Harness;
use onecad_app::api;
use onecad_core::{
    document::{
        datum::DatumPlane,
        record::{KnownOperation, Operation},
    },
    edit::{EditCommand, SketchEditOp},
    ids::{DatumPlaneId, EntityId, SketchId},
    math::Vec2,
    sketch::{Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use onecad_interaction::{Action, PreviewStatus, ProfileSelection};
use onecad_scene::{ViewSnapshot, Viewport};
use std::path::Path;
use uuid::Uuid;

fn sid(n: u128) -> SketchId {
    SketchId(Uuid::from_u128(0x5ee0_0000 + n))
}

async fn circle(h: &Harness, n: u128, sketch: Sketch, center: [f64; 2], radius: f64) {
    let id = sketch.id;
    api::apply_edit_command(&h.state, h.host.clone(), EditCommand::AddSketch { sketch })
        .await
        .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), id.to_string(), Some(true))
        .await
        .unwrap();
    let point = EntityId(Uuid::from_u128(0x5ee0_1000 + n));
    let curve = EntityId(Uuid::from_u128(0x5ee0_2000 + n));
    api::sketch_upsert(
        &h.state,
        h.host.clone(),
        id.to_string(),
        vec![
            SketchEditOp::AddEntity {
                entity: SketchEntity::point(
                    point,
                    Vec2::new_unchecked(center[0], center[1]),
                    false,
                    false,
                ),
            },
            SketchEditOp::AddEntity {
                entity: SketchEntity::circle(curve, point, radius, false).unwrap(),
            },
        ],
    )
    .await
    .unwrap();
    let result = api::finish_sketch(&h.state, h.host.clone(), id.to_string())
        .await
        .unwrap();
    assert_eq!(result.regions.len(), 1);
    h.settled(&h.projection().await).await;
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
    circle(
        &h,
        1,
        Sketch::on_world_plane(sid(1), "Circular path", WorldPlane::XY),
        [0.0, 0.0],
        4.0,
    )
    .await;
    circle(
        &h,
        3,
        Sketch::on_world_plane(sid(3), "Alternate path", WorldPlane::XY),
        [0.0, -0.5],
        3.5,
    )
    .await;
    let datum = DatumPlaneId(Uuid::from_u128(0x5ee0_0005));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "Tangent section", "YZ", 0.0),
        },
    )
    .await
    .unwrap();
    circle(
        &h,
        2,
        Sketch::new(
            sid(2),
            "Circular section",
            SketchAttachment::Datum { datum },
        ),
        [-4.0, 0.0],
        0.5,
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

async fn open(host: &mut Host, path: &Path) {
    host.app
        .document_request(crate::document_flow::Request::Open(Some(
            path.to_path_buf(),
        )));
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.doc.documents.running
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.profiles.iter().any(|p| p.reference.sketch == sid(2)))
    })
    .await;
}

fn select_section(host: &mut Host) {
    let display = host.app.doc.display.clone().unwrap();
    let context = crate::sketch_services::document_context(&display);
    let view = ViewSnapshot::new(
        context.scene,
        host.app.view_number + 1,
        host.app.doc.camera,
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let profile = display
        .profiles
        .iter()
        .find(|p| p.reference.sketch == sid(2))
        .unwrap()
        .reference
        .clone();
    host.app
        .dispatch(Action::SelectProfile(Box::new(ProfileSelection {
            profile,
            context,
            capture: view.identity(),
        })));
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic input is not physical acceptance"]
async fn circular_sweep_ui_preview_commit_undo_reopen() {
    let root = tempfile::tempdir().unwrap();
    let saved = root.path().join("sweep.onecad");
    Box::pin(fixture(root.path(), &saved)).await;
    let seed_bytes = std::fs::read(&saved).unwrap();
    let mut host = Box::new(Box::pin(boxed(&root.path().join("native"))).await);
    open(&mut host, &saved).await;
    select_section(&mut host);
    assert!(crate::sweep_ui::ready(&host.app));
    let before = host.app.doc.display.as_ref().unwrap().records.len();
    host.control("More tools");
    host.control("Sweep");
    host.app.dispatch(Action::Confirm);
    host.redraw();
    assert_eq!(host.app.doc.display.as_ref().unwrap().records.len(), before);
    assert!(
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|t| !t.ready() && t.preview == PreviewStatus::Empty),
        "tool={:?}; status={}; profiles={:?}",
        host.app.doc.interaction.tool,
        host.app.doc.status,
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .profiles
            .iter()
            .map(|p| &p.reference)
            .collect::<Vec<_>>()
    );
    assert!(host.app.doc.interaction.tool.as_ref().is_some_and(|t| matches!(t.effective_op(), Operation::Known(KnownOperation::Sweep(p)) if p.path_sketch.is_none())));
    tap(&mut host, id::OP_CANCEL);
    assert!(host.app.doc.interaction.tool.is_none());
    assert_eq!(host.app.doc.display.as_ref().unwrap().records.len(), before);
    select_section(&mut host);
    host.control("More tools");
    host.control("Sweep");
    host.control("Choose path…");
    host.control("Circular path");
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        if let Some(tool) = &app.doc.interaction.tool {
            if !matches!(tool.preview, PreviewStatus::Pending | PreviewStatus::Ready) {
                panic!(
                    "preview stopped: {:?}; params {:?}",
                    tool.preview, tool.params
                );
            }
            tool.ready() && tool.preview == PreviewStatus::Ready
        } else {
            false
        }
    })
    .await;
    host.redraw();
    let confirm = named(&mut host, id::OP_CONFIRM);
    assert!(
        host.screen().contains_rect(confirm),
        "Create outside viewport: {confirm:?}"
    );
    tap(&mut host, id::OP_CONFIRM);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        if let onecad_interaction::CommitStatus::Rejected(error) = &app.doc.interaction.status {
            panic!("commit refused: {error}");
        }
        app.doc.interaction.tool.is_none()
            && !app.busy()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.bodies.len() == 1)
    })
    .await;
    crate::input::submit_history(&mut host.app, crate::bridge::Command::Undo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.busy()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.bodies.is_empty())
    })
    .await;
    crate::input::submit_history(&mut host.app, crate::bridge::Command::Redo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.busy()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.bodies.len() == 1)
    })
    .await;
    host.app
        .document_request(crate::document_flow::Request::Save { save_as: false });
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.doc.documents.running && !app.busy() && !app.document_dirty()
    })
    .await;
    let saved_path = std::path::PathBuf::from(
        host.app
            .doc
            .document_state
            .as_ref()
            .unwrap()
            .path
            .as_ref()
            .unwrap(),
    );
    assert_ne!(saved_path, saved);
    assert!(saved_path.is_file());
    assert_eq!(std::fs::read(&saved).unwrap(), seed_bytes);
    host.stop().await;
    Box::pin(reopen_and_edit(&saved_path)).await;
}

async fn reopen_and_edit(saved: &Path) {
    let root = tempfile::tempdir().unwrap();
    let mut host = Box::new(Box::pin(boxed(root.path())).await);
    open(&mut host, saved).await;
    let display = host.app.doc.display.clone().unwrap();
    assert_eq!(display.projection.bodies.len(), 1);
    let old_min_x = display.scene.bounds().unwrap().min.x;
    let index = display
        .records
        .iter()
        .position(|r| {
            matches!(&r.op,
        Operation::Known(KnownOperation::Sweep(p)) if p.path_sketch == Some(sid(1)))
        })
        .unwrap();
    host.app.ui.inspector_open = true;
    host.app.ui.inspector_tab = crate::ui_state::InspectorTab::History;
    tap(&mut host, &id::history(index));
    host.control("Edit feature");
    host.control("Circular path");
    host.control("Alternate path");
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|t| t.ready() && t.preview == PreviewStatus::Ready)
    })
    .await;
    tap(&mut host, id::OP_CONFIRM);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| app.doc.interaction.tool.is_none()
        && !app.busy() && app.doc.display.as_ref().is_some_and(|d| d.records.iter().any(|r|
            matches!(&r.op, Operation::Known(KnownOperation::Sweep(p)) if p.path_sketch == Some(sid(3)))))).await;
    let edited = host.app.doc.display.as_ref().unwrap();
    let new_min_x = edited.scene.bounds().unwrap().min.x;
    assert!(
        new_min_x - old_min_x > 0.5,
        "edited path did not change mesh bounds: {old_min_x} -> {new_min_x}"
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic viewport pointer is not physical acceptance"]
async fn viewport_pointer_selects_current_sweep_section() {
    let root = tempfile::tempdir().unwrap();
    let saved = root.path().join("sweep-pick.onecad");
    Box::pin(fixture(root.path(), &saved)).await;
    let mut host = Box::new(Box::pin(boxed(&root.path().join("native"))).await);
    open(&mut host, &saved).await;
    pick_section_at_known_point(&mut host);
    assert!(crate::sweep_ui::ready(&host.app));
    host.stop().await;
}

fn pick_section_at_known_point(host: &mut Host) {
    host.app.doc.camera.target = onecad_scene::DVec3::new(4.0, 0.0, 0.0);
    host.app.doc.camera.offset = onecad_scene::DVec3::new(12.0, -15.0, 12.0);
    host.app.doc.fit_pending = false;
    host.redraw();
    let view = host.app.doc.view.expect("current captured view");
    let logical = view
        .project(onecad_scene::DVec3::new(4.0, 0.0, 0.25))
        .expect("section interior visible")
        .logical;
    let viewport = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    let screen = viewport.min + egui::vec2(logical.x as f32, logical.y as f32);
    assert!(viewport.contains(screen));
    host.click(screen);
    let selected = host
        .app
        .doc
        .interaction
        .selection
        .as_ref()
        .expect("viewport profile pick");
    assert_eq!(selected.profile.sketch, sid(2));
    assert!(selected.profile.region_anchor.is_some());
}

async fn suppress_fixture_paths(root: &Path, saved: &Path) {
    Box::pin(fixture(root, saved)).await;
    let h = Harness::new(&root.join("suppression")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        saved.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let projection = h.settled(&h.projection().await).await;
    for feature in projection.features.iter().take(2) {
        let record: onecad_core::ids::RecordId = feature.id.parse().unwrap();
        let operation = api::get_operation_record(&h.state, record).await.unwrap();
        assert!(
            matches!(operation.op, Operation::Known(KnownOperation::Sketch(p)) if p.sketch == sid(1) || p.sketch == sid(3))
        );
        let changed = api::apply_edit_command(
            &h.state,
            h.host.clone(),
            EditCommand::SetOperationSuppression {
                record,
                suppressed: true,
                cascade: false,
            },
        )
        .await
        .unwrap();
        let report = h.terminal(&changed).await;
        assert!(
            matches!(report.outcome.as_str(), "published" | "noop"),
            "{report:?}"
        );
        assert!(
            report.failed_steps.is_empty() && report.repair_steps.is_empty(),
            "{report:?}"
        );
        assert!(
            api::get_operation_record(&h.state, record)
                .await
                .unwrap()
                .suppressed
        );
    }
    let saved_result = api::save_document(
        &h.state,
        h.host.clone(),
        Some(saved.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved_result.clean);
    h.close().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic viewport pointer is not physical acceptance"]
async fn sweep_rejects_suppressed_paths_in_real_capture() {
    let root = tempfile::tempdir().unwrap();
    let saved = root.path().join("suppressed.onecad");
    Box::pin(suppress_fixture_paths(root.path(), &saved)).await;
    let mut host = Box::new(Box::pin(boxed(&root.path().join("native"))).await);
    open(&mut host, &saved).await;
    let display = host.app.doc.display.as_ref().unwrap();
    assert_eq!(display.projection.status, onecad_app::dto::DocStatus::Ready);
    assert_eq!(display.records.iter().filter(|r| r.suppressed).count(), 2);
    assert_eq!(
        display
            .profiles
            .iter()
            .filter(|p| p.reference.sketch == sid(1) || p.reference.sketch == sid(3))
            .count(),
        2
    );
    assert_eq!(
        display
            .projection
            .features
            .iter()
            .filter(|f| f.suppressed)
            .count(),
        2
    );
    pick_section_at_known_point(&mut host);
    let before = host.app.doc.display.as_ref().unwrap().records.len();
    assert!(!crate::sweep_ui::ready(&host.app));
    host.control("More tools");
    host.control("Sweep");
    assert!(host.app.doc.interaction.tool.is_none());
    assert_eq!(host.app.doc.display.as_ref().unwrap().records.len(), before);
    host.stop().await;
}
