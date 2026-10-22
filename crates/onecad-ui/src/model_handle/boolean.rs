use super::*;
pub(super) struct ChoicePress {
    record: RecordId,
    index: usize,
    frame: (onecad_scene::Camera, onecad_scene::Viewport, Rect),
}
pub(super) fn parameters(app: &App) -> Option<onecad_core::document::record::BooleanParams> {
    if !app.viewport_ready()
        || !app.doc.bridge.valid()
        || app.sketch_active()
        || crate::viewport_selection::sectioned(app)
    {
        return None;
    }
    let tool = app
        .doc
        .interaction
        .tool
        .as_ref()
        .filter(|t| t.phase == onecad_interaction::SessionPhase::Editing)?;
    match tool.effective_op() {
        onecad_core::document::record::Operation::Known(
            onecad_core::document::record::KnownOperation::Boolean(p),
        ) => Some(p),
        _ => None,
    }
}
pub(super) fn bounds(app: &App, view: &ViewSnapshot, rect: Rect) -> Option<Rect> {
    let p = parameters(app)?;
    let display = app.doc.display.as_ref()?;
    let context = anchors::Context {
        scene: &display.scene,
        records: &display.records,
        profiles: &display.profiles,
        profile_anchors: &display.profile_anchors,
    };
    let center = anchors::center(&context, p.target_body)?;
    let point = crate::gizmo::project(view, rect, center)?;
    let mut rect = rect;
    if app
        .doc
        .interaction
        .tool
        .as_ref()
        .is_some_and(|t| !t.editing_existing)
    {
        let chrome = crate::layout::chrome(Rect::from_min_max(Pos2::ZERO, rect.max), &app.ui)
            .with_feature_card();
        rect.max.x = rect
            .max
            .x
            .min(chrome.feature_card.left() - 8.0)
            .max(rect.min.x + 246.0);
    }
    let min = point + egui::vec2(-123.0, -64.0);
    Some(
        Rect::from_min_size(min, egui::vec2(246.0, 32.0)).translate(egui::vec2(
            (rect.left() - min.x).max(0.0) - (min.x + 246.0 - rect.right()).max(0.0),
            (rect.top() - min.y).max(0.0) - (min.y + 32.0 - rect.bottom()).max(0.0),
        )),
    )
}
pub(super) fn show(app: &mut App, ui: &mut egui::Ui, view: &ViewSnapshot, rect: Rect) -> bool {
    let (Some(p), Some(bounds)) = (parameters(app), bounds(app, view, rect)) else {
        return false;
    };
    let target = p.target_body;
    let mut operation = p.operation;
    if let Some(index) = boolean_events(app, ui, view, bounds) {
        operation = [
            onecad_core::document::record::BooleanOp::Union,
            onecad_core::document::record::BooleanOp::Cut,
            onecad_core::document::record::BooleanOp::Intersect,
        ][index];
    }
    ui.painter()
        .rect_filled(bounds, 5.0, ui.visuals().panel_fill);
    choices(ui, bounds, &mut operation);
    if operation != p.operation
        && let Some(tool) = app.doc.interaction.tool.as_ref()
    {
        app.dispatch(Action::ModelingOptionsChanged {
            tag: tool.tag.clone(),
            options: ModelingOptions::Boolean {
                operation,
                target,
                tools: p.tools,
                keep: p.keep_tools,
            },
        });
    }
    ui.input(|i| i.pointer.hover_pos().is_some_and(|p| bounds.contains(p)))
}

fn choices(
    ui: &mut egui::Ui,
    bounds: Rect,
    operation: &mut onecad_core::document::record::BooleanOp,
) {
    ui.add_enabled_ui(crate::viewport_widgets::gesture_allowed(ui), |ui| {
        for (index, (value, label)) in [
            (onecad_core::document::record::BooleanOp::Union, "Union"),
            (onecad_core::document::record::BooleanOp::Cut, "Subtract"),
            (
                onecad_core::document::record::BooleanOp::Intersect,
                "Intersect",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let button = Rect::from_min_size(
                bounds.min + egui::vec2(3.0 + index as f32 * 80.0, 3.0),
                egui::vec2(78.0, 26.0),
            );
            let response = ui.put(
                button,
                egui::Button::new(label).selected(*operation == value),
            );
            crate::automation::name(
                ui.ctx(),
                response.id,
                &format!("onecad.viewport.boolean.{label}"),
            );
            if response.clicked() {
                *operation = value;
            }
        }
    });
}

fn boolean_events(
    app: &mut App,
    ui: &egui::Ui,
    view: &ViewSnapshot,
    bounds: Rect,
) -> Option<usize> {
    let record = app.doc.interaction.tool.as_ref()?.tag.record_id;
    let frame = (view.camera(), view.viewport(), bounds);
    if !crate::viewport_widgets::gesture_allowed(ui)
        || app
            .doc
            .model_handle
            .choice
            .as_ref()
            .is_some_and(|p| p.record != record || p.frame != frame)
    {
        app.doc.model_handle.choice = None;
        return None;
    }
    let mut chosen = None;
    for event in ui.input(|i| i.events.clone()) {
        match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } => {
                if crate::viewport_widgets::allows_pointer(ui, bounds, pos) {
                    app.doc.model_handle.choice = index_at(bounds, pos).map(|index| ChoicePress {
                        record,
                        index,
                        frame,
                    });
                }
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } => {
                if let Some(press) = app.doc.model_handle.choice.take()
                    && crate::viewport_widgets::allows_pointer(ui, bounds, pos)
                    && index_at(bounds, pos) == Some(press.index)
                {
                    chosen = Some(press.index);
                }
            }
            egui::Event::PointerGone | egui::Event::WindowFocused(false) => {
                app.doc.model_handle.choice = None
            }
            _ => {}
        }
    }
    chosen
}

fn index_at(bounds: Rect, pos: Pos2) -> Option<usize> {
    (0..3).find(|&index| {
        Rect::from_min_size(
            bounds.min + egui::vec2(3.0 + index as f32 * 80.0, 3.0),
            egui::vec2(78.0, 26.0),
        )
        .contains(pos)
    })
}

pub(super) fn settle(app: &mut App, focused: bool, down: bool) {
    if app.doc.model_handle.choice.as_ref().is_some_and(|p| {
        !focused
            || !down
            || Some(p.record) != app.doc.interaction.tool.as_ref().map(|t| t.tag.record_id)
            || p.frame.0 != app.doc.camera
            || app
                .doc
                .view
                .as_ref()
                .is_none_or(|v| p.frame.1 != v.viewport())
    }) {
        app.doc.model_handle.choice = None;
    }
}
