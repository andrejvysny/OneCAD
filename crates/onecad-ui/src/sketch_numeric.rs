//! Creation chips retain authored input independently from measured pointer hints.
mod editor;
mod fields;
mod pointer;
use crate::app::App;
use egui::{Context, Key};
use onecad_core::{document::variables::Scalar, math::Vec2};
use onecad_interaction::{
    DocumentContext,
    sketch::{
        CreationDimensions, CreationField, NumericCreationMethod, PrimitiveIds, SketchAction,
        SketchDraft, SketchPhase, SketchSessionKey, SketchStatus, SketchTool,
    },
};

#[derive(Clone)]
struct FieldDraft {
    field: CreationField,
    text: String,
    authored: bool,
}

#[derive(Clone)]
struct Draft {
    key: SketchSessionKey,
    document: DocumentContext,
    tool: SketchTool,
    method: NumericCreationMethod,
    anchor: Vec2,
    secondary: Option<Vec2>,
    generation: u64,
    sequence: u64,
    fields: Vec<FieldDraft>,
    focus: Option<usize>,
    position: Option<egui::Pos2>,
    submission: Option<u64>,
    error: Option<String>,
}

fn id() -> egui::Id {
    egui::Id::unique("sketch-numeric-creation")
}

pub(crate) fn capture_viewport(ctx: &Context, rect: egui::Rect) {
    ctx.data_mut(|data| data.insert_temp(id().with("viewport"), rect));
}

fn viewport_bounds(ctx: &Context) -> egui::Rect {
    ctx.data(|data| data.get_temp::<egui::Rect>(id().with("viewport")))
        .unwrap_or_else(|| ctx.content_rect())
        .intersect(ctx.content_rect())
}

fn seed_position(app: &App, ctx: &Context) -> Option<egui::Pos2> {
    let rect = viewport_bounds(ctx);
    let session = app.sketch.session.as_ref()?;
    let world = session.plane.to_world(session.draft.as_ref()?.cursor);
    let screen = app
        .view
        .as_ref()?
        .project(onecad_scene::DVec3::new(world.x, world.y, world.z))?;
    Some(
        rect.min
            + egui::vec2(
                screen.logical.x as f32 + 16.0,
                screen.logical.y as f32 + 16.0,
            ),
    )
}

pub(crate) fn editor_open(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<Draft>(id()).is_some())
}

pub(crate) fn open(app: &mut App, ctx: &Context) -> bool {
    if editor_open(ctx) {
        return true;
    }
    if availability(app).is_err() {
        return false;
    }
    let seed = app
        .sketch
        .session
        .as_ref()
        .filter(|session| session.tool == SketchTool::Point && session.draft.is_none())
        .map(|session| session.key.clone());
    if let Some(key) = seed {
        app.sketch_dispatch(SketchAction::BeginPointNumeric { key });
    }
    let Some(session) = app.sketch.session.as_ref() else {
        return false;
    };
    let Some(draft) = &session.draft else {
        return false;
    };
    let measured = fields::initial_fields(session);
    let fields = method_fields(session.numeric_method());
    let draft = Draft {
        key: session.key.clone(),
        document: session.document.clone(),
        tool: session.tool,
        method: session.numeric_method(),
        anchor: draft.anchor,
        secondary: draft.secondary.as_ref().map(|anchor| anchor.point),
        generation: session.draft_generation,
        sequence: session.request.sequence,
        fields: fields
            .iter()
            .copied()
            .zip(measured)
            .map(|(field, text)| FieldDraft {
                field,
                text,
                authored: false,
            })
            .collect(),
        focus: Some(0),
        position: seed_position(app, ctx),
        submission: None,
        error: None,
    };
    ctx.data_mut(|data| data.insert_temp(id(), draft));
    true
}

fn method_fields(method: NumericCreationMethod) -> &'static [CreationField] {
    use CreationField::*;
    match method {
        NumericCreationMethod::TangentArc => &[Radius],
        NumericCreationMethod::Tool(tool) => match tool {
            SketchTool::Point => &[X, Y],
            SketchTool::Line => &[Length],
            SketchTool::Rectangle | SketchTool::CenterRectangle => &[Width, Height],
            SketchTool::Circle => &[Diameter],
            SketchTool::Arc => &[Radius, Sweep],
            SketchTool::ThreePointArc => &[Radius],
            SketchTool::Slot => &[Width],
            SketchTool::Select => &[],
        },
    }
}

pub(crate) fn availability(app: &App) -> Result<(), &'static str> {
    if !ready(app) {
        return Err("Sketch is not ready for numeric creation");
    }
    let session = app.sketch.session.as_ref().ok_or("Open a sketch first")?;
    match session.tool {
        SketchTool::Select => Err("Choose a creation tool"),
        SketchTool::Point => Ok(()),
        SketchTool::Arc | SketchTool::ThreePointArc | SketchTool::Slot
            if session
                .draft
                .as_ref()
                .is_none_or(|draft| draft.secondary.is_none()) =>
        {
            Err("Place the first two anchors before numeric creation")
        }
        _ if session.draft.is_none() => Err("Place a primitive anchor before numeric creation"),
        _ => Ok(()),
    }
}

pub(crate) fn route(app: &mut App, ctx: &Context, raw: &mut egui::RawInput) -> bool {
    let tab = raw.events.iter().any(|event| {
        matches!(event,
        egui::Event::Key { key: Key::Tab, pressed: true, repeat: false, modifiers, .. }
        if !modifiers.command && !modifiers.ctrl && !modifiers.alt)
    });
    if tab && open(app, ctx) {
        raw.events
            .retain(|event| !matches!(event, egui::Event::Key { key: Key::Tab, .. }));
        return true;
    }
    false
}

fn current(draft: &Draft, app: &App) -> bool {
    if !app.viewport_ready() || !app.bridge.valid() {
        return false;
    }
    app.sketch.session.as_ref().is_some_and(|session| {
        session.key == draft.key
            && session.draft_generation == draft.generation
            && session.tool == draft.tool
            && session.numeric_method() == draft.method
            && app.sketch.context.as_ref() == Some(&session.document)
            && session.draft.as_ref().is_some_and(|current| {
                (draft.tool == SketchTool::Point || current.anchor == draft.anchor)
                    && current.secondary.as_ref().map(|anchor| anchor.point) == draft.secondary
            })
            && if let Some(sequence) = draft.submission {
                session.request.sequence == sequence
                    && matches!(
                        session.phase,
                        SketchPhase::Active | SketchPhase::Submitting | SketchPhase::Accepted
                    )
            } else {
                session.document == draft.document
                    && session.request.sequence == draft.sequence
                    && session.phase == SketchPhase::Active
            }
    })
}

fn ready(app: &App) -> bool {
    crate::sketch_ui::tools_enabled(app)
        && app.sketch.session.as_ref().is_some_and(|session| {
            app.view.as_ref().is_some_and(|view| {
                session.accepted_capture == Some(view.identity())
                    && session.document.scene == view.identity().scene
            })
        })
}

pub(crate) fn show(app: &mut App, ctx: &Context) {
    let Some(mut draft) = ctx.data_mut(|data| {
        let draft = data.get_temp::<Draft>(id());
        data.remove::<Draft>(id());
        draft
    }) else {
        return;
    };
    if !current(&draft, app) {
        return;
    }
    if let Some(session) = app.sketch.session.as_ref() {
        if draft.submission.is_some() && session.phase == SketchPhase::Active {
            if let SketchStatus::Refused(message) = &session.status {
                draft.error = Some(message.clone());
                draft.submission = None;
                draft.sequence = session.request.sequence;
                draft.document = session.document.clone();
            } else {
                return;
            }
        }
        if draft.submission.is_none() {
            let measured = fields::initial_fields(session);
            for (field, value) in draft.fields.iter_mut().zip(measured) {
                if !field.authored {
                    field.text = value
                }
            }
        }
    }
    let (apply, cancel) = editor::show(app, ctx, &mut draft);
    if cancel {
        cancel_editor(app, &draft);
        return;
    }
    if draft.submission.is_none() {
        preview(app, &draft);
        if apply {
            commit(app, &mut draft)
        }
    }
    ctx.data_mut(|data| data.insert_temp(id(), draft));
}

fn cancel_editor(app: &mut App, draft: &Draft) {
    if draft.submission.is_some() {
        if let Some(tag) = app
            .sketch
            .session
            .as_ref()
            .map(|session| session.request.clone())
        {
            app.cancel_sketch_candidate(&tag);
        }
        app.sketch_dispatch(SketchAction::Escape);
    } else if let Some(point) = app.sketch.session.as_ref().and_then(|session| {
        Some(onecad_interaction::sketch::CapturedSketchPoint {
            key: session.key.clone(),
            capture: session.accepted_capture?,
            point: session.draft.as_ref()?.cursor,
        })
    }) {
        app.sketch_dispatch(SketchAction::Move(point));
    }
}

fn preview(app: &mut App, draft: &Draft) {
    let Ok(dimensions) = fields::dimensions(draft, app) else {
        return;
    };
    let Some(capture) = app
        .sketch
        .session
        .as_ref()
        .and_then(|session| session.accepted_capture)
    else {
        return;
    };
    app.sketch_dispatch(SketchAction::PreviewNumeric {
        key: draft.key.clone(),
        capture,
        dimensions: Box::new(dimensions),
    });
}

fn commit(app: &mut App, draft: &mut Draft) {
    if !current(draft, app) || !ready(app) {
        return;
    }
    let Ok(dimensions) = fields::dimensions(draft, app) else {
        return;
    };
    let Some(capture) = app
        .sketch
        .session
        .as_ref()
        .and_then(|session| session.accepted_capture)
    else {
        return;
    };
    app.sketch_dispatch(SketchAction::CreateNumericAuthored {
        draft_generation: draft.generation,
        key: draft.key.clone(),
        capture,
        ids: PrimitiveIds {
            entities: std::array::from_fn(|_| onecad_core::ids::EntityId::new()),
            constraints: std::array::from_fn(|_| onecad_core::ids::ConstraintId::new()),
        },
        dimensions: Box::new(dimensions),
        authored: draft
            .fields
            .iter()
            .filter(|field| field.authored)
            .map(|field| field.field)
            .collect(),
    });
    if let Some(session) = app.sketch.session.as_ref() {
        if session.phase == SketchPhase::Submitting {
            draft.submission = Some(session.request.sequence);
        } else if let SketchStatus::Refused(message) = &session.status {
            draft.error = Some(message.clone());
        }
    }
}

/// Only canvas-owned motion may change untouched measured fields.
pub(crate) fn viewport(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &onecad_scene::ViewSnapshot,
) {
    let Some(mut draft) = ui.ctx().data(|data| data.get_temp::<Draft>(id())) else {
        return;
    };
    if !current(&draft, app) || draft.submission.is_some() {
        return;
    }
    if draft.position.is_none() {
        draft.position = app.sketch.session.as_ref().and_then(|session| {
            let point = session.plane.to_world(session.draft.as_ref()?.cursor);
            let screen = view.project(onecad_scene::DVec3::new(point.x, point.y, point.z))?;
            Some(
                rect.min
                    + egui::vec2(
                        screen.logical.x as f32 + 16.0,
                        screen.logical.y as f32 + 16.0,
                    ),
            )
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(id(), draft.clone()));
    }
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let key = session.key.clone();
    let document = session.document.clone();
    app.sketch_dispatch(SketchAction::AcceptView {
        key,
        document,
        capture: view.identity(),
    });
    let blocked = app.composing
        || app.documents.confirm.is_some()
        || crate::constraint_authoring::editor_open(ui.ctx())
        || crate::input::ui_owns_keyboard(&app.ui, ui.ctx());
    let Some(position) = pointer::canvas_position(ui, rect, blocked) else {
        return;
    };
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let logical = onecad_scene::DVec2::new(
        f64::from(position.x - rect.min.x),
        f64::from(position.y - rect.min.y),
    );
    let Ok(point) = onecad_interaction::sketch::capture_sketch_pointer(session, view, logical)
    else {
        return;
    };
    app.sketch_dispatch(SketchAction::Move(point));
}
