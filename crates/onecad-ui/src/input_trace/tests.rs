use super::*;

#[test]
fn disabled_trace_has_no_state_or_records() {
    let trace = Trace::new(false);
    trace.record("test", Value::Null);
    trace.observe_window(&egui::Context::default());
    assert!(!trace.enabled());
    assert!(trace.state.is_none());
}

#[test]
fn window_transitions_record_without_ui_passes_or_extra_repaints() {
    let context = egui::Context::default();
    let trace = Trace::new(true);
    let mut raw = egui::RawInput::default();
    let sample = |raw: &egui::RawInput| {
        let output = context.run_logic(raw, |ctx| trace.observe_window(ctx));
        assert!(output.viewport_commands.is_empty());
        let state = trace.state.as_ref().unwrap().lock().unwrap();
        (state.sequence, state.window.clone().unwrap())
    };
    let (sequence, initial) = sample(&raw);
    assert_eq!(sequence, 1);
    assert!(initial["occluded"].is_null());
    assert!(initial["inner_rect_points"].is_null());
    assert!(initial["monitor_size_points"].is_null());
    assert_eq!(sample(&raw).0, 1);

    let viewport = raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
    viewport.occluded = Some(true);
    viewport.minimized = Some(false);
    viewport.focused = Some(false);
    viewport.native_pixels_per_point = Some(2.0);
    viewport.inner_rect = Some(egui::Rect::from_min_size(
        egui::pos2(-1280.0, 24.0),
        egui::vec2(1280.0, 820.0),
    ));
    viewport.monitor_size = Some(egui::vec2(2560.0, 1440.0));
    raw.focused = false;
    let (sequence, hidden) = sample(&raw);
    assert_eq!(sequence, 2);
    assert_eq!(hidden["occluded"], true);
    assert_eq!(hidden["visible_for_rendering"], false);
    assert_eq!(hidden["input_focused"], false);
    assert_eq!(
        hidden["inner_rect_points"],
        json!([-1280.0, 24.0, 1280.0, 820.0])
    );
    assert_eq!(hidden["native_pixels_per_point"], 2.0);
    assert_eq!(sample(&raw).0, 2);
    raw.viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .occluded = Some(false);
    let (sequence, visible) = sample(&raw);
    assert_eq!(sequence, 3);
    assert_eq!(visible["visible_for_rendering"], true);
    assert_eq!(context.cumulative_pass_nr(), 0);
}

#[test]
fn trace_stops_after_limit_record() {
    let trace = Trace::new(true);
    for _ in 0..MAX_RECORDS + 10 {
        trace.record("test", Value::Null);
    }
    assert_eq!(
        trace.state.as_ref().unwrap().lock().unwrap().sequence,
        MAX_RECORDS + 1
    );
}

#[test]
fn detail_and_identity_caps_preserve_valid_utf8_and_json() {
    let detail = bounded_detail(json!({"large": "x".repeat(MAX_DETAIL_BYTES)}));
    assert_eq!(
        serde_json::from_str::<Value>(&detail).unwrap(),
        json!({"omitted": "detail_limit"})
    );
    let id = "é".repeat(MAX_ID_BYTES);
    assert!(bounded_id(&id).len() <= MAX_ID_BYTES);
    assert!(id.starts_with(bounded_id(&id)));
}

#[test]
fn text_clipboard_ime_and_typing_keys_never_expose_contents() {
    let secret = "private document name";
    for event in [
        egui::Event::Text(secret.into()),
        egui::Event::Paste(secret.into()),
        egui::Event::Ime(egui::ImeEvent::Commit(secret.into())),
    ] {
        assert!(!event_detail(&event).to_string().contains(secret));
    }
    assert_eq!(safe_key(egui::Key::A, egui::Modifiers::NONE), "redacted");
    assert_eq!(safe_key(egui::Key::Escape, egui::Modifiers::NONE), "Escape");
}

#[test]
fn repaint_observer_keeps_integration_callback_and_never_requests_repaint() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let context = egui::Context::default();
    let callbacks = Arc::new(AtomicU64::new(0));
    let counter = callbacks.clone();
    context.set_request_repaint_callback(move |_| {
        counter.fetch_add(1, Ordering::Relaxed);
    });
    let trace = Arc::new(Trace::new(true));
    trace.observe_repaints(&context);
    assert_eq!(callbacks.load(Ordering::Relaxed), 0);
    context.request_repaint();
    assert!(callbacks.load(Ordering::Relaxed) > 0);
    let records = trace.state.as_ref().unwrap().lock().unwrap().sequence;
    assert!(records > 0);
    trace.record("test", Value::Null);
    assert_eq!(
        trace.state.as_ref().unwrap().lock().unwrap().sequence,
        records + 1
    );
}
