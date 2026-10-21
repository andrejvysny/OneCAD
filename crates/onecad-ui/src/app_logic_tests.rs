//! Real host delivery must progress through eframe logic while presentation is occluded.
use crate::{
    app::App,
    document_flow::{Pending, Request},
    qa::QaPaths,
};
use eframe::App as _;

#[path = "row_delivery_tests.rs"]
mod row_delivery_tests;

#[path = "item_delivery_tests.rs"]
mod item_delivery_tests;

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn occluded_app_logic_delivers_new_and_protects_close_without_ui() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let initial = app
        .doc
        .display
        .as_ref()
        .unwrap()
        .context
        .runtime_session
        .clone();
    app.document_request(Request::New);
    assert!(app.doc.documents.running);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert_ne!(
        app.doc.display.as_ref().unwrap().context.runtime_session,
        initial
    );
    assert!(
        app.doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .features
            .is_empty()
    );
    assert!(app.doc.bridge.valid());
    assert!(!app.ui.start_open);

    app.doc.authoring_dirty = true;
    let output = logic_tick(&mut app, &context, &mut frame, true);
    assert!(has_command(&output, |command| matches!(
        command,
        egui::ViewportCommand::CancelClose
    )));
    assert!(matches!(app.doc.documents.confirm, Some(Pending::Quit)));
    assert!(!app.stopping);
    assert!(!app.may_close);
    let session = app
        .doc
        .display
        .as_ref()
        .unwrap()
        .context
        .runtime_session
        .clone();
    app.doc.documents.confirm = None;
    logic_tick(&mut app, &context, &mut frame, false);
    assert_eq!(
        app.doc.display.as_ref().unwrap().context.runtime_session,
        session
    );
    assert!(!app.stopping);

    app.doc.authoring_dirty = false;
    logic_tick(&mut app, &context, &mut frame, true);
    assert!(app.stopping);
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
    assert!(
        app.doc
            .bridge
            .shared
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
    );
}

async fn create_app(root: &std::path::Path) -> (App, egui::Context) {
    let (app, context) = create_home_app(root).await;
    app.doc
        .bridge
        .submit(crate::bridge::Command::Document(Request::New))
        .unwrap();
    (app, context)
}

async fn create_home_app(root: &std::path::Path) -> (App, egui::Context) {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let worker = std::env::var_os("ONECAD_WORKER_PATH")
        .map(std::path::PathBuf::from)
        .expect("native host gate requires explicit ONECAD_WORKER_PATH");
    assert!(
        worker.is_file(),
        "native host gate requires existing worker"
    );
    assert_eq!(onecad_app::worker::resolve_worker_path(), Some(worker));
    let context = egui::Context::default();
    let configuration = crate::viewport_gpu_configuration();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let render =
        egui_wgpu::RenderState::create(&configuration, &instance, None, Default::default())
            .await
            .unwrap();
    let adapter = render.adapter.get_info();
    eprintln!("Native host gate adapter: {adapter:?}");
    assert_eq!(adapter.backend, wgpu::Backend::Metal);
    let mut creation = eframe::CreationContext::_new_kittest(context.clone());
    creation.wgpu_render_state = Some(render);
    let paths =
        QaPaths::from_args(["--qa-root".into(), root.join("qa").into_os_string()].into_iter())
            .unwrap();
    (App::new(&creation, paths).unwrap(), context)
}

fn logic_tick(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    close: bool,
) -> egui::LogicOutput {
    let mut input = egui::RawInput::default();
    let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
    viewport.occluded = Some(true);
    if close {
        viewport.events.push(egui::ViewportEvent::Close);
    }
    context.run_logic(&input, |context| {
        assert_eq!(context.input(|input| input.viewport().occluded), Some(true));
        app.logic(context, frame);
    })
}

async fn pump_until(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    done: impl Fn(&App) -> bool,
) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            logic_tick(app, context, frame, false);
            if done(app) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("occluded logic did not progress: {}", app.doc.status));
}

fn has_command(
    output: &egui::LogicOutput,
    predicate: impl Fn(&egui::ViewportCommand) -> bool,
) -> bool {
    output.viewport_commands.values().flatten().any(predicate)
}

#[path = "document_lifecycle_tests.rs"]
mod document_lifecycle_tests;

#[path = "document_tabs_tests.rs"]
mod document_tabs_tests;

#[path = "document_tab_delivery_tests.rs"]
mod document_tab_delivery_tests;

#[path = "overlay_routing_tests.rs"]
mod overlay_routing_tests;

#[path = "diagnostic_delivery_tests.rs"]
mod diagnostic_delivery_tests;

#[path = "dimension_delivery_tests.rs"]
mod dimension_delivery_tests;

#[path = "plate_delivery_tests.rs"]
mod plate_delivery_tests;

#[path = "numeric_delivery_tests.rs"]
mod numeric_delivery_tests;

#[path = "profile_delivery_tests.rs"]
mod profile_delivery_tests;

#[path = "selection_delivery_tests.rs"]
mod selection_delivery_tests;

#[path = "gesture_delivery_tests.rs"]
mod gesture_delivery_tests;
#[path = "point_center_delivery_tests.rs"]
mod point_center_delivery_tests;

#[path = "arc_gesture_delivery_tests.rs"]
mod arc_gesture_delivery_tests;

#[path = "arc_slot_delivery_tests.rs"]
mod arc_slot_delivery_tests;

#[path = "sketch_entry_delivery_tests.rs"]
mod sketch_entry_delivery_tests;
#[path = "sketch_plane_choice_tests.rs"]
mod sketch_plane_choice_tests;

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn home_without_document_and_empty_new_share_back_and_escape_predicate() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_home_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.bridge.shared.recovery.lock().unwrap().checked
    })
    .await;
    assert!(app.ui.start_open);
    assert!(!app.document_open());
    assert!(!app.document_dirty());
    crate::start_screen::leave(&mut app);
    assert!(app.ui.start_open);
    assert!(!crate::bars::save_allowed(&app));
    assert!(!crate::bars::export_allowed(&app));
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert!(app.document_open());
    assert!(app.doc.document_state.as_ref().unwrap().path.is_none());
    let session = app
        .doc
        .document_state
        .as_ref()
        .unwrap()
        .runtime_session
        .clone();
    app.ui.start_open = true;
    crate::start_screen::leave(&mut app);
    assert!(!app.ui.start_open);
    assert_eq!(
        app.doc.document_state.as_ref().unwrap().runtime_session,
        session
    );
    assert!(crate::bars::save_allowed(&app));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[path = "capability_delivery_tests.rs"]
mod capability_delivery_tests;

#[path = "extrude_handle_delivery_tests.rs"]
mod extrude_handle_delivery_tests;

#[path = "sketch_selection_delivery_tests.rs"]
mod sketch_selection_delivery_tests;

#[path = "sketch_finish_delivery_tests.rs"]
mod sketch_finish_delivery_tests;
