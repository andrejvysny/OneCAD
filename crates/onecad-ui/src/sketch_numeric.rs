//! Numeric creation is an explicit command; blur never commits geometry.
use crate::app::App;
use egui::{Context, Key};
use onecad_core::{document::variables::Scalar, math::Vec2};
use onecad_interaction::{
    DocumentContext,
    sketch::{
        CreationDimensions, PrimitiveIds, SketchAction, SketchDraft, SketchPhase, SketchSessionKey,
        SketchTool,
    },
};

#[derive(Clone)]
struct Draft {
    key: SketchSessionKey,
    document: DocumentContext,
    tool: SketchTool,
    anchor: Vec2,
    cursor: Vec2,
    secondary: Option<Vec2>,
    fields: Vec<String>,
    focus: bool,
}

fn id() -> egui::Id {
    egui::Id::unique("sketch-numeric-creation")
}

pub(crate) fn editor_open(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<Draft>(id()).is_some())
}

pub(crate) fn open(app: &mut App, ctx: &Context) -> bool {
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
    let Some(session) = app.sketch.session.as_ref().filter(|session| {
        session.phase == SketchPhase::Active && session.tool != SketchTool::Select
    }) else {
        return false;
    };
    let Some(draft) = &session.draft else {
        return false;
    };
    if session.tool == SketchTool::ThreePointArc
        || matches!(session.tool, SketchTool::Arc | SketchTool::Slot) && draft.secondary.is_none()
    {
        return false;
    }
    ctx.data_mut(|data| {
        data.insert_temp(
            id(),
            Draft {
                key: session.key.clone(),
                document: session.document.clone(),
                tool: session.tool,
                anchor: draft.anchor,
                cursor: draft.cursor,
                secondary: draft.secondary.as_ref().map(|anchor| anchor.point),
                fields: initial_fields(session.tool, draft),
                focus: true,
            },
        )
    });
    true
}

pub(crate) fn availability(app: &App) -> Result<(), &'static str> {
    if !ready(app) {
        return Err("Sketch is not ready for numeric creation");
    }
    let session = app.sketch.session.as_ref().ok_or("Open a sketch first")?;
    match session.tool {
        SketchTool::Select => Err("Choose a creation tool"),
        SketchTool::ThreePointArc => {
            Err("Three-point arc numeric radius is unavailable; place the through point")
        }
        SketchTool::Point => Ok(()),
        SketchTool::Arc | SketchTool::Slot
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

fn initial_fields(tool: SketchTool, draft: &SketchDraft) -> Vec<String> {
    let (anchor, cursor) = (draft.anchor, draft.cursor);
    let delta = Vec2::new_unchecked(cursor.x - anchor.x, cursor.y - anchor.y);
    let values = match tool {
        SketchTool::Point => vec![cursor.x, cursor.y],
        SketchTool::CenterRectangle => vec![2.0 * delta.x.abs(), 2.0 * delta.y.abs()],
        SketchTool::Line => vec![delta.x.hypot(delta.y)],
        SketchTool::Rectangle => vec![delta.x.abs(), delta.y.abs()],
        SketchTool::Circle => vec![2.0 * delta.x.hypot(delta.y)],
        SketchTool::Arc => arc_values(draft),
        SketchTool::Slot => slot_values(draft),
        SketchTool::Select | SketchTool::ThreePointArc => Vec::new(),
    };
    values
        .into_iter()
        .map(|value| {
            if tool == SketchTool::Point || value.is_finite() && value > 0.0 {
                value
            } else {
                value.max(1.0)
            }
            .to_string()
        })
        .collect()
}

fn arc_values(draft: &SketchDraft) -> Vec<f64> {
    let Some(start) = draft.secondary.as_ref().map(|anchor| anchor.point) else {
        return Vec::new();
    };
    let radius = (start.x - draft.anchor.x).hypot(start.y - draft.anchor.y);
    let sweep = onecad_interaction::sketch::arc_geometry::center_start_end(
        draft.anchor,
        start,
        draft.cursor,
    )
    .map_or(90.0, |arc| arc.sweep.to_degrees());
    vec![radius, sweep]
}

fn slot_values(draft: &SketchDraft) -> Vec<f64> {
    let Some(end) = draft.secondary.as_ref().map(|anchor| anchor.point) else {
        return Vec::new();
    };
    let (dx, dy) = (end.x - draft.anchor.x, end.y - draft.anchor.y);
    vec![
        2.0 * (dx * (draft.cursor.y - draft.anchor.y) - dy * (draft.cursor.x - draft.anchor.x))
            .abs()
            / dx.hypot(dy),
    ]
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

fn dimensions(draft: &Draft, app: &App) -> Result<CreationDimensions, String> {
    let variables = app
        .display
        .as_ref()
        .map_or(&[][..], |display| display.projection.variables.as_slice());
    let values: Vec<Scalar> = draft
        .fields
        .iter()
        .enumerate()
        .map(|(index, text)| {
            if draft.tool == SketchTool::Arc && index == 1 {
                crate::constraint_authoring::angle_scalar(text, variables)
            } else if draft.tool == SketchTool::Point {
                crate::constraint_authoring::coordinate_scalar(text, variables)
            } else {
                crate::constraint_authoring::creation_scalar(text, variables)
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(match draft.tool {
        SketchTool::Point => CreationDimensions::Point {
            x: values[0].clone(),
            y: values[1].clone(),
        },
        SketchTool::CenterRectangle => CreationDimensions::CenterRectangle {
            width: values[0].clone(),
            height: values[1].clone(),
        },
        SketchTool::Line => CreationDimensions::Line {
            length: values[0].clone(),
        },
        SketchTool::Rectangle => CreationDimensions::Rectangle {
            width: values[0].clone(),
            height: values[1].clone(),
        },
        SketchTool::Circle => CreationDimensions::Circle {
            diameter: values[0].clone(),
        },
        SketchTool::Arc => CreationDimensions::Arc {
            radius: values[0].clone(),
            sweep: values[1].value,
        },
        SketchTool::Slot => CreationDimensions::Slot {
            width: values[0].clone(),
        },
        SketchTool::ThreePointArc => {
            return Err(
                "Three-point arc numeric radius is unavailable; place the through point".into(),
            );
        }
        SketchTool::Select => return Err("Choose a creation tool".into()),
    })
}

fn current(draft: &Draft, app: &App) -> bool {
    ready(app)
        && app.sketch.session.as_ref().is_some_and(|session| {
            session.key == draft.key
                && session.document == draft.document
                && session.tool == draft.tool
                && session.phase == SketchPhase::Active
                && session.draft.as_ref().is_some_and(|current| {
                    current.anchor == draft.anchor
                        && current.cursor == draft.cursor
                        && current.secondary.as_ref().map(|anchor| anchor.point) == draft.secondary
                })
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
    let (apply, cancel) = editor(app, ctx, &mut draft);
    preview(app, &draft);
    if cancel {
        app.sketch_dispatch(SketchAction::Escape);
    } else if apply {
        commit(app, &draft);
    } else {
        ctx.data_mut(|data| data.insert_temp(id(), draft));
    }
}

fn editor(app: &App, ctx: &Context, draft: &mut Draft) -> (bool, bool) {
    let mut apply = false;
    let mut cancel = false;
    let modal = egui::Modal::new(id()).show(ctx, |ui| {
        ui.heading("Create precisely");
        ui.small("Millimetres or expressions. Tab changes fields; Enter creates.");
        if draft.tool == SketchTool::Point {
            ui.small("X and Y are measured from the sketch origin. A construction origin carries these dimensions.");
        }
        if draft.tool == SketchTool::Arc {
            ui.small("Sweep uses degrees. Its sign sets travel direction; sweep is geometry only. Radius expressions remain bound.");
        }
        let labels: &[&str] = match draft.tool {
            SketchTool::Point => &["X", "Y"],
            SketchTool::CenterRectangle => &["Width", "Height"],
            SketchTool::Line => &["Length"],
            SketchTool::Rectangle => &["Width", "Height"],
            SketchTool::Circle => &["Diameter"],
            SketchTool::Arc => &["Radius", "Sweep (degrees)"],
            SketchTool::Slot => &["Width"],
            SketchTool::Select | SketchTool::ThreePointArc => &[],
        };
        for (index, (text, label)) in draft.fields.iter_mut().zip(labels).enumerate() {
            ui.label(*label);
            let field = ui.add(
                egui::TextEdit::singleline(text).char_limit(onecad_core::expr::MAX_SOURCE_LEN),
            );
            if index == 0 && draft.focus {
                field.request_focus();
            }
        }
        draft.focus = false;
        let parsed = dimensions(draft, app);
        if let Err(error) = &parsed {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.horizontal(|ui| {
            let ready = parsed.is_ok() && !app.composing;
            apply = ui.add_enabled(ready, egui::Button::new("Create")).clicked()
                || ready && ui.input(|input| input.key_pressed(Key::Enter));
            cancel = ui.button("Cancel").clicked();
        });
    });
    (apply, cancel || modal.should_close())
}

fn preview(app: &mut App, draft: &Draft) {
    let Ok(dimensions) = dimensions(draft, app) else {
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

fn commit(app: &mut App, draft: &Draft) {
    if !current(draft, app) {
        return;
    }
    let Ok(dimensions) = dimensions(draft, app) else {
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
    app.sketch_dispatch(SketchAction::CreateNumeric {
        key: draft.key.clone(),
        capture,
        ids: PrimitiveIds {
            entities: std::array::from_fn(|_| onecad_core::ids::EntityId::new()),
            constraints: std::array::from_fn(|_| onecad_core::ids::ConstraintId::new()),
        },
        dimensions: Box::new(dimensions),
    });
}
