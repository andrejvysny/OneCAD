//! Shortcuts and the ⌃Tab switcher in one pass, in event order: a same-frame Save,
//! Undo or New reaches the document that is active at that event's position, even
//! when an earlier event in the frame switched documents.
use super::{TabShortcut, document_shortcut, history_shortcut, submit_history, tab_shortcut};
use crate::app::App;

/// `shortcuts` is false while an overlay owns the keyboard; the switcher still runs.
pub(crate) fn route(
    app: &mut App,
    raw: &mut egui::RawInput,
    shortcuts: bool,
    text: bool,
    composition: bool,
) {
    let focused = raw.focused;
    // Losing focus mid-switch must not leave the switcher waiting for a release
    // that will never be delivered to this window, nor commit on that release.
    if !focused {
        app.switcher = None;
    }
    let shown = |app: &App| (app.doc.tab, app.ui.start_open);
    let mut kept = Vec::with_capacity(raw.events.len());
    let mut switched_at = None;
    for event in std::mem::take(&mut raw.events) {
        let before = shown(app);
        let consumed = crate::tab_switcher::event(app, &event)
            || shortcuts && shortcut(app, &event, text, composition, focused);
        if shown(app) != before {
            switched_at = Some(kept.len());
        }
        if !consumed {
            kept.push(event);
        }
    }
    if !focused {
        app.switcher = None;
    }
    raw.events = match switched_at {
        Some(at) => after_switch(kept, at),
        None => kept,
    };
}

/// Executes one shortcut against the active document. Returns whether it was one.
fn shortcut(
    app: &mut App,
    event: &egui::Event,
    text: bool,
    composition: bool,
    focused: bool,
) -> bool {
    if let Some(command) = history_shortcut(event, text || composition, focused) {
        submit_history(app, command);
        return true;
    }
    let egui::Event::Key {
        key,
        pressed: true,
        modifiers,
        repeat,
        ..
    } = event
    else {
        return false;
    };
    if !modifiers.command || composition {
        return false;
    }
    if focused && let Some(request) = document_shortcut(*key, *modifiers) {
        app.document_request(request);
        return true;
    }
    if focused && let Some(action) = tab_shortcut(*key, *modifiers) {
        // A held Cmd+W must close one document, not every clean one.
        if !repeat {
            match action {
                TabShortcut::Close => app.request_close_tab(),
                TabShortcut::Cycle(step) => app.cycle_tab(step),
            }
        }
        return true;
    }
    if *key == egui::Key::Q {
        app.close_prompt = true;
        return true;
    }
    false
}

/// The rest of the frame is applied to the document shown after the last switch
/// (`at` events were kept before it). Input that preceded the switch belonged to
/// a document that is no longer shown and is dropped; pointer input after it is
/// dropped too, so no document receives half a gesture. Releases, modifier state
/// and focus still pass, so egui never keeps a button or modifier held.
fn after_switch(events: Vec<egui::Event>, at: usize) -> Vec<egui::Event> {
    events
        .into_iter()
        .enumerate()
        .filter(|(index, event)| {
            releases_state(event) || *index >= at && !starts_pointer_input(event)
        })
        .map(|(_, event)| event)
        .collect()
}

fn releases_state(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::ModifiersChanged(_)
            | egui::Event::PointerButton { pressed: false, .. }
            | egui::Event::PointerGone
            | egui::Event::WindowFocused(_)
            | egui::Event::Key { pressed: false, .. }
    )
}

fn starts_pointer_input(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::PointerMoved(_)
            | egui::Event::MouseMoved(_)
            | egui::Event::PointerButton { pressed: true, .. }
            | egui::Event::MouseWheel { .. }
            | egui::Event::Zoom(_)
            | egui::Event::Rotate(_)
            | egui::Event::Touch { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn button(pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::pos2(10.0, 10.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_switch_drops_earlier_input_and_later_pointer_input_but_keeps_releases() {
        let moved = egui::Event::PointerMoved(egui::pos2(1.0, 1.0));
        let events = vec![
            key(egui::Key::Delete, true),
            moved.clone(),
            button(true),
            key(egui::Key::Delete, false),
            // The switch happened here.
            moved,
            button(true),
            key(egui::Key::Enter, true),
            button(false),
            egui::Event::PointerGone,
        ];
        assert_eq!(
            after_switch(events, 4),
            vec![
                key(egui::Key::Delete, false),
                key(egui::Key::Enter, true),
                button(false),
                egui::Event::PointerGone,
            ]
        );
    }
}
