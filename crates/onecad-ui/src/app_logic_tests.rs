//! Real host delivery must progress through eframe logic while presentation is occluded.
use crate::{
    app::App,
    document_flow::{Pending, Request},
    qa::QaPaths,
};
use eframe::App as _;

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn occluded_app_logic_delivers_new_and_protects_close_without_ui() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
    let initial = app
        .display
        .as_ref()
        .unwrap()
        .context
        .runtime_session
        .clone();
    app.document_request(Request::New);
    assert!(app.documents.running);
    pump_until(&mut app, &context, &mut frame, |app| !app.documents.running).await;
    assert_ne!(
        app.display.as_ref().unwrap().context.runtime_session,
        initial
    );
    assert!(app.display.as_ref().unwrap().projection.features.is_empty());
    assert!(app.bridge.valid());
    assert!(!app.ui.start_open);

    app.authoring_dirty = true;
    let output = logic_tick(&mut app, &context, &mut frame, true);
    assert!(has_command(&output, |command| matches!(
        command,
        egui::ViewportCommand::CancelClose
    )));
    assert!(matches!(app.documents.confirm, Some(Pending::Quit)));
    assert!(!app.stopping);
    assert!(!app.may_close);
    let session = app
        .display
        .as_ref()
        .unwrap()
        .context
        .runtime_session
        .clone();
    app.documents.confirm = None;
    logic_tick(&mut app, &context, &mut frame, false);
    assert_eq!(
        app.display.as_ref().unwrap().context.runtime_session,
        session
    );
    assert!(!app.stopping);

    app.authoring_dirty = false;
    logic_tick(&mut app, &context, &mut frame, true);
    assert!(app.stopping);
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
    assert!(
        app.bridge
            .shared
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
    );
}

async fn create_app(root: &std::path::Path) -> (App, egui::Context) {
    let context = egui::Context::default();
    let configuration = crate::viewport_gpu_configuration();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let render =
        egui_wgpu::RenderState::create(&configuration, &instance, None, Default::default())
            .await
            .unwrap();
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
    .unwrap_or_else(|_| panic!("occluded logic did not progress: {}", app.status));
}

fn has_command(
    output: &egui::LogicOutput,
    predicate: impl Fn(&egui::ViewportCommand) -> bool,
) -> bool {
    output.viewport_commands.values().flatten().any(predicate)
}

#[path = "document_lifecycle_tests.rs"]
mod document_lifecycle_tests;
