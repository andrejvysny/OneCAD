//! Text owns editing shortcuts; model history is only used without text focus.
use crate::{app::App, bridge::Command, menu};
use onecad_interaction::Action;

pub fn route(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    if app.sampled {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = done.clone();
        app.gpu.queue.on_submitted_work_done(move || {
            finished.store(true, std::sync::atomic::Ordering::Release)
        });
        app.sampling_done = done;
        app.sampled = false;
    }
    if app.cad_needs_fence {
        let context = ctx.clone();
        if let Err(error) = app.renderer.after_submit(move || context.request_repaint()) {
            app.status = error.to_string();
        }
        app.cad_needs_fence = false;
    }
    let actions = app.menu.as_ref().map_or_else(Vec::new, |menu| menu.drain());
    menu::remove_duplicate_events(&actions, &mut raw.events);
    let text = app
        .numeric_id
        .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
    for action in actions {
        route_menu(app, ctx, &mut raw.events, action, text);
    }
    let mut commands = Vec::new();
    raw.events.retain(|event| {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
            && modifiers.command
        {
            match key {
                egui::Key::Z if !text => {
                    commands.push(if modifiers.shift {
                        Command::Redo
                    } else {
                        Command::Undo
                    });
                    return false;
                }
                egui::Key::S => {
                    commands.push(Command::Save);
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
    for command in commands {
        app.submit(command);
    }
    if !text && app.sketch_active() {
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
        app.dispatch(Action::Cancel);
    }
    protect_composition(app, raw, text);
}

fn route_menu(
    app: &mut App,
    _ctx: &egui::Context,
    events: &mut Vec<egui::Event>,
    action: menu::Action,
    text: bool,
) {
    match action {
        menu::Action::Undo | menu::Action::Redo if text => {
            menu::push_command(events, egui::Key::Z, action == menu::Action::Redo)
        }
        menu::Action::Undo => app.submit(Command::Undo),
        menu::Action::Redo => app.submit(Command::Redo),
        menu::Action::Save => app.submit(Command::Save),
        menu::Action::Quit => app.close_prompt = true,
        menu::Action::Fit => app.fit_pending = true,
        menu::Action::Home => {
            app.camera = onecad_scene::Camera::default();
            app.fit_pending = true;
        }
        menu::Action::Cut if text => events.push(egui::Event::Cut),
        menu::Action::Copy if text => events.push(egui::Event::Copy),
        menu::Action::SelectAll if text => menu::push_command(events, egui::Key::A, false),
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

fn protect_composition(app: &mut App, raw: &mut egui::RawInput, text: bool) {
    if !text || !raw.focused {
        app.composing = false;
        return;
    }
    let mut protected = app.composing;
    for event in &raw.events {
        match event {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                app.composing = !text.is_empty();
                protected = true;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                app.composing = false;
                protected = true;
            }
            #[allow(deprecated)]
            egui::Event::Ime(egui::ImeEvent::Disabled) => {
                protected |= app.composing;
                app.composing = false;
            }
            _ => {}
        }
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
