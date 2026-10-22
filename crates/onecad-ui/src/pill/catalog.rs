//! Overflow uses the primary pill's applicability and dispatch.
use super::{ToolDef, ToolId, view};
use crate::{app::App, layout::Chrome};

pub(super) fn show(
    app: &App,
    chrome: &Chrome,
    button: Option<&egui::Response>,
    running: Option<ToolId>,
    drawn: &[ToolId],
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
                    for def in overflow(current, catalog, drawn) {
                        let availability = view(app, def, running);
                        let response = row(ui, def, availability.enabled);
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

/// A compact pill moves its other tools here, so a row carries the same
/// automation id its pill button would; `overflow` skips drawn tools, so an id
/// never repeats.
pub(super) fn row(ui: &mut egui::Ui, def: &ToolDef, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(def.label));
    crate::automation::name(ui.ctx(), response.id, &crate::automation::tool(def.id));
    response
}

pub(super) fn overflow<'a>(
    current: &[&'a [ToolDef]],
    catalog: &'a [ToolDef],
    drawn: &[ToolId],
) -> Vec<&'a ToolDef> {
    let mut shown: Vec<&ToolDef> = Vec::new();
    for def in current
        .iter()
        .flat_map(|group| group.iter())
        .chain(catalog)
        .filter(|def| !matches!(def.id, ToolId::Select | ToolId::More))
    {
        if drawn.contains(&def.id) || shown.iter().any(|s| s.id == def.id) {
            continue;
        }
        shown.push(def);
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: ToolId) -> ToolDef {
        ToolDef {
            id,
            icon: "",
            label: "",
            shortcut: None,
            source: None,
        }
    }

    #[test]
    fn overflow_skips_drawn_and_dedupes() {
        let group = [
            def(ToolId::Select),
            def(ToolId::Constraints),
            def(ToolId::More),
        ];
        let catalog = [
            def(ToolId::Constraints),
            def(ToolId::Section),
            def(ToolId::CheckGeometry),
        ];
        let ids = |drawn: &[ToolId]| -> Vec<ToolId> {
            overflow(&[&group[..]], &catalog, drawn)
                .iter()
                .map(|d| d.id)
                .collect()
        };
        assert_eq!(
            ids(&[]),
            [ToolId::Constraints, ToolId::Section, ToolId::CheckGeometry]
        );
        assert_eq!(
            ids(&[ToolId::Select, ToolId::Constraints, ToolId::More]),
            [ToolId::Section, ToolId::CheckGeometry]
        );
    }
}
