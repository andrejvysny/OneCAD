//! Overflow uses the primary pill's applicability and dispatch.
use super::{ToolId, view};
use crate::{app::App, layout::Chrome};

pub(super) fn show(
    app: &App,
    chrome: &Chrome,
    button: Option<&egui::Response>,
    running: Option<ToolId>,
) -> Option<ToolId> {
    let mut clicked = None;
    if let Some(button) = button {
        egui::Popup::menu(button).width(280.0).show(|ui| {
            let catalog = if app.sketch_active() {
                crate::tool_registry::PLANNED_SKETCH
            } else {
                crate::tool_registry::PLANNED_MODEL
            };
            egui::ScrollArea::vertical()
                .max_height((chrome.viewport.height() - 110.0).max(80.0))
                .show(ui, |ui| {
                    let current =
                        crate::tool_registry::pill_tools(crate::tool_registry::selection_kind(app));
                    let mut shown = Vec::new();
                    for def in current
                        .iter()
                        .flat_map(|group| group.iter())
                        .chain(catalog)
                        .filter(|def| !matches!(def.id, ToolId::Select | ToolId::More))
                    {
                        if shown.contains(&def.id) {
                            continue;
                        }
                        shown.push(def.id);
                        let availability = view(app, def, running);
                        let response =
                            ui.add_enabled(availability.enabled, egui::Button::new(def.label));
                        if response.clicked() {
                            clicked = Some(def.id);
                            ui.close();
                        }
                        if let Some(reason) = availability.reason {
                            response.on_disabled_hover_text(reason);
                        }
                    }
                });
        });
    }
    clicked
}
