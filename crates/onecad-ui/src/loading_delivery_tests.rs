use super::point_center_delivery_tests::support::widgets;
use super::*;

fn draw(app: &mut App, context: &egui::Context) -> egui::FullOutput {
    let mut output = app.frame(
        context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            focused: false,
            ..Default::default()
        },
    );
    output.textures_delta.clear();
    output
}

async fn scheduled_frame(
    app: &mut App,
    context: &egui::Context,
    requests: &mut tokio::sync::mpsc::UnboundedReceiver<std::time::Duration>,
    previous: &egui::FullOutput,
) -> egui::FullOutput {
    let delay = previous.viewport_output[&egui::ViewportId::ROOT]
        .repaint_delay
        .min(std::time::Duration::from_secs(60));
    tokio::select! {
        _ = tokio::time::sleep(delay) => {},
        Some(request) = requests.recv() => tokio::time::sleep(request.min(delay)).await,
    }
    draw(app, context)
}

async fn open_waiting(root: &std::path::Path) -> (App, egui::Context, crate::budget::Lease) {
    let (mut app, context) = create_home_app(root).await;
    context.enable_accesskit();
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.bridge.shared.recovery.lock().unwrap().ready() && app.doc.command_status
    })
    .await;
    let budget = app.doc.bridge.shared.host_budget.clone();
    let capacity = crate::budget::Policy::for_profile(app.renderer.profile()).aggregate;
    let held = budget
        .reserve_kind(
            capacity - budget.snapshot().used,
            crate::budget::Kind::Preparation,
        )
        .unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    app.document_request(Request::Open(Some(source)));
    assert!(app.doc.documents.running, "{}", app.doc.status);
    app.ui.start_open = false;
    (app, context, held)
}

fn assert_loading(app: &App, output: &egui::FullOutput) {
    assert!(
        widgets(output)
            .iter()
            .any(|(label, _)| label == "Opening project…")
    );
    assert!(output.shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::Shape::Rect(painted) if painted.fill == app.theme.canvas_model
            && painted.rect.contains(egui::pos2(640.0, 410.0))
    )));
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
            <= crate::viewport_loading::POLL_INTERVAL
    );
    assert!(app.doc.display.is_none());
}

fn poll_delay(app: &App, context: &egui::Context, hidden: bool) -> std::time::Duration {
    let mut raw = egui::RawInput::default();
    let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
    viewport.occluded = Some(hidden);
    viewport.minimized = Some(hidden);
    let delay = std::sync::Arc::new(std::sync::Mutex::new(std::time::Duration::MAX));
    let _ = context.run_logic(&raw, |context| {
        let observed = delay.clone();
        context.set_repaint_observer(move |request| {
            let mut delay = observed.lock().unwrap();
            *delay = (*delay).min(request.delay);
        });
        crate::viewport_loading::repaint(app, context);
        context.set_repaint_observer(|_| {});
    });
    *delay.lock().unwrap()
}

fn assert_poll_lifecycle(app: &mut App, context: &egui::Context) {
    assert!(!crate::viewport_loading::pending(app));
    app.doc.awaiting_scene = true;
    assert!(crate::viewport_loading::pending(app));
    assert_eq!(poll_delay(app, context, true), std::time::Duration::MAX);
    assert!(poll_delay(app, context, false) <= crate::viewport_loading::POLL_INTERVAL);
    app.doc.display_error = Some("scene refused".into());
    assert!(!crate::viewport_loading::pending(app));
    assert_eq!(poll_delay(app, context, false), std::time::Duration::MAX);
    app.doc.display_error = None;
    app.ui.start_open = true;
    assert!(!crate::viewport_loading::pending(app));
    app.ui.start_open = false;
    app.doc.awaiting_scene = false;
}

fn assert_recovery_feedback(app: &mut App, context: &egui::Context) {
    let texture = app.texture.take();
    let output = draw(app, context);
    assert!(
        widgets(&output)
            .iter()
            .any(|(label, _)| label == "Preparing view…")
    );
    assert!(crate::viewport_loading::pending(app));
    app.resize_refusal = Some((app.renderer.size(), "Allocation refused".into()));
    let output = draw(app, context);
    assert!(
        widgets(&output)
            .iter()
            .any(|(label, _)| label == "Retry viewport")
    );
    assert!(
        !widgets(&output)
            .iter()
            .any(|(label, _)| label == "Preparing view…")
    );
    let display = app.doc.display.take();
    let output = draw(app, context);
    assert!(
        widgets(&output)
            .iter()
            .any(|(label, _)| label == "Model unavailable")
    );
    app.doc.display = display;
    app.resize_refusal = None;
    app.doc.awaiting_scene = true;
    app.doc.display_error = Some("Scene refresh refused".into());
    let output = draw(app, context);
    assert!(
        widgets(&output)
            .iter()
            .any(|(label, _)| label == "Model unavailable")
    );
    assert!(
        !widgets(&output)
            .iter()
            .any(|(label, _)| label == "Preparing view…")
    );
    app.doc.display_error = None;
    app.doc.awaiting_scene = false;
    app.stopping = true;
    let output = draw(app, context);
    assert!(
        widgets(&output)
            .iter()
            .any(|(label, _)| label == "Closing project…")
    );
    assert!(
        !widgets(&output)
            .iter()
            .any(|(label, _)| label == "Preparing view…")
    );
    app.stopping = false;
    app.texture = texture;
    let output = draw(app, context);
    assert!(
        !widgets(&output)
            .iter()
            .any(|(label, _)| label == "Preparing view…")
    );
    assert!(!crate::viewport_loading::pending(app));
}

#[tokio::test]
#[ignore = "requires production worker and Metal"]
async fn opening_project_paints_loading_and_finishes_without_input() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, held) = open_waiting(area.path()).await;
    let (sender, mut requests) = tokio::sync::mpsc::unbounded_channel();
    context.set_request_repaint_callback(move |request| {
        let _ = sender.send(request.delay);
    });
    let mut output = draw(&mut app, &context);
    assert_loading(&app, &output);
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while app.doc.display.is_none() || app.texture.is_none() || app.doc.documents.running {
            output = scheduled_frame(&mut app, &context, &mut requests, &output).await;
        }
    })
    .await
    .unwrap();
    let output = draw(&mut app, &context);
    assert!(
        !widgets(&output)
            .iter()
            .any(|(label, _)| label == "Opening project…")
    );
    assert!(app.doc.display_error.is_none());
    assert_recovery_feedback(&mut app, &context);
    assert_poll_lifecycle(&mut app, &context);
    app.stop_discard();
    assert!(!crate::viewport_loading::pending(&app));
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
