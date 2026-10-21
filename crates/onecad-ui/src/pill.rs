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
    selection_set::{SelectionItem, SelectionMode},
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

mod catalog;
mod layout;
use layout::{bounded_pill, toolbar_rect};

pub(crate) fn center_x(ctx: &Context, chrome: &Chrome) -> f32 {
    toolbar_rect(ctx, chrome).center().x
}

type Groups<'a> = &'a [&'a [ToolDef]];

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
    if !app.document_open() {
        return gate(false, "Create or open a document first");
    }
    if app.doc.interaction.tool.is_some() || app.busy() {
        return gate(false, "Finish the current operation first");
    }
    if !app.viewport_ready() {
        return gate(false, "Viewport unavailable");
    }
    if !app.doc.bridge.valid() {
        return gate(false, "Worker unavailable");
    }
    if crate::viewport_selection::sectioned(app) {
        return gate(false, "Turn off section view first");
    }
    gate(eligible, reason)
}

fn sketch_view(app: &App, tool: SketchTool) -> View {
    let selected = app
        .doc
        .sketch
        .session
        .as_ref()
        .is_some_and(|s| s.tool == tool);
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
            app.doc.interaction.selection.is_some(),
            "Select a closed sketch profile first",
        ),
        ToolId::Edit => model_view(
            app,
            app.doc.selected_feature().is_some_and(|id| {
                app.doc
                    .display
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
        ToolId::MoveRotate => model_view(
            app,
            app.doc.selected_body().is_some(),
            "Select a body first",
        ),
        ToolId::Import => gate(
            crate::bars::replacement_allowed(app),
            "Finish the current operation first",
        ),
        ToolId::More => gate(true, ""),
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
                    .doc
                    .sketch
                    .session
                    .as_ref()
                    .is_some_and(|session| session.construction)
            {
                state.state = ToolState::SketchActive;
            }
            state
        }
        ToolId::InsertComponent | ToolId::MakeComponent
            if crate::editor_facade::PREVIEW_FACADES =>
        {
            gate(true, "")
        }
        ToolId::Flange | ToolId::Dogbone | ToolId::CheckGeometry
            if crate::editor_facade::PREVIEW_FACADES =>
        {
            let mut state = gate(true, "");
            if app.ui.editor_facade.active_tool == Some(def.id) {
                state.state = ToolState::Active;
            }
            state
        }
        ToolId::Constraints => {
            let mut v = gate(sketch_ui::tools_enabled(app), "Sketch is not ready");
            if app.ui.constraints_open {
                v.state = ToolState::Active;
            }
            v
        }
        _ => {
            let reason = crate::tool_registry::unavailable_reason(def.id);
            gate(false, reason.split(" (ONECAD-").next().unwrap_or(reason))
        }
    }
}

/// Registry shortcuts are design data; show one only once the app binds it (none yet).
fn bound_shortcut(def: &ToolDef) -> Option<&'static str> {
    const BOUND: &[ToolId] = &[];
    def.shortcut.filter(|_| BOUND.contains(&def.id))
}

pub(crate) fn activate(app: &mut App, ctx: &Context, id: ToolId) {
    let definition = ToolDef {
        id,
        icon: "select",
        label: "",
        shortcut: None,
        source: None,
    };
    if !view(app, &definition, None).enabled {
        return;
    }
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
            app.doc.gesture += 1;
            let step = app
                .doc
                .display
                .as_ref()
                .map_or(0, |display| display.projection.applied_ops as u32);
            app.dispatch(Action::BeginExtrude {
                record_id: onecad_core::ids::RecordId::new(),
                gesture: app.doc.gesture,
                step_index: step,
            });
        }
        ToolId::Edit => crate::inspector_history::edit_selected_feature(app),
        ToolId::MoveRotate => app.begin_transform(),
        ToolId::Import => app.document_request(crate::document_flow::Request::Import),
        ToolId::Section => crate::display_menu::toggle(app, ctx.content_rect().shrink(48.0)),
        ToolId::InsertComponent if crate::editor_facade::PREVIEW_FACADES => {
            crate::editor_facade::show_library(app)
        }
        ToolId::MakeComponent if crate::editor_facade::PREVIEW_FACADES => {
            crate::component_wizard::open(&mut app.ui)
        }
        ToolId::Flange | ToolId::Dogbone | ToolId::CheckGeometry
            if crate::editor_facade::PREVIEW_FACADES =>
        {
            app.ui.editor_facade.press_extension_tool(id)
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
    if app.doc.interaction.tool.is_none() {
        crate::item_selection::retain(app, |item| !matches!(item, SelectionItem::Feature(_)));
    }
}

pub(crate) fn cancel_or_deselect(app: &mut App) {
    if app.doc.interaction.tool.is_some() {
        cancel_tool(app);
    } else {
        clear_selection(app, selection_kind(app));
    }
}

pub(crate) fn select_feature(app: &mut App, id: onecad_core::ids::RecordId) {
    crate::item_selection::apply(
        app,
        SelectionMode::Replace,
        vec![SelectionItem::Feature(id)],
    );
}

/// The label's close button explicitly deselects as well as canceling a draft.
fn clear_selection(app: &mut App, kind: SelectionKind) {
    if kind == SelectionKind::SketchEntity {
        if let Some(session) = app.doc.sketch.session.as_ref()
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
    crate::item_selection::clear(app);
}

pub(crate) fn body_name(app: &App, id: onecad_core::ids::BodyId) -> String {
    app.doc
        .display
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
    if let Some(tool) = &app.doc.interaction.tool
        && tool.editing_existing
    {
        return Some((tool.record.name.clone(), "feature".into(), model));
    }
    let count = app.doc.selection.len();
    let context = || match app.doc.selected_body() {
        Some(body) => body_name(app, body),
        None => {
            let items = app.doc.selection.items().iter();
            let bodies: std::collections::HashSet<_> =
                items.filter_map(SelectionItem::body).collect();
            format!("{} bodies", bodies.len())
        }
    };
    match kind {
        SelectionKind::Face => Some((counted(count, "Face", "Faces"), context(), model)),
        SelectionKind::Edge => Some((counted(count, "Edge", "Edges"), context(), model)),
        SelectionKind::Body if count == 1 => Some(("Body".into(), context(), model)),
        SelectionKind::Body => Some((counted(count, "Body", "Bodies"), "bodies".into(), model)),
        SelectionKind::Region => Some((
            counted(count.max(1), "Region", "Regions"),
            "profile".into(),
            model,
        )),
        SelectionKind::Mixed => Some((format!("{count} items"), "mixed".into(), model)),
        SelectionKind::Feature if count > 1 => Some((
            counted(count, "Feature", "Features"),
            "feature".into(),
            model,
        )),
        SelectionKind::Feature => {
            let id = app.doc.selected_feature()?.to_string();
            let label = app
                .doc
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
            let session = app.doc.sketch.session.as_ref()?;
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

/// "Face" for one item, "3 Faces" for several: the existing label, counted.
fn counted(count: usize, one: &str, many: &str) -> String {
    if count > 1 {
        format!("{count} {many}")
    } else {
        one.into()
    }
}

fn accent(app: &App, kind: SelectionKind) -> PillAccent {
    if app.doc.interaction.tool.is_some() {
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
    more: Option<egui::Response>,
    drawn: Vec<ToolId>,
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
            hits.drawn.push(def.id);
            #[cfg(test)]
            hits.tools.push((def.id, r.rect));
            if def.id == ToolId::Constraints {
                hits.constraints = Some(r.rect);
            }
            if def.id == ToolId::More {
                hits.more = Some(r);
            } else if def.id == ToolId::Section {
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
    let label = selection_label(app, kind);
    let mut bounds = toolbar_rect(ctx, chrome);
    let lane = layout::toolbar_bounds(chrome);
    let active = groups
        .iter()
        .flat_map(|group| group.iter())
        .find(|def| {
            matches!(
                view(app, def, running).state,
                ToolState::SketchActive | ToolState::Running
            )
        })
        .map(|def| def.id);
    let compact = layout::compact_tools(groups, active, lane.width(), label.is_some());
    let mode_key = egui::Id::unique("tool-pill-compact");
    if ctx.data(|data| data.get_temp::<bool>(mode_key)) != Some(compact.is_some()) {
        bounds = lane;
        ctx.data_mut(|data| data.insert_temp(mode_key, compact.is_some()));
    }
    let compact_groups = [compact.as_deref().unwrap_or_default()];
    let groups = if compact.is_some() {
        &compact_groups[..]
    } else {
        groups
    };
    let views: Vec<Vec<View>> = groups
        .iter()
        .map(|g| g.iter().map(|d| view(app, d, running)).collect())
        .collect();
    let mode = if app.sketch_active() {
        PillMode::Sketch
    } else {
        PillMode::Model
    };
    let accent = accent(app, kind);
    let (mut hits, mut clear) = (Hits::default(), false);
    frame::floating(egui::Id::unique("tool-pill"), bounds.min, Order::Middle)
        .pivot(Align2::LEFT_TOP)
        .show(ctx, |ui| {
            bounded_pill(ui, lane.width(), None, mode, accent, |ui| {
                if compact.is_none()
                    && let Some((k, c, v)) = &label
                {
                    clear = SelectionLabel::new(k, c).variant(*v).show(ui).clear;
                    pill_divider(ui);
                }
                hits = draw_groups(ui, groups, &views, running);
            });
        });
    hits.clicked =
        catalog::show(app, chrome, hits.more.as_ref(), running, &hits.drawn).or(hits.clicked);
    if let Some(id) = hits.clicked {
        activate(app, ctx, id);
    }
    if clear {
        clear_selection(app, kind);
    }
    crate::constraints_menu::show(
        app,
        ctx,
        chrome,
        hits.constraints
            .or_else(|| hits.more.as_ref().map(|response| response.rect)),
    );
    crate::constraint_authoring::show(app, ctx);
    crate::sketch_view::show_target_chooser(app, ctx);
    if let Some(r) = hits.section {
        crate::display_menu::popup_for(app, ctx, &r);
    }
}
