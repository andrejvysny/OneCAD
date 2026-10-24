//! Existing-kernel modeling commands and choice controls for the shared feature card.
use crate::{app::App, tool_registry::ToolId};
use onecad_core::{
    document::{body::BodyHealth, record::*},
    ids::{BodyId, RecordId},
    math::Vec3,
};
use onecad_interaction::{
    Action, ModelSession, ModelingOptions, SessionPhase, modeling_records, operand_binding,
    selection_set::SelectionItem,
};
use onecad_widgets::components::overlays::FeatureCardRows;

pub(crate) fn editable(op: &Operation) -> bool {
    !onecad_interaction::numeric_parameters(op).is_empty()
        || matches!(
            op,
            Operation::Known(
                KnownOperation::Boolean(_)
                    | KnownOperation::MirrorBody(_)
                    | KnownOperation::Loft(_)
            )
        )
}

pub(crate) fn selected_bodies(app: &App) -> Option<Vec<BodyId>> {
    let display = app.doc.display.as_ref()?;
    let bodies: Option<Vec<_>> = app
        .doc
        .selection
        .items()
        .iter()
        .map(|item| {
            let SelectionItem::Body(id) = item else {
                return None;
            };
            display
                .projection
                .bodies
                .values()
                .any(|body| {
                    body.id == id.to_string() && body.visible && body.health == BodyHealth::Healthy
                })
                .then_some(*id)
        })
        .collect();
    bodies.filter(|bodies| !bodies.is_empty())
}

pub(crate) fn begin_body_operation(app: &mut App, id: ToolId) {
    let bodies = if id == ToolId::Generators {
        Vec::new()
    } else {
        let Some(bodies) = selected_bodies(app) else {
            return;
        };
        bodies
    };
    let Some(context) = app.doc.interaction.context.clone() else {
        return;
    };
    let step = app
        .doc
        .display
        .as_ref()
        .map_or(0, |d| d.projection.applied_ops as u32);
    let record_id = RecordId::new();
    let record = match id {
        ToolId::Generators => Ok(modeling_records::gear(record_id, step)),
        ToolId::LinearPattern => {
            let spacing = app
                .doc
                .display
                .as_ref()
                .and_then(|d| d.scene.bodies().iter().find(|b| b.body_id() == bodies[0]))
                .map_or(10.0, |b| (b.bounds().max.x - b.bounds().min.x).abs() + 10.0);
            Ok(modeling_records::linear_pattern(
                bodies[0], spacing, record_id, step,
            ))
        }
        ToolId::CircularPattern => Ok(modeling_records::circular_pattern(
            bodies[0], record_id, step,
        )),
        ToolId::MirrorBody => Ok(modeling_records::mirror(bodies[0], record_id, step)),
        ToolId::Boolean => modeling_records::boolean(&bodies, record_id, step),
        _ => return,
    };
    app.doc.gesture += 1;
    match record.and_then(|record| operand_binding::begin_bound(record, app.doc.gesture, context)) {
        Ok(action) => app.dispatch(action),
        Err(error) => app.doc.status = error,
    }
    app.draw_dirty = true;
}

pub(crate) fn rows(app: &mut App, rows: &mut FeatureCardRows, tool: &ModelSession) {
    if tool.phase != SessionPhase::Editing {
        return;
    }
    let Operation::Known(op) = &tool.record.op else {
        return;
    };
    let options = match op {
        KnownOperation::Loft(p) => crate::loft_ui::rows(app, rows, tool, p),
        KnownOperation::Extrude(p) => extent(rows, p),
        KnownOperation::OffsetFace(p) => face_offset(rows, p),
        KnownOperation::Revolve(p) => {
            crate::revolve_ui::rows(app, rows, tool, p);
            profile_boolean(app, rows, p.boolean_mode, p.target_body)
        }
        KnownOperation::LinearPattern(p) => pattern(
            rows,
            p.count,
            p.direction,
            Vec3::new_unchecked(0.0, 0.0, 0.0),
            p.fuse_result,
            false,
        ),
        KnownOperation::CircularPattern(p) => pattern(
            rows,
            p.count,
            p.axis_direction,
            p.axis_origin,
            p.fuse_result,
            true,
        ),
        KnownOperation::MirrorBody(p) => mirror(rows, p),
        KnownOperation::Boolean(p) => boolean(app, rows, p),
        KnownOperation::Hole(p) => hole(rows, p),
        KnownOperation::Gear(p) => gear(rows, p),
        KnownOperation::TransformBody(p) => {
            let mut copy = p.copy;
            rows.row("Copy", |ui| {
                named(
                    ui,
                    egui::Checkbox::new(&mut copy, "Keep original"),
                    "transformCopy",
                );
            });
            (copy != p.copy).then_some(ModelingOptions::TransformCopy(copy))
        }
        _ => None,
    };
    if let Some(options) = options {
        app.dispatch(Action::ModelingOptionsChanged {
            tag: tool.tag.clone(),
            options,
        });
    }
}

fn extent(rows: &mut FeatureCardRows, p: &ExtrudeParams) -> Option<ModelingOptions> {
    // An existing ToFace/two-direction record keeps its authored targets.
    if p.two_directions || p.mode == ExtrudeMode::ToFace {
        return None;
    }
    let mut mode = p.mode;
    rows.row("Extent", |ui| {
        choice(
            ui,
            "extrude-extent",
            &mut mode,
            &[
                (ExtrudeMode::Blind, "Distance"),
                (ExtrudeMode::Symmetric, "Symmetric"),
                (ExtrudeMode::ThroughAll, "Through all"),
            ],
        );
    });
    (mode != p.mode).then_some(ModelingOptions::ExtrudeExtent(mode))
}

fn face_offset(rows: &mut FeatureCardRows, p: &OffsetFaceParams) -> Option<ModelingOptions> {
    if p.faces.len() != 1 || p.distance_type == OffsetDistanceType::Total {
        return None;
    }
    let mut mode = p.distance_type;
    rows.row("Distance type", |ui| {
        choice(
            ui,
            "offset-distance-type",
            &mut mode,
            &[
                (OffsetDistanceType::Offset, "Signed offset"),
                (OffsetDistanceType::Radius, "Cylinder radius"),
                (OffsetDistanceType::Diameter, "Cylinder diameter"),
            ],
        );
    });
    (mode != p.distance_type).then_some(ModelingOptions::FaceOffsetMode(mode))
}

fn pattern(
    rows: &mut FeatureCardRows,
    count: u32,
    direction: Vec3,
    origin: Vec3,
    fuse: bool,
    circular: bool,
) -> Option<ModelingOptions> {
    let (mut n, mut axis, mut point, mut joined) = (count, direction, origin, fuse);
    rows.row("Instances", |ui| {
        named(
            ui,
            egui::DragValue::new(&mut n).range(2..=128),
            "patternCount",
        );
    });
    rows.row(
        if circular {
            "Axis direction"
        } else {
            "Direction"
        },
        |ui| vector(ui, "patternDirection", &mut axis),
    );
    if circular {
        rows.row("Axis origin (mm)", |ui| vector(ui, "origin", &mut point));
    }
    rows.row("Result", |ui| {
        named(
            ui,
            egui::Checkbox::new(&mut joined, "Fuse connected instances"),
            "patternFuse",
        );
    });
    ((n, axis, point, joined) != (count, direction, origin, fuse)).then_some(
        ModelingOptions::Pattern {
            count: n,
            direction: axis,
            origin: point,
            fuse: joined,
        },
    )
}

fn mirror(rows: &mut FeatureCardRows, p: &MirrorBodyParams) -> Option<ModelingOptions> {
    let (mut point, mut normal, mut fuse) = (p.plane_point, p.plane_normal, p.fuse_with_original);
    rows.row("Plane point (mm)", |ui| vector(ui, "origin", &mut point));
    rows.row("Plane normal", |ui| vector(ui, "planeNormal", &mut normal));
    rows.row("Result", |ui| {
        named(
            ui,
            egui::Checkbox::new(&mut fuse, "Fuse with source"),
            "mirrorFuse",
        );
    });
    ((point, normal, fuse) != (p.plane_point, p.plane_normal, p.fuse_with_original)).then_some(
        ModelingOptions::Mirror {
            point,
            normal,
            fuse,
        },
    )
}

fn hole(rows: &mut FeatureCardRows, p: &HoleParams) -> Option<ModelingOptions> {
    if p.points.is_some() {
        return None;
    }
    let (mut kind, mut through_all, mut point) = (p.hole_type, p.depth.is_none(), p.point);
    rows.row("Hole type", |ui| {
        choice(
            ui,
            "hole-kind",
            &mut kind,
            &[
                (HoleType::Simple, "Simple"),
                (HoleType::Counterbore, "Counterbore"),
                (HoleType::Countersink, "Countersink"),
            ],
        )
    });
    rows.row("Extent", |ui| {
        named(
            ui,
            egui::Checkbox::new(&mut through_all, "Through all"),
            "holeThroughAll",
        );
    });
    rows.row("Center (world mm)", |ui| vector(ui, "origin", &mut point));
    ((kind, through_all, point) != (p.hole_type, p.depth.is_none(), p.point)).then_some(
        ModelingOptions::Hole {
            kind,
            through_all,
            point,
        },
    )
}

fn boolean(app: &App, rows: &mut FeatureCardRows, p: &BooleanParams) -> Option<ModelingOptions> {
    let (mut operation, mut target, mut keep) = (p.operation, p.target_body, p.keep_tools);
    rows.row("Operation", |ui| {
        choice(
            ui,
            "boolean-kind",
            &mut operation,
            &[
                (BooleanOp::Union, "Union"),
                (BooleanOp::Cut, "Subtract"),
                (BooleanOp::Intersect, "Intersect"),
            ],
        )
    });
    let bodies: Vec<_> = std::iter::once(p.target_body)
        .chain(p.tools.iter().copied())
        .collect();
    rows.row("Target", |ui| {
        egui::ComboBox::from_id_salt("boolean-target")
            .selected_text(body_name(app, target))
            .show_ui(ui, |ui| {
                for id in &bodies {
                    ui.selectable_value(&mut target, *id, body_name(app, *id));
                }
            });
    });
    rows.row("Tools", |ui| {
        ui.label(format!("{} selected bodies", p.tools.len()));
    });
    rows.row("Result", |ui| {
        named(
            ui,
            egui::Checkbox::new(&mut keep, "Keep tool bodies"),
            "booleanKeepTools",
        );
    });
    ((operation, target, keep) != (p.operation, p.target_body, p.keep_tools)).then(|| {
        ModelingOptions::Boolean {
            operation,
            target,
            tools: bodies.into_iter().filter(|id| *id != target).collect(),
            keep,
        }
    })
}

fn body_name(app: &App, id: BodyId) -> String {
    app.doc
        .display
        .as_ref()
        .and_then(|d| d.projection.bodies.get(&id.to_string()))
        .map_or_else(|| id.to_string(), |b| b.name.clone())
}

fn profile_boolean(
    app: &App,
    rows: &mut FeatureCardRows,
    mode: BooleanMode,
    target: Option<BodyId>,
) -> Option<ModelingOptions> {
    let (mut next_mode, mut next_target) = (mode, target);
    rows.row("Operation", |ui| {
        choice(
            ui,
            "profile-boolean",
            &mut next_mode,
            &[
                (BooleanMode::NewBody, "New body"),
                (BooleanMode::Add, "Add"),
                (BooleanMode::Cut, "Cut"),
                (BooleanMode::Intersect, "Intersect"),
            ],
        )
    });
    if next_mode != BooleanMode::NewBody {
        rows.row("Target body", |ui| {
            egui::ComboBox::from_id_salt("profile-target")
                .selected_text(next_target.map_or("Select body".into(), |id| body_name(app, id)))
                .show_ui(ui, |ui| {
                    for (id, name) in profile_targets(app) {
                        ui.selectable_value(&mut next_target, Some(id), name);
                    }
                });
        });
    }
    ((next_mode, next_target) != (mode, target)).then_some(ModelingOptions::ProfileBoolean {
        mode: next_mode,
        target: next_target,
    })
}

fn profile_targets(app: &App) -> Vec<(BodyId, String)> {
    if let Some(eligible) = crate::model_handle::eligible_bodies(app) {
        return eligible
            .iter()
            .map(|&id| (id, body_name(app, id)))
            .collect();
    }
    app.doc.display.as_ref().map_or_else(Vec::new, |display| {
        display
            .projection
            .bodies
            .values()
            .filter(|body| body.visible && body.health == BodyHealth::Healthy)
            .filter_map(|body| Some((body.id.parse().ok()?, body.name.clone())))
            .collect()
    })
}

fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, value: &mut T, choices: &[(T, &str)]) {
    let label = choices
        .iter()
        .find(|(item, _)| item == value)
        .map_or("Select", |(_, label)| *label);
    let response = egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .show_ui(ui, |ui| {
            for (item, label) in choices {
                ui.selectable_value(value, *item, *label);
            }
        })
        .response;
    crate::automation::name(ui.ctx(), response.id, &format!("onecad.option.{id}"));
}

fn vector(ui: &mut egui::Ui, id: &str, v: &mut Vec3) {
    ui.vertical(|ui| {
        for (label, value) in [("X", &mut v.x), ("Y", &mut v.y), ("Z", &mut v.z)] {
            ui.horizontal(|ui| {
                let label_id = ui.label(label).id;
                let response = vector_value(ui, value).labelled_by(label_id);
                crate::automation::name(
                    ui.ctx(),
                    response.id,
                    &format!("onecad.option.{id}{label}"),
                );
            });
        }
    });
}

fn vector_value(ui: &mut egui::Ui, value: &mut f64) -> egui::Response {
    let field_id = ui.next_auto_id();
    let ctx = ui.ctx().clone();
    ui.add(
        egui::DragValue::new(value)
            .speed(0.1)
            .custom_formatter(move |value, _| {
                if ctx.memory(|memory| memory.has_focus(field_id)) {
                    value.to_string()
                } else {
                    format!("{value:.2}")
                }
            }),
    )
}

fn gear(rows: &mut FeatureCardRows, p: &GearParams) -> Option<ModelingOptions> {
    let inv = p.involute()?;
    let (mut teeth, mut axle, mut offset, mut point) =
        (inv.teeth, inv.axle_hole, inv.offset_hole, p.placement.point);
    rows.row("Teeth", |ui| {
        named(
            ui,
            egui::DragValue::new(&mut teeth).range(3..=400),
            "gearTeeth",
        );
    });
    rows.row("Bores", |ui| {
        named(
            ui,
            egui::Checkbox::new(&mut axle, "Axle hole"),
            "gearAxleHole",
        );
        named(
            ui,
            egui::Checkbox::new(&mut offset, "Offset hole"),
            "gearOffsetHole",
        );
    });
    rows.row("Center (world mm)", |ui| vector(ui, "origin", &mut point));
    ((teeth, axle, offset, point) != (inv.teeth, inv.axle_hole, inv.offset_hole, p.placement.point))
        .then_some(ModelingOptions::Gear {
            teeth,
            axle_hole: axle,
            offset_hole: offset,
            point,
        })
}

fn named(ui: &mut egui::Ui, widget: impl egui::Widget, id: &str) {
    let response = ui.add(widget);
    crate::automation::name(ui.ctx(), response.id, &format!("onecad.option.{id}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_default_rounding_and_focus_blur_never_quantize_bound_values() {
        let ctx = egui::Context::default();
        let original = 50.00019918212638;
        let mut value = original;
        let mut id = None;
        for focused in [false, true, false, false] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                if let Some(id) = id {
                    ui.memory_mut(|memory| {
                        if focused {
                            memory.request_focus(id);
                        } else {
                            memory.surrender_focus(id);
                        }
                    });
                }
                let response = vector_value(ui, &mut value);
                id = Some(response.id);
                assert!(!response.changed());
            });
            out.textures_delta.clear();
            assert_eq!(value, original);
            let expected = if focused {
                original.to_string()
            } else {
                "50.00".into()
            };
            assert!(out.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.job.text == expected)
            }), "missing shown value: {expected}");
        }
    }

    #[test]
    fn pattern_controls_are_addressable_by_both_hosts() {
        let nodes = crate::automation::testing::author_ids(|ui| {
            FeatureCardRows::inline(ui, |rows| {
                pattern(
                    rows,
                    3,
                    Vec3::new_unchecked(0.0, 0.0, 1.0),
                    Vec3::new_unchecked(0.0, 0.0, 0.0),
                    false,
                    true,
                );
            });
        });
        for suffix in [
            "patternCount",
            "patternDirectionX",
            "patternDirectionY",
            "patternDirectionZ",
            "originX",
            "originY",
            "originZ",
            "patternFuse",
        ] {
            assert_eq!(
                nodes
                    .iter()
                    .filter(|(id, _, _)| id == &format!("onecad.option.{suffix}"))
                    .count(),
                1,
                "{suffix}: {nodes:?}"
            );
        }
    }
}
