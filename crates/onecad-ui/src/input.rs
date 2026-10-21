//! Text owns editing shortcuts; model history is only used without text focus.
use crate::{app::App, bridge::Command, document_flow::Request, menu};

mod overlay;

pub(crate) fn overlay_owns_keyboard(app: &App, ctx: &egui::Context) -> bool {
    app.input_routing.overlay_blocks(ctx.cumulative_pass_nr())
        || app.documents.confirm.is_some()
        || crate::constraint_authoring::editor_open(ctx)
        || crate::sketch_numeric::editor_open(ctx)
        || ui_owns_keyboard(&app.ui, ctx)
}

pub(crate) fn ui_owns_keyboard(ui: &crate::ui_state::UiState, ctx: &egui::Context) -> bool {
    overlay::owner(ui, false, egui::Popup::is_any_open(ctx)).is_some()
}

pub(crate) fn navigation_blocked(app: &App, ctx: &egui::Context) -> bool {
    let document = app.documents.confirm.is_some()
        || crate::constraint_authoring::editor_open(ctx)
        || crate::sketch_numeric::editor_open(ctx);
    overlay::navigation_blocked(&app.ui, document, egui::Popup::is_any_open(ctx))
}

#[derive(Default)]
pub(crate) struct RoutingState {
    pass: u64,
    retained: usize,
    overlay_blocked: bool,
}

impl RoutingState {
    fn overlay_blocks(&self, pass: u64) -> bool {
        self.pass == pass && self.overlay_blocked
    }
}

fn route_events(
    state: &mut RoutingState,
    pass: u64,
    raw: &mut egui::RawInput,
    fresh: impl FnOnce(&mut egui::RawInput),
) {
    // Logic-only eframe ticks prepend input that egui has not consumed yet.
    // Keep that prefix for text widgets without dispatching its CAD actions again.
    let retained = if state.pass == pass {
        state.retained.min(raw.events.len())
    } else {
        0
    };
    let events = raw.events.split_off(retained);
    let mut prefix = std::mem::replace(&mut raw.events, events);
    fresh(raw);
    prefix.append(&mut raw.events);
    raw.events = prefix;
    state.pass = pass;
    state.retained = raw.events.len();
}

fn history_shortcut(event: &egui::Event, text: bool, focused: bool) -> Option<Command> {
    match event {
        egui::Event::Key {
            key: egui::Key::Z,
            pressed: true,
            modifiers,
            ..
        } if modifiers.command && !text && focused => Some(if modifiers.shift {
            Command::Redo
        } else {
            Command::Undo
        }),
        _ => None,
    }
}

pub fn route(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    let mut state = std::mem::take(&mut app.input_routing);
    let pass = ctx.cumulative_pass_nr();
    let owned = app.documents.confirm.is_some()
        || crate::constraint_authoring::editor_open(ctx)
        || crate::sketch_numeric::editor_open(ctx)
        || ui_owns_keyboard(&app.ui, ctx);
    state.overlay_blocked = owned || state.overlay_blocks(pass);
    route_events(&mut state, pass, raw, |raw| route_fresh(app, ctx, raw));
    app.input_routing = state;
}

fn route_fresh(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    app.poll_device_failure();
    app.sync_document_state();
    crate::input_trace::input(app, ctx, raw);
    crate::sketch_gestures::route_raw(app, ctx, raw);
    crate::sketch_creation::chain_input::route_raw(app, ctx, raw);
    if app.sampled && app.viewport_ready() {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = done.clone();
        app.gpu.queue.on_submitted_work_done(move || {
            finished.store(true, std::sync::atomic::Ordering::Release)
        });
        app.sampling_done = done;
        app.sampled = false;
    }
    if app.cad_needs_fence && app.viewport_ready() {
        let context = ctx.clone();
        if let Err(error) = app.renderer.after_submit(move || context.request_repaint()) {
            app.fail_viewport(error.to_string());
        }
        app.cad_needs_fence = false;
    }
    let owner = overlay::owner(
        &app.ui,
        app.documents.confirm.is_some()
            || crate::constraint_authoring::editor_open(ctx)
            || crate::sketch_numeric::editor_open(ctx),
        egui::Popup::is_any_open(ctx),
    );
    if let Some(menu) = &app.menu {
        let text = text_owns_keyboard(ctx);
        menu.set_history_enabled(
            text || owner.is_none() && history_available(app, false),
            text || owner.is_none() && history_available(app, true),
        );
        menu.set_document_enabled(
            owner.is_none() && crate::bars::replacement_allowed(app),
            owner.is_none() && crate::bars::save_allowed(app),
            owner.is_none() && crate::bars::export_allowed(app),
        );
    }
    let actions = app.menu.as_ref().map_or_else(Vec::new, |menu| menu.drain());
    menu::remove_duplicate_events(&actions, &mut raw.events);
    let text = text_owns_keyboard(ctx);
    let composition = app.composing
        || raw
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Ime(_)));
    protect_composition(&mut app.composing, raw, text);
    if let Some(owner) = owner {
        for action in actions {
            if app.bridge.shared.input_trace.enabled() {
                crate::input_trace::app(
                    app,
                    "menu.route",
                    serde_json::json!({"action": format!("{action:?}"), "text_focus": text, "focused": raw.focused, "overlay": true, "allowed": overlay::text_menu(action, text, raw.focused)}),
                );
            }
            if overlay::text_menu(action, text, raw.focused) {
                route_menu(app, ctx, &mut raw.events, action, text);
            }
        }
        overlay::dismiss(&mut app.ui, owner, raw, text);
        return;
    }
    for action in actions {
        if app.bridge.shared.input_trace.enabled() {
            crate::input_trace::app(
                app,
                "menu.route",
                serde_json::json!({"action": format!("{action:?}"), "text_focus": text, "focused": raw.focused, "overlay": false}),
            );
        }
        route_menu(app, ctx, &mut raw.events, action, text);
    }
    let mut commands = Vec::new();
    let mut documents = Vec::new();
    raw.events.retain(|event| {
        if let Some(command) = history_shortcut(event, text || composition, raw.focused) {
            commands.push(command);
            return false;
        }
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
            && modifiers.command
            && !composition
        {
            match key {
                _ if raw.focused && document_shortcut(*key, *modifiers).is_some() => {
                    if let Some(request) = document_shortcut(*key, *modifiers) {
                        documents.push(request);
                    }
                    return false;
                }
                egui::Key::Q => {
                    app.close_prompt = true;
                    return false;
                }
                _ => {}
            }
        }
        true
    });
    for request in documents {
        app.document_request(request);
    }
    for command in commands {
        submit_history(app, command);
    }
    if app.documents.confirm.is_some() {
        return;
    }
    if !text && raw.focused && !composition && app.sketch_active() && app.viewport_ready() {
        sketch_arc_keys(app, ctx, &mut raw.events);
    }
    if !text && !composition && raw.focused && crate::sketch_numeric::route(app, ctx, raw) {
        return;
    }
    if raw.focused && !composition {
        crate::editor_facade::keys(app, raw, text);
    }
    if !text && !composition && raw.focused && app.sketch_active() {
        let delete = raw.events.iter().any(|event| matches!(event,
            egui::Event::Key { key: egui::Key::Delete | egui::Key::Backspace, pressed: true, modifiers, .. }
                if !modifiers.command && !modifiers.alt && !modifiers.ctrl));
        if delete {
            crate::sketch_repair::delete_selected(app);
            raw.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Delete | egui::Key::Backspace,
                        ..
                    }
                )
            });
        }
    }
    if !text && !composition && raw.focused && app.sketch_active() && app.viewport_ready() {
        let keys: Vec<_> = raw
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    repeat: false,
                    ..
                } if !modifiers.command && matches!(key, egui::Key::Enter | egui::Key::Escape) => {
                    Some(*key)
                }
                _ => None,
            })
            .collect();
        raw.events.retain(|event| {
            !matches!(event, egui::Event::Key {
            key: egui::Key::Enter | egui::Key::Escape, pressed: true, modifiers, ..
        } if !modifiers.command)
        });
        for key in keys {
            app.sketch_dispatch(if key == egui::Key::Enter {
                onecad_interaction::sketch::SketchAction::Enter
            } else {
                onecad_interaction::sketch::SketchAction::Escape
            });
        }
    }
    if !text
        && !composition
        && raw.focused
        && !app.sketch_active()
        && raw.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                }
            )
        })
    {
        crate::pill::cancel_or_deselect(app);
    }
}

fn route_menu(
    app: &mut App,
    ctx: &egui::Context,
    events: &mut Vec<egui::Event>,
    action: menu::Action,
    text: bool,
) {
    if text && route_text_menu(events, action) {
        return;
    }
    match action {
        menu::Action::Undo => submit_history(app, Command::Undo),
        menu::Action::Redo => submit_history(app, Command::Redo),
        menu::Action::New => app.document_request(Request::New),
        menu::Action::Open => app.document_request(Request::Open(None)),
        menu::Action::Import => app.document_request(Request::Import),
        menu::Action::Save => app.document_request(Request::Save { save_as: false }),
        menu::Action::SaveAs => app.document_request(Request::Save { save_as: true }),
        menu::Action::Export(format) => app.document_request(Request::Export(format)),
        menu::Action::Quit => app.close_prompt = true,
        menu::Action::Fit => app.fit_pending = true,
        menu::Action::Home => {
            app.camera = onecad_scene::Camera::default();
            app.fit_pending = true;
        }
        menu::Action::Appearance(choice) => {
            app.ui.appearance = choice;
            crate::appearance::sync(app, ctx);
            if let Some(menu) = &app.menu {
                menu.set_appearance(choice);
            }
        }
        menu::Action::Paste if text => {
            #[cfg(target_os = "macos")]
            if let Ok(text) =
                arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
            {
                events.push(egui::Event::Paste(text));
            }
        }
        _ => {}
    }
}

fn document_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<Request> {
    if !modifiers.command || modifiers.alt {
        return None;
    }
    match key {
        egui::Key::N if !modifiers.shift => Some(Request::New),
        egui::Key::O if !modifiers.shift => Some(Request::Open(None)),
        egui::Key::S => Some(Request::Save {
            save_as: modifiers.shift,
        }),
        _ => None,
    }
}

pub(crate) fn history_available(app: &App, redo: bool) -> bool {
    if app.busy() {
        return false;
    }
    if app.sketch_active() {
        return crate::sketch_ui::tools_enabled(app)
            && app.sketch.session.as_ref().is_some_and(|session| {
                session.draft.is_some()
                    || session.history.as_ref().is_some_and(|history| {
                        if redo {
                            history.redo_depth > 0
                        } else {
                            history.undo_depth > 0
                        }
                    })
            });
    }
    app.viewport_ready()
        && app.bridge.valid()
        && !crate::viewport_selection::sectioned(app)
        && app.display.as_ref().is_some_and(|display| {
            if redo {
                display.projection.redo_depth > 0
            } else {
                display.projection.undo_depth > 0
            }
        })
}

pub(crate) fn submit_history(app: &mut App, command: Command) {
    if app.sketch_active() {
        let direction = match command {
            Command::Undo => onecad_core::edit::HistoryDirection::Undo,
            Command::Redo => onecad_core::edit::HistoryDirection::Redo,
            _ => return,
        };
        app.sketch_dispatch(onecad_interaction::sketch::SketchAction::History(direction));
    } else {
        app.submit(command);
    }
}

fn route_text_menu(events: &mut Vec<egui::Event>, action: menu::Action) -> bool {
    match action {
        menu::Action::Undo | menu::Action::Redo => {
            menu::push_command(events, egui::Key::Z, action == menu::Action::Redo);
        }
        menu::Action::Cut => events.push(egui::Event::Cut),
        menu::Action::Copy => events.push(egui::Event::Copy),
        menu::Action::SelectAll => menu::push_command(events, egui::Key::A, false),
        _ => return false,
    }
    true
}

/// All native editors use egui text state, independent of their widget IDs.
pub(crate) fn text_owns_keyboard(ctx: &egui::Context) -> bool {
    ctx.text_edit_focused()
}

pub(crate) fn protect_composition(composing: &mut bool, raw: &mut egui::RawInput, text: bool) {
    let mut protected = *composing || !raw.focused;
    for event in &raw.events {
        match event {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                *composing = !text.is_empty();
                protected = true;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                *composing = false;
                protected = true;
            }
            #[allow(deprecated)]
            egui::Event::Ime(egui::ImeEvent::Disabled) => {
                protected = true;
                *composing = false;
            }
            _ => {}
        }
    }
    if !text || !raw.focused {
        *composing = false;
    }
    if protected {
        raw.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Enter | egui::Key::Escape,
                    pressed: true,
                    ..
                }
            )
        });
    }
}

#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod tests;

fn arc_key(event: &egui::Event) -> Option<bool> {
    match event {
        egui::Event::Key {
            key: egui::Key::A,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if !modifiers.command && !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => {
            Some(modifiers.shift)
        }
        _ => None,
    }
}

fn sketch_arc_keys(app: &mut App, ctx: &egui::Context, events: &mut Vec<egui::Event>) {
    events.retain(|event| {
        let Some(three_point) = arc_key(event) else {
            return true;
        };
        if !three_point
            && let Some(session) = app
                .sketch
                .session
                .as_ref()
                .filter(|session| session.can_tangent_arc())
        {
            let key = session.key.clone();
            let enabled = !session
                .draft
                .as_ref()
                .is_some_and(|draft| draft.tangent_arc);
            app.sketch_dispatch(onecad_interaction::sketch::SketchAction::SetTangentArc {
                key,
                enabled,
            });
        } else {
            app.ui.arc_three_point = three_point;
            crate::pill::activate(app, ctx, crate::tool_registry::ToolId::Arc);
        }
        false
    });
}

#[cfg(test)]
mod arc_key_tests {
    use super::*;
    fn key(modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }
    #[test]
    fn arc_keys_keep_modifiers_and_repeat_distinct() {
        assert_eq!(arc_key(&key(egui::Modifiers::NONE, false)), Some(false));
        assert_eq!(arc_key(&key(egui::Modifiers::SHIFT, false)), Some(true));
        for modifiers in [
            egui::Modifiers::ALT,
            egui::Modifiers::CTRL,
            egui::Modifiers::COMMAND,
            egui::Modifiers::MAC_CMD,
        ] {
            assert_eq!(arc_key(&key(modifiers, false)), None);
        }
        assert_eq!(arc_key(&key(egui::Modifiers::NONE, true)), None);
    }
}
