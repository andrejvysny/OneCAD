//! ToolPill host: filters tools by selection kind and dispatches the same calls the old
//! toolbar made. Tools without a backing action render disabled.
use crate::{
    app::App,
    layout::Chrome,
    sketch_ui,
    tool_registry::{SelectionKind, ToolDef, ToolId, pill_tools, running_tool, selection_kind},
};
use egui::{Align2, Context, Order};
use onecad_interaction::{
    Action,
    sketch::{SketchAction, SketchTool},
};
use onecad_widgets::{
    components::{
        actions::{
            PillAccent, PillMode, SelectionLabel, SelectionVariant, ToolButton, ToolState,
            pill_divider,
        },
        tooltip::Tip,
    },
    frame,
};

mod layout;
use layout::{bounded_pill, toolbar_rect};

pub(crate) fn center_x(ctx: &Context, chrome: &Chrome) -> f32 {
    toolbar_rect(ctx, chrome).center().x
}

type Groups<'a> = &'a [&'a [ToolDef]];

const NOT_YET: &str = "Not available yet";

#[derive(Clone, Copy)]
struct View {
    state: ToolState,
    enabled: bool,
    reason: Option<&'static str>,
}

fn gate(ok: bool, reason: &'static str) -> View {
    View {
        state: ToolState::Default,
        enabled: ok,
        reason: (!ok).then_some(reason),
    }
}

fn model_view(app: &App, eligible: bool, reason: &'static str) -> View {
    if app.interaction.tool.is_some() || app.busy() {
        return gate(false, "Finish the current operation first");
    }
    if !app.viewport_ready() {
        return gate(false, "Viewport unavailable");
    }
    if !app.bridge.valid() {
        return gate(false, "Worker unavailable");
    }
    if crate::viewport_selection::sectioned(app) {
        return gate(false, "Turn off section view first");
    }
    gate(eligible, reason)
}

fn sketch_view(app: &App, tool: SketchTool) -> View {
    let selected = app.sketch.session.as_ref().is_some_and(|s| s.tool == tool);
    let mut view = gate(sketch_ui::tools_enabled(app), "Sketch is not ready");
    if selected {
        view.state = ToolState::SketchActive;
    }
    view
}

fn arc_tool(app: &App) -> SketchTool {
    if app.ui.arc_three_point {
        SketchTool::ThreePointArc
    } else {
        SketchTool::Arc
    }
}

fn view(app: &App, def: &ToolDef, running: Option<ToolId>) -> View {
    if running == Some(def.id) {
        return View {
            state: ToolState::Running,
            enabled: true,
            reason: None,
        };
    }
    match def.id {
        ToolId::Select if app.sketch_active() => sketch_view(app, SketchTool::Select),
        ToolId::Select => View {
            state: ToolState::Active,
            enabled: true,
            reason: None,
        },
        ToolId::Line => sketch_view(app, SketchTool::Line),
        ToolId::Rectangle => sketch_view(
            app,
            if app.ui.rectangle_centered {
                SketchTool::CenterRectangle
            } else {
                SketchTool::Rectangle
            },
        ),
        ToolId::Point => sketch_view(app, SketchTool::Point),
        ToolId::Circle => sketch_view(app, SketchTool::Circle),
        ToolId::Arc => sketch_view(app, arc_tool(app)),
        ToolId::Slot => sketch_view(app, SketchTool::Slot),
        ToolId::Sketch => model_view(app, true, "Finish the current operation first"),
        ToolId::Extrude => model_view(
            app,
            app.interaction.selection.is_some(),
            "Select a closed sketch profile first",
        ),
        ToolId::Edit => model_view(
            app,
            app.selected_feature.is_some_and(|id| {
                app.display
                    .as_ref()
                    .and_then(|display| {
                        display.records.iter().find(|record| record.record_id == id)
                    })
                    .is_some_and(|record| {
                        !onecad_interaction::numeric_parameters(&record.op).is_empty()
                    })
            }),
            "Select an editable feature first",
        ),
        ToolId::MoveRotate => model_view(app, app.selected_body.is_some(), "Select a body first"),
        ToolId::Section => gate(app.viewport_ready(), "Viewport unavailable"),
        ToolId::Horizontal
        | ToolId::Vertical
        | ToolId::Parallel
        | ToolId::Perpendicular
        | ToolId::Equal
        | ToolId::Midpoint
        | ToolId::Fix
        | ToolId::Distance
        | ToolId::Diameter
        | ToolId::Dimension => gate(
            crate::constraint_authoring::available(app, def.id),
            "Choose applicable operands or edit the existing relation; see Constraints for details",
        ),
        ToolId::Construction | ToolId::Delete if app.sketch_active() => {
            let allowed = crate::sketch_repair::availability(app, def.id == ToolId::Delete);
            let mut state = gate(allowed.is_ok(), allowed.err().unwrap_or(""));
            if def.id == ToolId::Construction
                && app
                    .sketch
                    .session
                    .as_ref()
                    .is_some_and(|session| session.construction)
            {
                state.state = ToolState::SketchActive;
            }
            state
        }
        ToolId::InsertComponent | ToolId::MakeComponent => gate(true, NOT_YET),
        ToolId::Flange | ToolId::Dogbone => extension_view(app, def.id),
        ToolId::CheckGeometry if app.ui.editor_facade.topology_failed => {
            gate(false, "Topology Check stopped responding")
        }
        ToolId::CheckGeometry => gate(true, NOT_YET),
        ToolId::Constraints => {
            let mut v = gate(sketch_ui::tools_enabled(app), "Sketch is not ready");
            if app.ui.constraints_open {
                v.state = ToolState::Active;
            }
            v
        }
        _ => gate(false, NOT_YET),
    }
}

fn extension_view(app: &App, id: ToolId) -> View {
    let mut view = gate(true, NOT_YET);
    if app.ui.editor_facade.active_tool == Some(id) {
        view.state = ToolState::Active;
    }
    view
}

/// Registry shortcuts are design data; show one only once the app binds it (none yet).
fn bound_shortcut(def: &ToolDef) -> Option<&'static str> {
    const BOUND: &[ToolId] = &[];
    def.shortcut.filter(|_| BOUND.contains(&def.id))
}

pub(crate) fn activate(app: &mut App, ctx: &Context, id: ToolId) {
    let sketch_tool = match id {
        ToolId::Select if app.sketch_active() => Some(SketchTool::Select),
        ToolId::Line => Some(SketchTool::Line),
        ToolId::Rectangle => Some(if app.ui.rectangle_centered {
            SketchTool::CenterRectangle
        } else {
            SketchTool::Rectangle
        }),
        ToolId::Point => Some(SketchTool::Point),
        ToolId::Circle => Some(SketchTool::Circle),
        ToolId::Arc => Some(arc_tool(app)),
        ToolId::Slot => Some(SketchTool::Slot),
        _ => None,
    };
    if let Some(tool) = sketch_tool {
        app.sketch_dispatch(SketchAction::SetTool(tool));
        return;
    }
    match id {
        ToolId::Construction => crate::sketch_repair::toggle_construction(app),
        ToolId::Delete if app.sketch_active() => crate::sketch_repair::delete_selected(app),
        ToolId::Sketch => app.choose_sketch_support(),
        ToolId::Extrude => {
            app.gesture += 1;
            let step = app
                .display
                .as_ref()
                .map_or(0, |display| display.projection.applied_ops as u32);
            app.dispatch(Action::BeginExtrude {
                record_id: onecad_core::ids::RecordId::new(),
                gesture: app.gesture,
                step_index: step,
            });
        }
        ToolId::Edit => crate::inspector_history::edit_selected_feature(app),
        ToolId::MoveRotate => app.begin_transform(),
        ToolId::InsertComponent => crate::editor_facade::show_library(app),
        ToolId::MakeComponent => crate::component_wizard::open(&mut app.ui),
        ToolId::Flange | ToolId::Dogbone | ToolId::CheckGeometry => {
            app.ui.editor_facade.press_extension_tool(id);
        }
        ToolId::Constraints => crate::constraints_menu::toggle(&mut app.ui),
        ToolId::Horizontal
        | ToolId::Vertical
        | ToolId::Parallel
        | ToolId::Perpendicular
        | ToolId::Equal
        | ToolId::Midpoint
        | ToolId::Fix
        | ToolId::Distance
        | ToolId::Diameter
        | ToolId::Dimension => crate::constraint_authoring::activate(app, ctx, id),
        _ => {}
    }
}

/// Canceling a draft keeps its selected input available for another tool.
pub(crate) fn cancel_tool(app: &mut App) {
    app.dispatch(Action::Cancel);
    if app.interaction.tool.is_none() {
        app.selected_feature = None;
    }
}

pub(crate) fn cancel_or_deselect(app: &mut App) {
    if app.interaction.tool.is_some() {
        cancel_tool(app);
    } else {
        clear_selection(app, selection_kind(app));
    }
}

pub(crate) fn select_feature(app: &mut App, id: onecad_core::ids::RecordId) {
    app.dispatch(Action::ClearSelection);
    app.topology.select(None);
    app.selected_feature = Some(id);
    app.draw_dirty = true;
}

/// The label's close button explicitly deselects as well as canceling a draft.
fn clear_selection(app: &mut App, kind: SelectionKind) {
    if kind == SelectionKind::SketchEntity {
        if let Some(session) = app.sketch.session.as_ref()
            && let Some(capture) = session.accepted_capture
        {
            app.sketch_dispatch(SketchAction::Select {
                key: session.key.clone(),
                document: session.document.clone(),
                capture,
                targets: Default::default(),
                mode: onecad_interaction::sketch::SelectionMode::Replace,
            });
        }
        return;
    }
    cancel_tool(app);
    app.dispatch(Action::ClearSelection);
    app.selected_body = None;
    app.selected_feature = None;
    app.topology.select(None);
    app.draw_dirty = true;
}

pub(crate) fn body_name(app: &App, id: onecad_core::ids::BodyId) -> String {
    app.display
        .as_ref()
        .and_then(|d| {
            d.projection
                .bodies
                .values()
                .find(|b| b.id.parse().ok() == Some(id))
                .map(|b| b.name.clone())
        })
        .unwrap_or_default()
}

pub(crate) fn selection_label(
    app: &App,
    kind: SelectionKind,
) -> Option<(String, String, SelectionVariant)> {
    let model = SelectionVariant::Model;
    if let Some(tool) = &app.interaction.tool
        && tool.editing_existing
    {
        return Some((tool.record.name.clone(), "feature".into(), model));
    }
    match kind {
        SelectionKind::Face | SelectionKind::Edge => {
            let (_, body) = app.topology.selected_kind()?;
            let name = if kind == SelectionKind::Face {
                "Face"
            } else {
                "Edge"
            };
            Some((name.into(), body_name(app, body), model))
        }
        SelectionKind::Body => Some(("Body".into(), body_name(app, app.selected_body?), model)),
        SelectionKind::Region => Some(("Region".into(), "profile".into(), model)),
        SelectionKind::Feature => {
            let id = app.selected_feature?.to_string();
            let label = app
                .display
                .as_ref()?
                .projection
                .features
                .iter()
                .find(|f| f.id == id)
                .map_or_else(|| "Feature".into(), |f| f.label.clone());
            Some((label, "feature".into(), model))
        }
        SelectionKind::SketchEntity => {
            let session = app.sketch.session.as_ref()?;
            let points = session
                .selection
                .iter()
                .filter(|target| {
                    matches!(
                        target,
                        onecad_interaction::sketch::SketchTarget::Point { .. }
                    )
                })
                .count();
            let entities = session.selection.len() - points;
            let label = if let [target] = session.selection.as_slice() {
                crate::sketch_view::target_name(*target, &session.authoritative)
            } else {
                match (points, entities) {
                    (_, 0) => "Points",
                    (0, _) => "Entities",
                    _ => "Sketch targets",
                }
                .into()
            };
            Some((
                label,
                format!("{} selected", session.selection.len()),
                SelectionVariant::Sketch,
            ))
        }
        SelectionKind::None | SelectionKind::SketchIdle => None,
    }
}

fn accent(app: &App, kind: SelectionKind) -> PillAccent {
    if app.interaction.tool.is_some() {
        return PillAccent::Running;
    }
    match kind {
        SelectionKind::None | SelectionKind::SketchIdle => PillAccent::None,
        SelectionKind::SketchEntity => PillAccent::SketchSelection,
        _ => PillAccent::Selection,
    }
}

#[derive(Default)]
struct Hits {
    clicked: Option<ToolId>,
    section: Option<egui::Response>,
    constraints: Option<egui::Rect>,
    #[cfg(test)]
    tools: Vec<(ToolId, egui::Rect)>,
}

fn draw_groups(
    ui: &mut egui::Ui,
    groups: Groups<'_>,
    views: &[Vec<View>],
    running: Option<ToolId>,
) -> Hits {
    let mut hits = Hits::default();
    for (gi, group) in groups.iter().enumerate() {
        if gi > 0 {
            pill_divider(ui);
        }
        for (def, v) in group.iter().zip(&views[gi]) {
            let source = def
                .source
                .map(|(ext, publisher)| format!("{ext} · {publisher}"));
            let tip = Tip::new(def.label)
                .shortcut(bound_shortcut(def))
                .reason(v.reason)
                .source(source.as_deref());
            let button = ToolButton::new(def.icon)
                .state(v.state)
                .enabled(v.enabled)
                .extension(def.source.is_some());
            let r = ui.add(button.tip(tip));
            #[cfg(test)]
            hits.tools.push((def.id, r.rect));
            if def.id == ToolId::Constraints {
                hits.constraints = Some(r.rect);
            }
            if def.id == ToolId::Section {
                hits.section = Some(r);
            } else if r.clicked() && running != Some(def.id) {
                hits.clicked = Some(def.id);
            }
        }
    }
    hits
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    if crate::editor_facade::PREVIEW_FACADES && app.ui.editor_facade.placing.is_some() {
        crate::editor_facade::placement_pill(app, ctx, chrome);
        return;
    }
    let kind = selection_kind(app);
    let running = running_tool(app);
    let definition = crate::tool_registry::running_definition(app);
    let running_groups = [definition.as_slice()];
    let groups = if running.is_some() {
        &running_groups[..]
    } else {
        pill_tools(kind)
    };
    let views: Vec<Vec<View>> = groups
        .iter()
        .map(|g| g.iter().map(|d| view(app, d, running)).collect())
        .collect();
    let label = selection_label(app, kind);
    let mode = if app.sketch_active() {
        PillMode::Sketch
    } else {
        PillMode::Model
    };
    let accent = accent(app, kind);
    let (mut hits, mut clear) = (Hits::default(), false);
    let bounds = toolbar_rect(ctx, chrome);
    frame::floating(egui::Id::unique("tool-pill"), bounds.min, Order::Middle)
        .pivot(Align2::LEFT_TOP)
        .show(ctx, |ui| {
            bounded_pill(ui, bounds.width(), None, mode, accent, |ui| {
                if let Some((k, c, v)) = &label {
                    clear = SelectionLabel::new(k, c).variant(*v).show(ui).clear;
                    pill_divider(ui);
                }
                hits = draw_groups(ui, groups, &views, running);
            });
        });
    if let Some(id) = hits.clicked {
        activate(app, ctx, id);
    }
    if clear {
        clear_selection(app, kind);
    }
    crate::constraints_menu::show(app, ctx, chrome, hits.constraints);
    crate::constraint_authoring::show(app, ctx);
    crate::sketch_view::show_target_chooser(app, ctx);
    if let Some(r) = hits.section {
        crate::display_menu::popup_for(app, ctx, &r);
    }
}
