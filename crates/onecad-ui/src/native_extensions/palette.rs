use super::command_available;
use crate::app::App;

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
    let idle = !app.busy();
    let state = &mut app.ui.extensions.native;
    let mut picked = None;
    let mut first_available = None;
    let mut close = false;
    egui::Modal::new(egui::Id::unique("extension-command-palette")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Extension commands");
        let search = ui.add(
            egui::TextEdit::singleline(&mut state.palette_query)
                .hint_text("Search extension commands")
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut state.palette_focus) {
            search.request_focus();
        }
        let mut count = 0;
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
            let enabled =
                idle && entry.enabled && command_available(&entry.command, document, selection);
            if enabled && first_available.is_none() {
                first_available = Some(entry.command.id.clone());
            }
            if ui
                .add_enabled(enabled, egui::Button::new(&entry.command.title))
                .on_disabled_hover_text(
                    "Enable the extension and provide its required document or selection",
                )
                .clicked()
            {
                picked = Some(entry.command.id.clone());
            }
            if !entry.command.description.is_empty() {
                ui.small(&entry.command.description);
            }
        }
        if count == 0 {
            ui.label(
                "No matching extension commands. Install or enable an extension in Extensions.",
            );
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
    if let Some(command) = picked {
        app.run_extension_command(&command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
