//! Pencil accessory actions (IP19, `docs/ios/INPUT.md`): the adaptive tool
//! palette and the switch back to the previous sketch tool. Both only choose a
//! tool through the pill's own availability and dispatch, so the pill is their
//! visible control equivalent; neither ever authors geometry.
use crate::{
    app::App,
    tool_registry::{ToolId, pill_tools, selection_kind},
};
use onecad_interaction::sketch::SketchAction;
use onecad_widgets::components::lists::{MenuRow, menu_frame};

const WIDTH: f32 = 200.0;

pub(crate) fn open(app: &mut App, at: egui::Pos2) {
    if at.is_finite() {
        app.ui.tool_palette = Some(at);
    }
}

/// Switches to the sketch tool used before the current one; `false` when there
/// is no sketch, no other previous tool, or the sketch tools are unavailable.
pub(crate) fn toggle_previous(app: &mut App) -> bool {
    let Some(previous) = app.ui.previous_sketch_tool else {
        return false;
    };
    let current = app.doc.sketch.session.as_ref().map(|session| session.tool);
    if !app.sketch_active() || current == Some(previous) || !crate::sketch_ui::tools_enabled(app) {
        return false;
    }
    app.sketch_dispatch(SketchAction::SetTool(previous));
    app.doc.sketch.session.as_ref().map(|session| session.tool) == Some(previous)
}

/// The tools the pill offers right now and would accept: the palette adapts to
/// the sketch or model mode and the selection, like the pill.
fn tools(app: &App) -> Vec<crate::tool_registry::ToolDef> {
    pill_tools(selection_kind(app))
        .iter()
        .flat_map(|group| group.iter())
        .filter(|def| !matches!(def.id, ToolId::More | ToolId::Section))
        .filter(|def| crate::pill::available(app, def))
        .copied()
        .collect()
}

pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(at) = app.ui.tool_palette else {
        return;
    };
    let tools = tools(app);
    // Escape is the palette's as an input overlay owner (`input::overlay`).
    if tools.is_empty() {
        app.ui.tool_palette = None;
        return;
    }
    let mut chosen = None;
    let area = egui::Area::new(egui::Id::unique("onecad-tool-palette"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(true)
        .show(ctx, |ui| {
            menu_frame(ctx).show(ui, |ui| {
                ui.set_width(WIDTH);
                for def in &tools {
                    if ui.add(MenuRow::new(Some(def.icon), def.label)).clicked() {
                        chosen = Some(def.id);
                    }
                }
            });
        });
    ctx.accesskit_node_builder(area.response.id, |node| {
        node.set_author_id("onecad.palette")
    });
    let outside = ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|position| !area.response.rect.contains(position))
    });
    if let Some(id) = chosen {
        app.ui.tool_palette = None;
        crate::pill::activate(app, ctx, id);
    } else if outside {
        app.ui.tool_palette = None;
    }
}
