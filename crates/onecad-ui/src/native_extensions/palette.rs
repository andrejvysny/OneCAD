use super::command_available;
use crate::{
    app::App,
    tool_registry::{PLANNED_MODEL, PLANNED_SKETCH, SelectionKind, ToolDef, ToolId, pill_tools},
};

#[derive(Clone)]
enum Command {
    Core(ToolId),
    Extension(String),
}

fn core_commands() -> Vec<ToolDef> {
    let mut commands = Vec::new();
    for kind in [
        SelectionKind::None,
        SelectionKind::Face,
        SelectionKind::Edge,
        SelectionKind::Body,
        SelectionKind::Region,
        SelectionKind::SketchIdle,
        SelectionKind::SketchEntity,
        SelectionKind::Feature,
    ] {
        for tool in pill_tools(kind)
            .iter()
            .flat_map(|group| group.iter())
            .chain(PLANNED_MODEL)
            .chain(PLANNED_SKETCH)
        {
            if tool.id != ToolId::More
                && !commands.iter().any(|entry: &ToolDef| entry.id == tool.id)
            {
                commands.push(*tool);
            }
        }
    }
    commands
}

pub(crate) fn route_shortcut(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !raw.focused
        || app.composing
        || crate::input::text_owns_keyboard(ctx)
        || crate::input::overlay_owns_keyboard(app, ctx)
    {
        return;
    }
    if raw.events.iter().any(shortcut) {
        app.ui.extensions.native.open_palette();
        raw.events.retain(|event| !shortcut(event));
        ctx.request_repaint();
    }
}

fn shortcut(event: &egui::Event) -> bool {
    matches!(event,
        egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
        if modifiers.command && !modifiers.alt && ((*key == egui::Key::K && !modifiers.shift) || (*key == egui::Key::P && modifiers.shift)))
}

fn matches_query(title: &str, description: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    title.to_lowercase().contains(&query) || description.to_lowercase().contains(&query)
}

pub(super) fn show(app: &mut App, ctx: &egui::Context) {
    if !app.ui.extensions.native.palette_open {
        return;
    }
    let document = crate::bars::export_allowed(app);
    let selection = app
        .doc
        .selection
        .items()
        .iter()
        .any(|item| item.body().is_some());
    let idle = !app.busy() && !app.ui.extensions.native.pending;
    let core: Vec<_> = core_commands()
        .into_iter()
        .map(|tool| {
            let reason = crate::pill::disabled_reason(app, &tool);
            (tool, reason)
        })
        .collect();
    let state = &mut app.ui.extensions.native;
    let mut picked = None;
    let mut first_available = None;
    let mut close = false;
    egui::Modal::new(egui::Id::unique("extension-command-palette")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Search commands");
        let search = ui.add(
            egui::TextEdit::singleline(&mut state.palette_query)
                .hint_text("Search commands")
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut state.palette_focus) {
            search.request_focus();
        }
        let mut count = 0;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                for (tool, reason) in &core {
                    if !matches_query(tool.label, "", &state.palette_query) {
                        continue;
                    }
                    count += 1;
                    let enabled = reason.is_none();
                    if enabled && first_available.is_none() {
                        first_available = Some(Command::Core(tool.id));
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new(tool.label))
                        .clicked()
                    {
                        picked = Some(Command::Core(tool.id));
                    }
                    if let Some(reason) = reason {
                        ui.small(*reason);
                    }
                }
                for entry in state.commands.iter().filter(|entry| {
                    entry
                        .command
                        .locations
                        .iter()
                        .any(|location| location == "palette")
                }) {
                    if !matches_query(
                        &entry.command.title,
                        &entry.command.description,
                        &state.palette_query,
                    ) {
                        continue;
                    }
                    count += 1;
                    let enabled = idle
                        && entry.enabled
                        && command_available(&entry.command, document, selection);
                    if enabled && first_available.is_none() {
                        first_available = Some(Command::Extension(entry.command.id.clone()));
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new(&entry.command.title))
                        .on_disabled_hover_text(
                            "Enable the extension and provide its required document or selection",
                        )
                        .clicked()
                    {
                        picked = Some(Command::Extension(entry.command.id.clone()));
                    }
                    if !entry.command.description.is_empty() {
                        ui.small(&entry.command.description);
                    }
                }
            });
        if count == 0 {
            ui.label("No matching commands.");
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            picked = first_available.take();
        }
        ui.small("Enter runs the first available match · Esc closes");
        if ui.button("Close").clicked() || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if close || picked.is_some() {
        state.palette_open = false;
    }
    match picked {
        Some(Command::Core(id)) => crate::pill::activate(app, ctx, id),
        Some(Command::Extension(command)) => app.run_extension_command(&command),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_finds_core_modeling_and_sketch_commands_once() {
        let commands = core_commands();
        for (query, expected) in [
            ("extrude", ToolId::Extrude),
            ("fillet", ToolId::Fillet),
            ("rectangle", ToolId::Rectangle),
            ("gear", ToolId::Generators),
        ] {
            assert_eq!(
                commands.iter().filter(|tool| tool.id == expected).count(),
                1
            );
            assert!(
                commands
                    .iter()
                    .any(|tool| tool.id == expected && matches_query(tool.label, "", query))
            );
        }
        assert!(!commands.iter().any(|tool| tool.id == ToolId::More));
        assert!(commands.iter().any(|tool| tool.id == ToolId::Sweep));
    }

    #[test]
    fn palette_search_matches_title_and_description_without_case() {
        assert!(matches_query(
            "Send to Bambu Studio",
            "Export selected bodies",
            " BAMBU "
        ));
        assert!(matches_query("Send", "Export selected bodies", "selected"));
        assert!(!matches_query(
            "Send",
            "Export selected bodies",
            "unrelated"
        ));
    }

    #[test]
    fn palette_shortcut_matches_the_title_bar_hint_and_legacy_binding() {
        let event = |key, shift| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                shift,
                ..Default::default()
            },
        };
        assert!(shortcut(&event(egui::Key::K, false)));
        assert!(shortcut(&event(egui::Key::P, true)));
        assert!(!shortcut(&event(egui::Key::P, false)));
        assert!(!shortcut(&event(egui::Key::K, true)));
    }
}
