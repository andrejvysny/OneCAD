use super::{Geometry, Kind, ModelHandle, Target, anchors::*, operands::Operands};
use crate::app::App;
use onecad_core::document::record::{
    GearParams, KnownOperation as Op, MirrorBodyParams, Operation, RevolveParams,
};
use onecad_interaction::{ModelSession, NumericParameter as N, ParameterKey, SessionPhase};

pub(super) fn targets(app: &App) -> Vec<Target> {
    if !app.viewport_ready()
        || !app.doc.bridge.valid()
        || app.sketch_active()
        || crate::viewport_selection::sectioned(app)
    {
        return vec![];
    }
    let Some(tool) = app
        .doc
        .interaction
        .tool
        .as_ref()
        .filter(|t| t.phase == SessionPhase::Editing)
    else {
        return vec![];
    };
    if tool.editing_existing && !super::history::ready(&app.doc.model_handle, tool) {
        return vec![];
    }
    let Some(display) = app.doc.display.as_ref() else {
        return vec![];
    };
    for_tool(
        tool,
        &Context {
            scene: &display.scene,
            records: &display.records,
            profiles: &display.profiles,
            profile_anchors: &display.profile_anchors,
        },
        &app.doc.model_handle,
    )
}
pub(super) fn for_tool(
    tool: &ModelSession,
    context: &Context<'_>,
    handles: &ModelHandle,
) -> Vec<Target> {
    let Operation::Known(op) = tool.effective_op() else {
        return vec![];
    };
    let operands = handles.operands(tool);
    if let Op::MirrorBody(p) = &op {
        return mirror(p, tool, context, operands).into_iter().collect();
    }
    geometries(&op, context, operands)
        .into_iter()
        .filter_map(|(parameter, geometry)| {
            let key = ParameterKey::Scalar(parameter);
            let field = tool.params.field(key).filter(|f| f.active)?;
            let value = tool
                .value(key)
                .or_else(|| handles.value(key))
                .filter(|v| v.is_finite())?;
            Some(Target {
                kind: Kind::Scalar(key),
                geometry,
                value,
                original: field.text.clone(),
                caption: match &op {
                    Op::OffsetFace(p) => Some(match p.distance_type {
                        onecad_core::document::record::OffsetDistanceType::Radius => {
                            "Cylinder radius"
                        }
                        onecad_core::document::record::OffsetDistanceType::Diameter => {
                            "Cylinder diameter"
                        }
                        _ => "Offset face distance",
                    }),
                    _ => None,
                },
                record: tool.tag.record_id,
            })
        })
        .collect()
}
fn geometries(op: &Op, context: &Context<'_>, operands: Option<&Operands>) -> Vec<(N, Geometry)> {
    match op {
        Op::Revolve(p) => one(N::RevolveAngle, revolve(p, context, operands)),
        Op::Fillet(_) | Op::Chamfer(_) => edge_tool(op, context, operands),
        Op::Shell(_) | Op::OffsetFace(_) | Op::Hole(_) => face_tool(op, context, operands),
        Op::LinearPattern(_) | Op::CircularPattern(_) => pattern_tool(op, context, operands),
        Op::Gear(p) => gear(p, context, operands),
        _ => vec![],
    }
}
fn one(parameter: N, geometry: Option<Geometry>) -> Vec<(N, Geometry)> {
    geometry.map(|g| vec![(parameter, g)]).unwrap_or_default()
}
fn edge_tool(op: &Op, context: &Context<'_>, operands: Option<&Operands>) -> Vec<(N, Geometry)> {
    let (parameter, reference) = match op {
        Op::Fillet(p) => (N::FilletRadius, p.edges.first()),
        Op::Chamfer(p) => (N::ChamferDistance, p.edges.first()),
        _ => return vec![],
    };
    one(
        parameter,
        operands
            .map_or_else(|| reference.and_then(|r| edge(context, r)), |o| o.edge)
            .map(|(base, direction)| Geometry::axis(base, direction, 1.0)),
    )
}
fn face_tool(op: &Op, context: &Context<'_>, operands: Option<&Operands>) -> Vec<(N, Geometry)> {
    if let Op::OffsetFace(p) = op {
        if let Some(surface) = operands.and_then(|o| o.surface) {
            return one(N::OffsetFaceDistance, surface.geometry(p.distance_type));
        }
        if p.distance_type != onecad_core::document::record::OffsetDistanceType::Offset {
            return vec![];
        }
    }
    let (parameter, reference, inward) = match op {
        Op::Shell(p) => (N::ShellThickness, p.faces.first(), true),
        Op::OffsetFace(p) => (N::OffsetFaceDistance, p.faces.first(), false),
        Op::Hole(p) => (N::HoleDiameter, Some(&p.face), true),
        _ => return vec![],
    };
    let Some((base, normal)) =
        operands.map_or_else(|| reference.and_then(|r| face_ref(context, r)), |o| o.face)
    else {
        return vec![];
    };
    if let Op::Hole(p) = op {
        let point = vector(p.point);
        return vec![
            (
                N::HoleDiameter,
                Geometry::axis(point, normal.any_orthonormal_vector(), 0.5),
            ),
            (N::HoleDepth, Geometry::axis(point, -normal, 1.0)),
        ];
    }
    one(
        parameter,
        Some(Geometry::axis(
            base,
            if inward { -normal } else { normal },
            1.0,
        )),
    )
}
fn pattern_tool(op: &Op, context: &Context<'_>, operands: Option<&Operands>) -> Vec<(N, Geometry)> {
    match op {
        Op::LinearPattern(p) => one(
            N::LinearPatternSpacing,
            p.source_body.and_then(|id| {
                Some(Geometry::axis(
                    operand_center(context, operands, id)?,
                    vector(p.direction).try_normalize()?,
                    1.0,
                ))
            }),
        ),
        Op::CircularPattern(p) => one(
            N::CircularPatternAngle,
            p.source_body.and_then(|id| {
                let origin = vector(p.axis_origin);
                let direction = vector(p.axis_direction).try_normalize()?;
                Geometry::ring(origin, direction, operand_center(context, operands, id)?).or_else(
                    || {
                        operand_bounds(context, operands, id)?
                            .corners()
                            .find_map(|seed| Geometry::ring(origin, direction, seed))
                    },
                )
            }),
        ),
        _ => vec![],
    }
}
fn revolve(
    p: &RevolveParams,
    context: &Context<'_>,
    operands: Option<&Operands>,
) -> Option<Geometry> {
    let wanted = p.profile.as_ref()?;
    let (origin, direction) = match p.axis.as_ref()? {
        onecad_core::document::refs::AxisRef::SketchLine { .. } => axis(context, p.axis.as_ref()?),
        _ => operands.map_or_else(|| axis(context, p.axis.as_ref()?), |o| o.axis),
    }?;
    if let Some(profile) = context.profiles.iter().find(|profile| {
        profile.reference.sketch == wanted.sketch && profile.reference.region == wanted.region
    }) {
        return Geometry::ring(origin, direction, profile.centroid()).or_else(|| {
            profile
                .vertices
                .iter()
                .find_map(|&p| Geometry::ring(origin, direction, p))
        });
    }
    let anchor = context
        .profile_anchors
        .iter()
        .find(|a| a.reference.sketch == wanted.sketch && a.reference.region == wanted.region)?;
    Geometry::ring(origin, direction, anchor.base).or_else(|| {
        anchor
            .triangle
            .iter()
            .find_map(|&p| Geometry::ring(origin, direction, p))
    })
}
fn gear(p: &GearParams, context: &Context<'_>, operands: Option<&Operands>) -> Vec<(N, Geometry)> {
    let Some(recipe) = p.involute() else {
        return vec![];
    };
    let frame = p
        .placement
        .frame
        .as_ref()
        .map(|f| (vector(f.axis), vector(f.x_dir)))
        .or_else(|| {
            operands
                .map_or_else(|| face_ref(context, p.placement.face.as_ref()?), |o| o.face)
                .map(|(_, normal)| (-normal, normal.any_orthonormal_vector()))
        });
    let Some((normal, radial)) = frame else {
        return vec![];
    };
    let point = vector(p.placement.point);
    vec![
        (N::GearHeight, Geometry::axis(point, normal, 1.0)),
        (
            N::GearModule,
            Geometry::axis(point, radial, f64::from(recipe.teeth) * 0.5),
        ),
    ]
}
fn mirror(
    p: &MirrorBodyParams,
    tool: &ModelSession,
    context: &Context<'_>,
    operands: Option<&Operands>,
) -> Option<Target> {
    let seed = operand_center(context, operands, p.source_body?)?;
    let normal = vector(p.plane_normal).try_normalize()?;
    let point = vector(p.plane_point);
    let base = seed - normal * (seed - point).dot(normal);
    Some(Target {
        kind: Kind::Mirror(p.clone()),
        geometry: Geometry::axis(base, normal, 1.0),
        value: 0.0,
        original: String::new(),
        caption: None,
        record: tool.tag.record_id,
    })
}

fn operand_bounds(
    context: &Context<'_>,
    operands: Option<&Operands>,
    body_id: onecad_core::ids::BodyId,
) -> Option<onecad_scene::Bounds> {
    operands.map_or_else(
        || Some(body(context, body_id)?.bounds()),
        |o| o.bounds(body_id),
    )
}
fn operand_center(
    context: &Context<'_>,
    operands: Option<&Operands>,
    body_id: onecad_core::ids::BodyId,
) -> Option<onecad_scene::DVec3> {
    let bounds = operand_bounds(context, operands, body_id)?;
    Some((bounds.min + bounds.max) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_offset_dimensions_without_analytic_surface_never_use_generic_face_scale() {
        let scene = super::super::test_support::scene();
        let record = super::super::test_support::records(&scene)
            .into_iter()
            .find_map(|(record, _)| match record.op {
                Operation::Known(Op::OffsetFace(p)) => Some(p),
                _ => None,
            })
            .unwrap();
        let context = Context {
            scene: &scene,
            records: &[],
            profiles: &[],
            profile_anchors: &[],
        };
        for distance_type in [
            onecad_core::document::record::OffsetDistanceType::Radius,
            onecad_core::document::record::OffsetDistanceType::Diameter,
            onecad_core::document::record::OffsetDistanceType::Total,
        ] {
            let mut params = record.clone();
            params.distance_type = distance_type;
            assert!(face_tool(&Op::OffsetFace(params), &context, None).is_empty());
        }
    }
}
