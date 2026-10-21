//! Synthetic native-host support entry with real worker and Metal, not physical input.
use super::dimension_delivery_tests::{click, ids};
use super::*;
use onecad_core::{ids::RecordId, sketch::WorldPlane};
use onecad_interaction::{
    Action, CommitStatus, DraftStatus, FeatureOutcome, PreviewStatus, ProfileSelection,
    sketch::{SketchAction, SketchPhase, SketchTool},
};
use onecad_scene::{ViewSnapshot, Viewport};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic service delivery is not physical acceptance"]
async fn world_planes_and_face_entry_preserve_native_frame_and_discard() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
    for (plane, modeling) in [
        (WorldPlane::XY, onecad_scene::Projection::Perspective),
        (WorldPlane::XZ, onecad_scene::Projection::Orthographic),
        (WorldPlane::YZ, onecad_scene::Projection::Perspective),
    ] {
        app.camera.projection = modeling;
        app.new_sketch_on_plane(plane);
        active(&mut app, &context, &mut frame).await;
        assert_eq!(app.sketch.session.as_ref().unwrap().plane, plane.plane());
        assert_camera(&app);
        discard(&mut app, &context, &mut frame).await;
        // Leaving the sketch restores the projection chosen for modeling.
        assert_eq!(app.camera.projection, modeling);
        assert!(app.display.as_ref().unwrap().projection.sketches.is_empty());
    }
    make_host(&mut app, &context, &mut frame).await;
    let display = app.display.clone().unwrap();
    let body = &display.scene.bodies()[0];
    let identity = body.mesh().unwrap().face_id(0).unwrap().to_owned();
    // A preselected face starts the sketch directly, without the support chooser.
    app.topology
        .select_face_for_test(display.scene.identity(), body.body_id(), &identity);
    app.choose_sketch_support();
    assert!(app.ui.sketch_entry.pending.is_some());
    assert!(crate::support_view::grid(&app).is_none());
    active(&mut app, &context, &mut frame).await;
    assert!(!app.ui.sketch_entry.open);
    let sketch = &app.sketch.session.as_ref().unwrap().authoritative;
    assert!(sketch.host_face().unwrap().primary.is_some());
    assert!(!sketch.entities().is_empty());
    assert!(
        sketch
            .entities()
            .iter()
            .all(|entity| entity.is_reference_locked())
    );
    assert_camera(&app);
    discard(&mut app, &context, &mut frame).await;
    assert_eq!(app.display.as_ref().unwrap().projection.sketches.len(), 1);
    assert_eq!(app.display.as_ref().unwrap().scene.bodies().len(), 1);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

async fn active(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    pump_until(app, ctx, frame, |app| {
        app.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
}

async fn discard(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    app.sketch_dispatch(SketchAction::Discard);
    pump_until(app, ctx, frame, |app| !app.sketch_active()).await;
}

fn capture(app: &mut App) -> ViewSnapshot {
    app.view_number += 1;
    let session = app.sketch.session.as_ref().unwrap();
    let view = ViewSnapshot::new(
        session.document.scene,
        app.view_number,
        app.camera,
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    app.sketch_dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
    });
    app.view = Some(view);
    view
}

fn assert_camera(app: &App) {
    let plane = app.sketch.session.as_ref().unwrap().plane;
    let vector = |v: onecad_core::math::Vec3| onecad_scene::DVec3::new(v.x, v.y, v.z);
    let basis = app.camera.basis().unwrap();
    assert!((basis.right - vector(plane.x_axis)).length() < 1e-12);
    assert!((basis.up - vector(plane.y_axis)).length() < 1e-12);
    assert!((basis.back - vector(plane.normal)).length() < 1e-12);
}

pub(super) async fn make_host(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    app.new_sketch_on_plane(WorldPlane::XZ);
    active(app, ctx, frame).await;
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Rectangle));
    let view = capture(app);
    click(app, view.identity(), 0.0, 0.0, ids());
    let rectangle = ids();
    click(app, view.identity(), 40.0, 30.0, rectangle.clone());
    pump_until(app, ctx, frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && session.authoritative.contains_entity(rectangle.entities[7])
        })
    })
    .await;
    app.sketch_dispatch(SketchAction::Finish);
    pump_until(app, ctx, frame, |app| !app.sketch_active()).await;
    extrude(app, ctx, frame).await;
}

async fn extrude(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    let display = app.display.clone().unwrap();
    let context = crate::sketch_services::document_context(&display);
    let view = ViewSnapshot::new(
        context.scene,
        app.view_number + 1,
        app.camera,
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    app.dispatch(Action::SelectProfile(Box::new(ProfileSelection {
        profile: display.profiles[0].reference.clone(),
        context,
        capture: view.identity(),
    })));
    app.dispatch(Action::BeginExtrude {
        record_id: RecordId::new(),
        gesture: 1,
        step_index: display.projection.applied_ops as u32,
    });
    app.dispatch(Action::DraftChanged("5".into()));
    pump_until(app, ctx, frame, |app| {
        app.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(&tool.draft_status, DraftStatus::Valid(value) if value.value == 5.0)
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    app.dispatch(Action::Confirm);
    pump_until(app, ctx, frame, |app| {
        matches!(
            app.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        )
    })
    .await;
}
