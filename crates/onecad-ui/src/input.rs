//! Text owns editing shortcuts; model history is only used without text focus.
use crate::{app::App, bridge::Command, document_flow::Request, menu};

#[derive(Default)]
pub(crate) struct RoutingState {
    pass: u64,
    retained: usize,
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
    route_events(&mut state, ctx.cumulative_pass_nr(), raw, |raw| {
        route_fresh(app, ctx, raw)
    });
    app.input_routing = state;
}

fn route_fresh(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    app.poll_device_failure();
    app.sync_document_state();
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
    if let Some(menu) = &app.menu {
        let text = text_owns_keyboard(ctx);
        menu.set_history_enabled(
            text || history_available(app, false),
            text || history_available(app, true),
        );
        menu.set_document_enabled(
            crate::bars::replacement_allowed(app),
            crate::bars::save_allowed(app),
            crate::bars::export_allowed(app),
        );
    }
    let actions = app.menu.as_ref().map_or_else(Vec::new, |menu| menu.drain());
    menu::remove_duplicate_events(&actions, &mut raw.events);
    let text = text_owns_keyboard(ctx);
    protect_composition(&mut app.composing, raw, text);
    for action in actions {
        route_menu(app, ctx, &mut raw.events, action, text);
    }
    let mut commands = Vec::new();
    let mut documents = Vec::new();
    raw.events.retain(|event| {
        if let Some(command) = history_shortcut(event, text, raw.focused) {
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
    if !text && raw.focused {
        close_constraints_on_escape(app, raw);
    }
    if !text && raw.focused && app.sketch_active() && app.viewport_ready() {
        let keys: Vec<_> = raw
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.command && matches!(key, egui::Key::Enter | egui::Key::Escape) => {
                    Some(*key)
                }
                _ => None,
            })
            .collect();
        for key in keys {
            app.sketch_dispatch(if key == egui::Key::Enter {
                onecad_interaction::sketch::SketchAction::Enter
            } else {
                onecad_interaction::sketch::SketchAction::Escape
            });
        }
    }
    if !text
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

/// Escape closes an open Constraints menu and is consumed so the sketch does not also see it.
fn close_constraints_on_escape(app: &mut App, raw: &mut egui::RawInput) {
    if !app.ui.constraints_open {
        return;
    }
    let is_escape = |event: &egui::Event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            }
        )
    };
    if raw.events.iter().any(is_escape) {
        raw.events.retain(|event| !is_escape(event));
        app.ui.constraints_open = false;
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
mod tests;
