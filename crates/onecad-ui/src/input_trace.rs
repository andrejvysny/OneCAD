//! Opt-in, bounded native input evidence; this does not observe physical presentation.
use crate::{app::App, bridge::Display};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(crate) mod actions;

const MAX_RECORDS: u64 = 16_384;
const MAX_DETAIL_BYTES: usize = 2_048;
const MAX_EVENTS_PER_BATCH: usize = 64;
const MAX_ID_BYTES: usize = 128;

pub(crate) struct Trace {
    start: Instant,
    state: Option<Mutex<State>>,
}

struct State {
    sequence: u64,
    context: Value,
    window: Option<Value>,
}

impl Trace {
    pub(crate) fn from_environment(context: &egui::Context) -> Arc<Self> {
        let enabled = std::env::var("ONECAD_INPUT_TRACE").as_deref() == Ok("1");
        let trace = Arc::new(Self::new(enabled));
        if enabled {
            trace.observe_repaints(context);
            trace.record("trace.start", json!({"max_records": MAX_RECORDS,
                "max_detail_bytes": MAX_DETAIL_BYTES, "max_events_per_batch": MAX_EVENTS_PER_BATCH}));
        }
        trace
    }

    fn observe_repaints(self: &Arc<Self>, context: &egui::Context) {
        let observer = self.clone();
        // The observer leaves eframe's repaint callback and scheduling intact.
        context.set_repaint_observer(move |request| {
            observer.record(
                "repaint.request",
                json!({
                    "pass": request.current_cumulative_pass_nr,
                    "delay_us": request.delay.as_micros().min(u128::from(u64::MAX)) as u64,
                    "viewport": format!("{:?}", request.viewport_id),
                }),
            );
        });
    }

    fn new(enabled: bool) -> Self {
        Self {
            start: Instant::now(),
            state: enabled.then(|| {
                Mutex::new(State {
                    sequence: 0,
                    context: Value::Null,
                    window: None,
                })
            }),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.state.is_some()
    }

    pub(crate) fn observe_window(&self, context: &egui::Context) {
        let Some(state) = &self.state else {
            return;
        };
        let detail = context.input(|input| {
            let viewport = input.viewport();
            json!({
                "viewport": format!("{:?}", input.raw.viewport_id),
                "focused": viewport.focused,
                "input_focused": input.focused,
                "occluded": viewport.occluded,
                "minimized": viewport.minimized,
                "visible_for_rendering": viewport.visible(),
                "inner_rect_points": viewport.inner_rect.map(rect_points),
                "outer_rect_points": viewport.outer_rect.map(rect_points),
                "monitor_size_points": viewport.monitor_size.map(|size| [size.x, size.y]),
                "native_pixels_per_point": viewport.native_pixels_per_point,
                "pixels_per_point": input.pixels_per_point,
            })
        });
        let Ok(mut state) = state.lock() else {
            return;
        };
        if state.window.as_ref() == Some(&detail) {
            return;
        }
        state.window = Some(detail.clone());
        drop(state);
        self.record("host.window_state", detail);
    }

    pub(crate) fn record(&self, kind: &'static str, detail: Value) {
        let Some(state) = &self.state else {
            return;
        };
        let Ok(mut state) = state.lock() else {
            return;
        };
        if state.sequence > MAX_RECORDS {
            return;
        }
        state.sequence += 1;
        let kind = if state.sequence > MAX_RECORDS {
            "trace.limit"
        } else {
            kind
        };
        let detail = bounded_detail(detail);
        let thread = std::thread::current();
        tracing::info!(target: "onecad_input", sequence = state.sequence,
            thread = bounded_id(thread.name().unwrap_or("unnamed")),
            elapsed_us = self.start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            kind, context = %state.context, detail = %detail, "native.input_trace");
    }

    pub(crate) fn gpu_submitted(
        self: &Arc<Self>,
        queue: &wgpu::Queue,
        display: &Display,
        view: &onecad_scene::ViewSnapshot,
        preview: bool,
    ) {
        if !self.enabled() {
            return;
        }
        let mut detail = display_identity(display);
        detail["view"] = json!(view.identity().view);
        detail["preview"] = json!(preview);
        detail["logical_size"] = json!([view.viewport().width, view.viewport().height]);
        detail["scale_factor"] = json!(view.viewport().scale_factor);
        self.record("viewport.gpu_submit", detail.clone());
        let trace = self.clone();
        queue.on_submitted_work_done(move || trace.record("viewport.gpu_complete", detail));
    }

    pub(crate) fn capture(&self, kind: &'static str, display: &Display) {
        if self.enabled() {
            self.record(kind, display_identity(display));
        }
    }
}

fn rect_points(rect: egui::Rect) -> [f32; 4] {
    [rect.min.x, rect.min.y, rect.width(), rect.height()]
}

fn bounded_detail(detail: Value) -> String {
    let encoded = detail.to_string();
    if encoded.len() > MAX_DETAIL_BYTES {
        "{\"omitted\":\"detail_limit\"}".into()
    } else {
        encoded
    }
}

pub(crate) fn bounded_id(id: &str) -> &str {
    let mut end = id.len().min(MAX_ID_BYTES);
    while !id.is_char_boundary(end) {
        end -= 1;
    }
    &id[..end]
}

fn display_identity(display: &Display) -> Value {
    json!({"document": bounded_id(&display.context.document_id),
        "runtime_session": bounded_id(&display.context.runtime_session),
        "revision": display.context.revision.0, "epoch": display.epoch,
        "scene_session": display.scene.identity().session, "scene": display.scene.identity().scene})
}

pub(crate) fn app(app: &App, kind: &'static str, detail: Value) {
    let trace = &app.doc.bridge.shared.input_trace;
    let Some(state) = &trace.state else {
        return;
    };
    let mut context = app
        .doc
        .display
        .as_ref()
        .map_or(Value::Null, |display| display_identity(display));
    if let Value::Object(fields) = &mut context {
        fields.insert(
            "view".into(),
            app.doc
                .view
                .map(|view| json!(view.identity().view))
                .unwrap_or(Value::Null),
        );
        fields.insert(
            "view_scene".into(),
            app.doc
                .view
                .map(|view| json!(view.identity().scene.scene))
                .unwrap_or(Value::Null),
        );
        fields.insert(
            "view_scene_session".into(),
            app.doc
                .view
                .map(|view| json!(view.identity().scene.session))
                .unwrap_or(Value::Null),
        );
        fields.insert("pointer_owner".into(), json!(pointer_owner(app)));
        if let Some(tool) = &app.doc.interaction.tool {
            fields.insert("model_gesture".into(), json!(tool.tag.gesture));
            fields.insert("model_sequence".into(), json!(tool.tag.sequence));
            fields.insert("model_phase".into(), json!(format!("{:?}", tool.phase)));
        }
        if let Some(session) = &app.doc.sketch.session {
            fields.insert("sketch".into(), json!(session.key.sketch_id.to_string()));
            fields.insert("sketch_generation".into(), json!(session.key.generation));
            fields.insert("sketch_phase".into(), json!(format!("{:?}", session.phase)));
            fields.insert(
                "gesture_phase".into(),
                json!(format!("{:?}", app.doc.sketch_gesture.phase())),
            );
        }
    }
    if let Ok(mut state) = state.lock() {
        state.context = context;
    }
    trace.record(kind, detail);
}

fn pointer_owner(app: &App) -> &'static str {
    let context = &app.doc.bridge.shared.context;
    if crate::input::overlay_owns_keyboard(app, context) {
        "overlay"
    } else if context.text_edit_focused() {
        "text_editor"
    } else if app.doc.transform_gizmo.gizmo.active() {
        "transform_gizmo"
    } else if app.doc.model_handle.active() {
        "model_handle"
    } else if app.doc.extrude_handle.active() {
        "extrude_handle"
    } else if app.doc.sketch_gesture.busy() {
        "sketch_drag"
    } else if app.doc.armed_sketch_gesture.is_some() {
        "sketch_armed"
    } else if app
        .doc
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| session.selection_dragging)
    {
        "sketch_marquee"
    } else if crate::sketch_creation::chain_input::pointer_owned(context) {
        "sketch_chain"
    } else if context.input(|input| input.pointer.secondary_down() || input.pointer.middle_down()) {
        "navigation_candidate"
    } else {
        "unclaimed"
    }
}

pub(crate) fn input(app: &App, ctx: &egui::Context, raw: &egui::RawInput) {
    if !app.doc.bridge.shared.input_trace.enabled() {
        return;
    }
    self::app(
        app,
        "input.batch",
        json!({"pass": ctx.cumulative_pass_nr(),
        "focused": raw.focused, "pixels_per_point": ctx.pixels_per_point(),
        "event_count": raw.events.len(), "overlay": crate::input::overlay_owns_keyboard(app, ctx),
        "text_focus": ctx.text_edit_focused()}),
    );
    for (index, event) in raw.events.iter().take(MAX_EVENTS_PER_BATCH).enumerate() {
        app.doc.bridge.shared.input_trace.record(
            "input.event",
            json!({"index": index, "event": event_detail(event)}),
        );
    }
    if raw.events.len() > MAX_EVENTS_PER_BATCH {
        app.doc.bridge.shared.input_trace.record(
            "input.omitted",
            json!({"count": raw.events.len() - MAX_EVENTS_PER_BATCH}),
        );
    }
}

fn event_detail(event: &egui::Event) -> Value {
    use egui::Event;
    match event {
        Event::PointerMoved(pos) => json!({"type": "pointer.move", "logical": [pos.x, pos.y]}),
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        } => json!({"type": "pointer.button",
            "logical": [pos.x, pos.y], "button": format!("{button:?}"), "pressed": pressed, "modifiers": format!("{modifiers:?}")}),
        Event::Key {
            key,
            pressed,
            repeat,
            modifiers,
            ..
        } => json!({"type": "key", "key": safe_key(*key, *modifiers),
            "pressed": pressed, "repeat": repeat, "modifiers": format!("{modifiers:?}")}),
        Event::WindowFocused(focused) => json!({"type": "window.focus", "focused": focused}),
        Event::PointerGone => json!({"type": "pointer.gone"}),
        Event::Text(text) => json!({"type": "text", "bytes": text.len(), "contents": "redacted"}),
        Event::Paste(_) | Event::PasteImage(_) | Event::MiddleClickPaste { .. } => {
            json!({"type": "paste", "contents": "redacted"})
        }
        Event::Ime(_) => json!({"type": "ime", "contents": "redacted"}),
        Event::ModifiersChanged(modifiers) => {
            json!({"type": "modifiers", "modifiers": format!("{modifiers:?}")})
        }
        Event::MouseWheel { delta, .. } => json!({"type": "wheel", "delta": [delta.x, delta.y]}),
        Event::MouseMoved(delta) => json!({"type": "mouse.motion", "delta": [delta.x, delta.y]}),
        Event::Zoom(scale) => json!({"type": "zoom", "scale": scale}),
        Event::Rotate(radians) => json!({"type": "rotate", "radians": radians}),
        Event::Touch { pos, phase, .. } => {
            json!({"type": "touch", "logical": [pos.x,pos.y], "phase": format!("{phase:?}")})
        }
        Event::Copy => json!({"type": "copy"}),
        Event::Cut => json!({"type": "cut"}),
        Event::AccessKitActionRequest(_) => {
            json!({"type": "accessibility", "contents": "redacted"})
        }
    }
}

fn safe_key(key: egui::Key, modifiers: egui::Modifiers) -> String {
    if modifiers.command
        || matches!(
            key,
            egui::Key::Escape
                | egui::Key::Enter
                | egui::Key::Tab
                | egui::Key::Delete
                | egui::Key::Backspace
                | egui::Key::ArrowDown
                | egui::Key::ArrowUp
                | egui::Key::ArrowLeft
                | egui::Key::ArrowRight
        )
    {
        format!("{key:?}")
    } else {
        "redacted".into()
    }
}

#[cfg(test)]
mod tests;
