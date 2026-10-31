//! Promotes every enabled catalog command into the pill, subject to a width budget.
use super::layout::required_width;
use crate::{
    app::App,
    tool_registry::{PLANNED_MODEL, PLANNED_SKETCH, ToolDef, ToolId},
};

/// Final groups: kind primaries, promoted commands, then the group holding More.
pub(super) fn groups(
    app: &App,
    base: &[&[ToolDef]],
    room: f32,
    selection: bool,
) -> Vec<Vec<ToolDef>> {
    let catalog = if app.sketch_active() {
        PLANNED_SKETCH
    } else {
        PLANNED_MODEL
    };
    let enabled: Vec<ToolDef> = catalog
        .iter()
        .filter(|def| super::available(app, def))
        .copied()
        .collect();
    compose(base, &enabled, room, selection)
}

/// Pure composition: `enabled` is the available catalog in registry order.
pub(super) fn compose(
    base: &[&[ToolDef]],
    enabled: &[ToolDef],
    room: f32,
    selection: bool,
) -> Vec<Vec<ToolDef>> {
    let mut out: Vec<Vec<ToolDef>> = base.iter().map(|group| group.to_vec()).collect();
    let mut promoted: Vec<ToolDef> = Vec::new();
    for def in enabled {
        let drawn = out.iter().flatten().any(|d| d.id == def.id);
        if matches!(def.id, ToolId::Select | ToolId::More) || drawn {
            continue;
        }
        let sketch = out.iter_mut().find_map(|g| {
            g.iter()
                .position(|d| d.id == ToolId::Sketch)
                .map(|i| (g, i))
        });
        match sketch {
            Some((group, at)) if def.id == ToolId::DatumPlane => group.insert(at + 1, *def),
            _ if promoted.iter().any(|d| d.id == def.id) => {}
            _ => promoted.push(*def),
        }
    }
    let at = out
        .iter()
        .position(|g| g.iter().any(|d| d.id == ToolId::More))
        .unwrap_or(out.len());
    while !promoted.is_empty() && !fits(&out, promoted.len(), room, selection) {
        promoted.pop();
    }
    if !promoted.is_empty() {
        out.insert(at, promoted);
    }
    out
}

fn fits(out: &[Vec<ToolDef>], promoted: usize, room: f32, selection: bool) -> bool {
    let tools = out.iter().map(Vec::len).sum::<usize>() + promoted;
    required_width(tools, out.len() + 1, selection) <= room
}

#[cfg(test)]
mod tests;
