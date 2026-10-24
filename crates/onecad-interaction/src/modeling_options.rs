//! Choice controls share the numeric candidate's sequence and preserve pending text.
use crate::{declared_keys, Effect, Interaction, ParameterSet, RequestTag};
use onecad_core::{
    document::{record::*, variables::Scalar},
    ids::BodyId,
    math::Vec3,
};

#[derive(Clone, Debug)]
pub enum ModelingOptions {
    Loft {
        profiles: Vec<onecad_core::document::refs::SketchRegionRef>,
        is_ruled: bool,
    },
    ExtrudeExtent(ExtrudeMode),
    FaceOffsetMode(OffsetDistanceType),
    ProfileBoolean {
        mode: BooleanMode,
        target: Option<BodyId>,
    },
    Pattern {
        count: u32,
        direction: Vec3,
        origin: Vec3,
        fuse: bool,
    },
    Boolean {
        operation: BooleanOp,
        target: BodyId,
        tools: Vec<BodyId>,
        keep: bool,
    },
    Mirror {
        point: Vec3,
        normal: Vec3,
        fuse: bool,
    },
    Hole {
        kind: HoleType,
        through_all: bool,
        point: Vec3,
    },
    TransformCopy(bool),
    RevolveAxis(Box<onecad_core::document::refs::AxisRef>),
    Gear {
        teeth: u32,
        axle_hole: bool,
        offset_hole: bool,
        point: Vec3,
    },
}

impl Interaction {
    pub(crate) fn change_modeling_options(
        &mut self,
        tag: RequestTag,
        options: ModelingOptions,
    ) -> Vec<Effect> {
        if self.context.as_ref() != Some(&tag.context) {
            return Vec::new();
        }
        let Some(tool) = self.editing_tool().filter(|tool| tool.tag == tag) else {
            return Vec::new();
        };
        let preserve_drag = matches!(&options, ModelingOptions::Mirror { .. });
        let mut operation = tool.effective_op();
        if let Err(message) = apply(&mut operation, options) {
            tool.params.op_error = Some((None, message));
            tool.preview = crate::PreviewStatus::Empty;
            return vec![Effect::ClearPreview {
                gesture: tag.gesture,
            }];
        }
        if operation == tool.record.op {
            return Vec::new();
        }
        let mut keys = declared_keys(&operation);
        let focus = tool
            .params
            .focus()
            .filter(|key| keys.contains(key))
            .or(keys.first().copied());
        for field in tool.params.fields() {
            if !keys.contains(&field.key)
                && field.key.parameter().registered_scalar(&operation).is_ok()
            {
                keys.push(field.key);
            }
        }
        let Ok(mut params) = ParameterSet::declare(&operation, &keys, focus) else {
            return Vec::new();
        };
        // Existing fields retain even invalid/pending expressions. Only newly
        // introduced fields get defaults; removed optional fields stay removed.
        for field in &mut params.fields {
            if let Some(previous) = tool.params.field(field.key) {
                let active = field.active;
                *field = previous.clone();
                field.active = active;
            }
        }
        tool.params = params;
        tool.record.op = operation;
        tool.record.inputs = tool.record.op.derive_inputs();
        if !preserve_drag {
            tool.drag = None;
        }
        self.changed(None)
    }
}

fn apply(operation: &mut Operation, options: ModelingOptions) -> Result<(), String> {
    let Operation::Known(op) = operation else {
        return Err("Unknown operation".into());
    };
    match (op, options) {
        (KnownOperation::Loft(p), ModelingOptions::Loft { profiles, is_ruled }) => {
            p.profiles = profiles;
            p.is_ruled = is_ruled;
        }
        (KnownOperation::OffsetFace(p), ModelingOptions::FaceOffsetMode(mode)) => {
            if mode == OffsetDistanceType::Total {
                return Err("Total wall thickness requires a staged opposite face".into());
            }
            p.distance_type = mode;
            p.opposite_face = None;
            p.opposite_face_id = None;
        }
        (KnownOperation::Extrude(p), ModelingOptions::ExtrudeExtent(mode)) => {
            if !matches!(
                mode,
                ExtrudeMode::Blind | ExtrudeMode::Symmetric | ExtrudeMode::ThroughAll
            ) {
                return Err("To face requires a staged target face".into());
            }
            p.mode = mode;
            p.two_directions = false;
            p.target_face = None;
        }
        (KnownOperation::Extrude(p), ModelingOptions::ProfileBoolean { mode, target }) => {
            p.boolean_mode = mode;
            p.target_body = if mode == BooleanMode::NewBody {
                None
            } else {
                target
            };
        }
        (KnownOperation::Revolve(p), ModelingOptions::ProfileBoolean { mode, target }) => {
            p.boolean_mode = mode;
            p.target_body = if mode == BooleanMode::NewBody {
                None
            } else {
                target
            };
        }
        (
            KnownOperation::LinearPattern(p),
            ModelingOptions::Pattern {
                count,
                direction,
                fuse,
                ..
            },
        ) => {
            p.count = count;
            p.direction = direction;
            p.fuse_result = fuse;
        }
        (
            KnownOperation::CircularPattern(p),
            ModelingOptions::Pattern {
                count,
                direction,
                origin,
                fuse,
            },
        ) => {
            p.count = count;
            p.axis_direction = direction;
            p.axis_origin = origin;
            p.fuse_result = fuse;
        }
        (
            KnownOperation::Boolean(p),
            ModelingOptions::Boolean {
                operation,
                target,
                tools,
                keep,
            },
        ) => {
            p.operation = operation;
            p.target_body = target;
            p.tools = tools;
            p.keep_tools = keep;
        }
        (
            KnownOperation::MirrorBody(p),
            ModelingOptions::Mirror {
                point,
                normal,
                fuse,
            },
        ) => {
            p.plane_point = point;
            p.plane_normal = normal;
            p.fuse_with_original = fuse;
        }
        (
            KnownOperation::Hole(p),
            ModelingOptions::Hole {
                kind,
                through_all,
                point,
            },
        ) => {
            p.hole_type = kind;
            p.point = point;
            p.depth = if through_all {
                None
            } else {
                Some(p.depth.clone().unwrap_or(Scalar::new(10.0)))
            };
            p.cb_diameter = optional(kind == HoleType::Counterbore, p.cb_diameter.take(), 8.0);
            p.cb_depth = optional(kind == HoleType::Counterbore, p.cb_depth.take(), 2.0);
            p.cs_diameter = optional(kind == HoleType::Countersink, p.cs_diameter.take(), 8.0);
            p.cs_angle_deg = optional(kind == HoleType::Countersink, p.cs_angle_deg.take(), 90.0);
        }
        (KnownOperation::TransformBody(p), ModelingOptions::TransformCopy(copy)) => p.copy = copy,
        (KnownOperation::Revolve(p), ModelingOptions::RevolveAxis(axis)) => p.axis = Some(*axis),
        (
            KnownOperation::Gear(p),
            ModelingOptions::Gear {
                teeth,
                axle_hole,
                offset_hole,
                point,
            },
        ) => {
            let inv = p.involute_external.as_mut().ok_or("Missing gear recipe")?;
            inv.teeth = teeth;
            inv.axle_hole = axle_hole;
            inv.axle_hole_diameter = optional(axle_hole, inv.axle_hole_diameter.take(), 5.0);
            inv.offset_hole = offset_hole;
            inv.offset_hole_diameter = optional(offset_hole, inv.offset_hole_diameter.take(), 3.0);
            inv.offset_hole_offset = optional(offset_hole, inv.offset_hole_offset.take(), 10.0);
            p.placement.point = point;
            if let Some(frame) = &mut p.placement.frame {
                frame.origin = point;
            }
        }
        _ => return Err("Control does not belong to the current operation".into()),
    }
    Ok(())
}

fn optional(enabled: bool, current: Option<Scalar>, default: f64) -> Option<Scalar> {
    enabled.then(|| current.unwrap_or(Scalar::new(default)))
}

pub(crate) fn validate(op: &KnownOperation) -> Result<(), String> {
    match op {
        KnownOperation::LinearPattern(p) => {
            pattern(p.source_body, p.count, p.direction)?;
            if !p.spacing.value.is_finite() || p.spacing.value.abs() < 1e-9 {
                return Err("Pattern spacing must be nonzero and finite".into());
            }
        }
        KnownOperation::CircularPattern(p) => {
            pattern(p.source_body, p.count, p.axis_direction)?;
            if !p.axis_origin.is_finite() || !(1.0..=360.0).contains(&p.angle_deg.value) {
                return Err(
                    "Circular pattern requires a finite origin and angle in 1–360 degrees".into(),
                );
            }
        }
        KnownOperation::MirrorBody(p) => {
            if p.source_body.is_none() || !p.plane_point.is_finite() || !direction(p.plane_normal) {
                return Err(
                    "Mirror requires a source body and a finite plane with a nonzero normal".into(),
                );
            }
        }
        KnownOperation::Boolean(p) => p.validate()?,
        _ => {}
    }
    Ok(())
}

fn pattern(body: Option<BodyId>, count: u32, axis: Vec3) -> Result<(), String> {
    if body.is_none() || !(2..=128).contains(&count) || !direction(axis) {
        return Err(
            "Pattern requires a body, 2–128 instances and a nonzero finite direction".into(),
        );
    }
    Ok(())
}

fn direction(v: Vec3) -> bool {
    v.is_finite() && v.x.abs().max(v.y.abs()).max(v.z.abs()) > 1e-9
}
