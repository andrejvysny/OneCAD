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
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    for (plane, modeling) in [
        (WorldPlane::XY, onecad_scene::Projection::Perspective),
        (WorldPlane::XZ, onecad_scene::Projection::Orthographic),
        (WorldPlane::YZ, onecad_scene::Projection::Perspective),
    ] {
        app.doc.camera.projection = modeling;
        app.doc.camera.target = onecad_scene::DVec3::new(12.0, -8.0, 25.0);
        app.doc.camera.offset = onecad_scene::DVec3::new(160.0, -240.0, 180.0);
        app.doc.camera.roll_radians = 0.12;
        let modeling_camera = app.doc.camera;
        app.new_sketch_on_plane(plane);
        active(&mut app, &context, &mut frame).await;
        assert_eq!(
            app.doc.sketch.session.as_ref().unwrap().plane,
            plane.plane()
        );
        assert_camera(&app);
        discard(&mut app, &context, &mut frame).await;
        assert_eq!(
            app.doc.camera, modeling_camera,
            "Discard restores the full modeling camera"
        );
        assert!(
            app.doc
                .display
                .as_ref()
                .unwrap()
                .projection
                .sketches
                .is_empty()
        );
    }
    make_host(&mut app, &context, &mut frame).await;
    let modeling_camera = app.doc.camera;
    let base_sketch = app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .sketches
        .keys()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    app.edit_sketch(base_sketch);
    active(&mut app, &context, &mut frame).await;
    app.sketch_dispatch(SketchAction::Finish);
    pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
    assert_eq!(
        app.doc.camera, modeling_camera,
        "Finish restores the full modeling camera"
    );
    let display = app.doc.display.clone().unwrap();
    let body = &display.scene.bodies()[0];
    let identity = body.mesh().unwrap().face_id(0).unwrap().to_owned();
    // A preselected face starts the sketch directly, without the support chooser.
    crate::model_pick::select_face_for_test(
        &mut app,
        display.scene.identity(),
        body.body_id(),
        &identity,
    );
    app.choose_sketch_support();
    assert!(app.doc.sketch_entry.pending.is_some());
    assert!(crate::support_view::grid(&app).is_none());
    active(&mut app, &context, &mut frame).await;
    assert!(!app.doc.sketch_entry.open);
    let sketch = &app.doc.sketch.session.as_ref().unwrap().authoritative;
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
    assert_eq!(
        app.doc.display.as_ref().unwrap().projection.sketches.len(),
        1
    );
    assert_eq!(app.doc.display.as_ref().unwrap().scene.bodies().len(), 1);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic service delivery is not physical acceptance"]
async fn face_entry_cancel_before_admission_leaves_document_unchanged() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = host_document(area.path()).await;
    let before = document(&app);
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    app.doc.sketch_entry.admission_gate = Some(gate.clone());
    select_host_face(&mut app);
    app.choose_sketch_support();
    assert!(app.doc.sketch_entry.pending.is_some());
    // Idle frames no longer release a request the service still executes.
    for _ in 0..3 {
        logic_tick(&mut app, &context, &mut frame, false);
        entry_frame(&mut app, &context);
    }
    assert!(app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_some());
    escape(&mut app, &context);
    entry_frame(&mut app, &context);
    assert!(app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_some());
    // An owned request is never superseded by a second entry.
    app.choose_sketch_support();
    gate.add_permits(1);
    until_entry_closed(&mut app, &context, &mut frame).await;
    assert!(!app.sketch_active());
    assert!(app.doc.status.contains("cancelled before admission"));
    assert_eq!(document(&app), before);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic service delivery is not physical acceptance"]
async fn face_entry_admission_wins_over_late_cancel_and_opens_once() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = host_document(area.path()).await;
    let sketches = sketch_count(&app);
    select_host_face(&mut app);
    app.choose_sketch_support();
    // The service finished and admitted the sketch; its delivery is not applied yet.
    let delivery = completed_sketch_delivery(&mut app, &context).await;
    escape(&mut app, &context);
    entry_frame(&mut app, &context);
    assert!(app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_some());
    app.message(delivery, &context);
    active(&mut app, &context, &mut frame).await;
    assert!(!app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_none());
    let session = app.doc.sketch.session.as_ref().unwrap();
    assert!(session.authoritative.host_face().unwrap().primary.is_some());
    assert_eq!(sketch_count(&app), sketches + 1);
    discard(&mut app, &context, &mut frame).await;
    assert_eq!(sketch_count(&app), sketches);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic service delivery is not physical acceptance"]
async fn face_entry_context_replacement_cancels_and_releases_after_delivery() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = host_document(area.path()).await;
    let before = document(&app);
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    app.doc.sketch_entry.admission_gate = Some(gate.clone());
    select_host_face(&mut app);
    app.choose_sketch_support();
    // The service is held, so the newer head is adopted only in the UI context.
    let mut replaced = app.doc.interaction.context.clone().unwrap();
    replaced.revision += 1;
    app.doc.interaction.context = Some(replaced);
    entry_frame(&mut app, &context);
    assert!(app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_some());
    gate.add_permits(1);
    until_entry_closed(&mut app, &context, &mut frame).await;
    assert!(!app.sketch_active());
    assert!(app.doc.status.contains("cancelled before admission"));
    assert_eq!(document(&app), before);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic service delivery is not physical acceptance"]
async fn face_entry_submit_failure_releases_ownership_without_sketch() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = host_document(area.path()).await;
    let before = document(&app);
    select_host_face(&mut app);
    // Bridge admission refuses while shutting down; nothing reaches the service.
    let stopping = app.doc.bridge.shared.clone();
    stopping
        .stopping
        .store(true, std::sync::atomic::Ordering::Release);
    app.choose_sketch_support();
    stopping
        .stopping
        .store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(app.doc.status, "Native host is shutting down");
    entry_frame(&mut app, &context);
    assert!(!app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_none());
    for _ in 0..3 {
        logic_tick(&mut app, &context, &mut frame, false);
    }
    assert!(!app.sketch_active());
    assert_eq!(document(&app), before);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic service delivery is not physical acceptance"]
async fn face_entry_ownership_stays_with_its_tab_when_another_document_opens() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = host_document(area.path()).await;
    let first = app.doc.tab;
    let sketches = sketch_count(&app);
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    app.doc.sketch_entry.admission_gate = Some(gate.clone());
    select_host_face(&mut app);
    app.choose_sketch_support();
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked.len() == 1 && app.document_open() && !app.busy()
    })
    .await;
    assert!(!app.doc.sketch_entry.open && app.doc.sketch_entry.pending.is_none());
    gate.add_permits(1);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked[0].sketch_entry.pending.is_none()
    })
    .await;
    // Admission won in the parked document; the active one never owned it.
    assert!(!app.sketch_active());
    assert!(!app.doc.sketch_entry.open);
    app.activate_tab(first);
    active(&mut app, &context, &mut frame).await;
    assert_eq!(sketch_count(&app), sketches + 1);
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

async fn host_document(root: &std::path::Path) -> (App, egui::Context, eframe::Frame) {
    let (mut app, context) = create_app(root).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    make_host(&mut app, &context, &mut frame).await;
    (app, context, frame)
}

fn select_host_face(app: &mut App) {
    let display = app.doc.display.clone().unwrap();
    let body = &display.scene.bodies()[0];
    let identity = body.mesh().unwrap().face_id(0).unwrap().to_owned();
    crate::model_pick::select_face_for_test(
        app,
        display.scene.identity(),
        body.body_id(),
        &identity,
    );
}

fn sketch_count(app: &App) -> usize {
    app.doc.display.as_ref().unwrap().projection.sketches.len()
}

/// Persisted projection plus document dirtiness; a cancelled entry changes neither.
fn document(app: &App) -> (serde_json::Value, bool) {
    (
        serde_json::to_value(&app.doc.display.as_ref().unwrap().projection).unwrap(),
        app.doc
            .document_state
            .as_ref()
            .is_some_and(|state| state.dirty),
    )
}

fn escape(app: &mut App, ctx: &egui::Context) {
    let mut input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        focused: true,
        ..Default::default()
    };
    crate::input::route(app, ctx, &mut input);
}

/// The UI pass that owns entry settlement and context checks.
fn entry_frame(app: &mut App, ctx: &egui::Context) {
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        crate::sketch_entry::show(app, ui.ctx());
    });
    output.textures_delta.clear();
}

async fn until_entry_closed(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            logic_tick(app, ctx, frame, false);
            entry_frame(app, ctx);
            if !app.doc.sketch_entry.open {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("face entry kept ownership: {}", app.doc.status));
    assert!(app.doc.sketch_entry.pending.is_none());
}

async fn completed_sketch_delivery(app: &mut App, ctx: &egui::Context) -> crate::bridge::Message {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match app.doc.bridge.messages.try_recv() {
                Ok(message @ crate::bridge::Message::Sketch { .. }) => return message,
                Ok(other) => app.message(other, ctx),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("face entry delivery")
}

async fn active(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    pump_until(app, ctx, frame, |app| {
        app.doc
            .sketch
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
    let session = app.doc.sketch.session.as_ref().unwrap();
    let view = ViewSnapshot::new(
        session.document.scene,
        app.view_number,
        app.doc.camera,
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
    app.doc.view = Some(view);
    view
}

fn assert_camera(app: &App) {
    let plane = app.doc.sketch.session.as_ref().unwrap().plane;
    let vector = |v: onecad_core::math::Vec3| onecad_scene::DVec3::new(v.x, v.y, v.z);
    let basis = app.doc.camera.basis().unwrap();
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
        app.doc.sketch.session.as_ref().is_some_and(|session| {
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
    let display = app.doc.display.clone().unwrap();
    let context = crate::sketch_services::document_context(&display);
    let view = ViewSnapshot::new(
        context.scene,
        app.view_number + 1,
        app.doc.camera,
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
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(&tool.draft_status, DraftStatus::Valid(value) if value.value == 5.0)
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    app.dispatch(Action::Confirm);
    pump_until(app, ctx, frame, |app| {
        matches!(
            app.doc.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        )
    })
    .await;
}
