use crate::layout::{CUBE, Chrome};
use egui::pos2;
use onecad_widgets::components::actions::{PillAccent, PillMode, ToolPill};

pub(super) fn toolbar_bounds(chrome: &Chrome) -> egui::Rect {
    let left = chrome.toolbar_left;
    let mut right = chrome.view_cluster.x - CUBE - 10.0;
    let top = chrome.viewport.top() + 10.0;
    if chrome.view_controls.top() < top + 48.0 {
        right = right.min(chrome.view_controls.left() - 10.0);
    }
    egui::Rect::from_min_size(pos2(left, top), egui::vec2((right - left).max(120.0), 48.0))
}

pub(super) fn toolbar_rect(ctx: &egui::Context, chrome: &Chrome) -> egui::Rect {
    let bounds = toolbar_bounds(chrome);
    let width = ctx
        .data(|data| data.get_temp::<f32>(egui::Id::unique("toolbar-width")))
        .unwrap_or(bounds.width())
        .min(bounds.width());
    let left = (chrome.pill_centre_x - width / 2.0).clamp(bounds.left(), bounds.right() - width);
    egui::Rect::from_min_size(pos2(left, bounds.top()), egui::vec2(width, bounds.height()))
}

pub(super) fn bounded_pill<R>(
    ui: &mut egui::Ui,
    width: f32,
    offset: Option<f32>,
    mode: PillMode,
    accent: PillAccent,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.set_max_width(width);
    let mut scroll = egui::ScrollArea::horizontal()
        .id_salt("tool-pill-scroll")
        .max_width(width)
        .auto_shrink([true, true]);
    if let Some(offset) = offset {
        scroll = scroll.horizontal_scroll_offset(offset);
    }
    let output = scroll.show(ui, |ui| {
        ToolPill::new()
            .mode(mode)
            .accent(accent)
            .show(ui, add)
            .inner
    });
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::unique("toolbar-width"), output.content_size.x)
    });
    output
}

#[cfg(test)]
mod tests;

/// Reserve a fixed overflow button instead of hiding commands in a horizontal scroller.
pub(super) fn compact_tools(
    groups: &[&[crate::tool_registry::ToolDef]],
    active: Option<crate::tool_registry::ToolId>,
    width: f32,
    selection: bool,
) -> Option<Vec<crate::tool_registry::ToolDef>> {
    use crate::tool_registry::ToolId;
    let tools: Vec<_> = groups
        .iter()
        .flat_map(|group| group.iter().copied())
        .collect();
    let required = tools.len() as f32 * 36.0
        + groups.len() as f32 * 14.0
        + if selection { 180.0 } else { 8.0 };
    if required <= width || tools.len() <= 1 {
        return None;
    }
    let mut shown = Vec::new();
    if let Some(select) = tools.iter().find(|tool| tool.id == ToolId::Select) {
        shown.push(*select);
    }
    let preferred = active.filter(|id| *id != ToolId::Select).or_else(|| {
        tools
            .iter()
            .find(|tool| !matches!(tool.id, ToolId::Select | ToolId::More))
            .map(|tool| tool.id)
    });
    if let Some(tool) = tools.iter().find(|tool| Some(tool.id) == preferred) {
        shown.push(*tool);
    }
    if width >= 158.0
        && let Some(point) = tools.iter().find(|tool| tool.id == ToolId::Point)
        && !shown.iter().any(|tool| tool.id == ToolId::Point)
    {
        shown.push(*point);
    }
    if let Some(more) = tools.iter().find(|tool| tool.id == ToolId::More) {
        shown.push(*more);
    }
    Some(shown)
}
