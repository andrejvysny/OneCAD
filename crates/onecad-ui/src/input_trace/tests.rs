use super::*;

#[test]
fn disabled_trace_has_no_state_or_records() {
    let trace = Trace::new(false);
    trace.record("test", Value::Null);
    assert!(!trace.enabled());
    assert!(trace.state.is_none());
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
