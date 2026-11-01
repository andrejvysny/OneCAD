//! Unmodified single-key tool shortcuts, scoped by model or sketch mode. Keys
//! resolve through `tool_registry::shortcut_tool` and dispatch like a pill click.
use crate::app::App;
use crate::shortcut_prefs::ShortcutOverride;
use crate::tool_registry::{ToolId, shortcut_tool};

/// Remove each plain key press that maps to an available tool, in event order.
fn take(
    events: &mut Vec<egui::Event>,
    overrides: &[ShortcutOverride],
    sketch: bool,
    available: impl Fn(ToolId) -> bool,
) -> Vec<ToolId> {
    let mut tools = Vec::new();
    events.retain(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = event
        else {
            return true;
        };
        match shortcut_tool(overrides, *key, sketch)
            .filter(|id| modifiers.is_none() && available(*id))
        {
            Some(id) => {
                tools.push(id);
                false
            }
            None => true,
        }
    });
    tools
}

/// Home and Settings cover the document: its tool hotkeys must neither fire into
/// it nor steal a key the Settings page is recording.
fn blocked(start_open: bool, other: bool) -> bool {
    start_open || other
}

pub(super) fn route(
    app: &mut App,
    ctx: &egui::Context,
    raw: &mut egui::RawInput,
    text: bool,
    composition: bool,
) {
    let pointer = ctx.input(|i| i.pointer.any_down());
    let running = crate::tool_registry::running_tool(app).is_some();
    if blocked(
        app.ui.start_open,
        text || composition || pointer || !raw.focused || running,
    ) {
        return;
    }
    let sketch = app.sketch_active();
    let available = |id| {
        let def = crate::tool_registry::ToolDef {
            id,
            icon: "select",
            label: "",
            source: None,
        };
        crate::pill::available(app, &def)
    };
    let tools = take(
        &mut raw.events,
        &app.ui.shortcut_overrides,
        sketch,
        available,
    );
    for id in tools {
        crate::pill::activate(app, ctx, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }

    fn run(events: Vec<egui::Event>, sketch: bool, ok: bool) -> (Vec<ToolId>, usize) {
        run_with(&[], events, sketch, ok)
    }

    fn run_with(
        overrides: &[ShortcutOverride],
        events: Vec<egui::Event>,
        sketch: bool,
        ok: bool,
    ) -> (Vec<ToolId>, usize) {
        let mut events = events;
        let tools = take(&mut events, overrides, sketch, |_| ok);
        (tools, events.len())
    }

    #[test]
    fn tool_keys_are_inert_while_home_is_open() {
        assert!(blocked(true, false));
        assert!(!blocked(false, false));
        assert!(blocked(false, true));
    }

    #[test]
    fn plain_keys_dispatch_by_mode_and_are_consumed() {
        let n = egui::Modifiers::NONE;
        let e = press(egui::Key::E, n, false);
        assert_eq!(
            run(vec![e.clone()], false, true),
            (vec![ToolId::Extrude], 0)
        );
        assert_eq!(run(vec![e], true, true), (vec![], 1));
        let l = press(egui::Key::L, n, false);
        assert_eq!(run(vec![l.clone()], true, true), (vec![ToolId::Line], 0));
        assert_eq!(run(vec![l], false, true), (vec![], 1));
    }

    #[test]
    fn modifiers_repeat_and_unavailable_tools_do_nothing() {
        for m in [
            egui::Modifiers::SHIFT,
            egui::Modifiers::ALT,
            egui::Modifiers::CTRL,
            egui::Modifiers::COMMAND,
            egui::Modifiers::MAC_CMD,
        ] {
            assert_eq!(
                run(vec![press(egui::Key::E, m, false)], false, true),
                (vec![], 1)
            );
        }
        let n = egui::Modifiers::NONE;
        assert_eq!(
            run(vec![press(egui::Key::E, n, true)], false, true),
            (vec![], 1)
        );
        assert_eq!(
            run(vec![press(egui::Key::E, n, false)], false, false),
            (vec![], 1)
        );
    }

    #[test]
    fn remapped_key_dispatches_and_old_key_does_not() {
        use crate::shortcut_prefs::{KeyContext, assign};
        let n = egui::Modifiers::NONE;
        let mut overrides = Vec::new();
        assign(
            &mut overrides,
            KeyContext::Modeling,
            ToolId::Extrude,
            egui::Key::Q,
        );
        let q = press(egui::Key::Q, n, false);
        let e = press(egui::Key::E, n, false);
        assert_eq!(
            run_with(&overrides, vec![q], false, true),
            (vec![ToolId::Extrude], 0)
        );
        assert_eq!(run_with(&overrides, vec![e], false, true), (vec![], 1));
        // Sketch mode is independent of the modeling remap.
        let l = press(egui::Key::L, n, false);
        assert_eq!(
            run_with(&overrides, vec![l], true, true),
            (vec![ToolId::Line], 0)
        );
    }
}
