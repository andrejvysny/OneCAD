//! Typed new-operation defaults. Geometry operands are staged before binding.
use onecad_core::{
    document::{
        record::*,
        refs::{AxisRef, ElementKind, ElementRef, SketchRegionRef},
        variables::Scalar,
    },
    ids::{BodyId, RecordId},
    math::Vec3,
};

pub fn shell(refs: Vec<ElementRef>, id: RecordId, step: u32) -> Result<OperationRecord, String> {
    let body = refs
        .first()
        .and_then(|r| r.primary.as_ref())
        .ok_or("Select open faces")?
        .body;
    let open_faces = crate::operand_binding::persistent_ids(&refs, ElementKind::Face, body)?;
    Ok(record(
        id,
        step,
        "Shell",
        KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(1.0),
            open_faces,
            faces: refs,
            target_body: Some(body),
            extra: Default::default(),
        }),
    ))
}

pub fn gear(id: RecordId, step: u32) -> OperationRecord {
    let origin = Vec3::new_unchecked(0.0, 0.0, 0.0);
    record(
        id,
        step,
        "Gear",
        KnownOperation::Gear(GearParams {
            recipe: GearRecipe::InvoluteExternal,
            placement: GearPlacement {
                face: None,
                frame: Some(GearFrame {
                    origin,
                    axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
                    x_dir: Vec3::new_unchecked(1.0, 0.0, 0.0),
                }),
                point: origin,
            },
            involute_external: Some(InvoluteExternalParams {
                teeth: 20,
                module: Scalar::new(2.0),
                height: Scalar::new(5.0),
                pressure_angle_deg: Scalar::new(20.0),
                shift: 0.0,
                helix_angle_deg: 0.0,
                double_helix: false,
                properties_from_tool: false,
                undercut: false,
                backlash: 0.0,
                clearance: 0.25,
                head: 0.0,
                sample_count: 20,
                axle_hole: false,
                axle_hole_diameter: None,
                offset_hole: false,
                offset_hole_diameter: None,
                offset_hole_offset: None,
            }),
            extra: Default::default(),
        }),
    )
}

pub fn hole(
    face: ElementRef,
    point: Vec3,
    id: RecordId,
    step: u32,
) -> Result<OperationRecord, String> {
    let body = face.primary.as_ref().ok_or("Select a planar face")?.body;
    crate::operand_binding::persistent_ids(std::slice::from_ref(&face), ElementKind::Face, body)?;
    Ok(record(
        id,
        step,
        "Hole",
        KnownOperation::Hole(HoleParams {
            target_body: body,
            face,
            point,
            points: None,
            hole_type: HoleType::Simple,
            diameter: Scalar::new(4.0),
            depth: None,
            cb_diameter: None,
            cb_depth: None,
            cs_diameter: None,
            cs_angle_deg: None,
            thread: None,
            result_policy_version: Some(2),
            extra: Default::default(),
        }),
    ))
}

pub fn revolve(
    profile: SketchRegionRef,
    edge: ElementRef,
    id: RecordId,
    step: u32,
) -> Result<OperationRecord, String> {
    let primary = edge.primary.as_ref().ok_or("Select an axis edge")?;
    crate::operand_binding::persistent_ids(
        std::slice::from_ref(&edge),
        ElementKind::Edge,
        primary.body,
    )?;
    let axis = AxisRef::Element {
        body: primary.body,
        edge: primary.element.clone(),
        edge_ref: Some(edge),
        extra: Default::default(),
    };
    Ok(revolve_axis(profile, axis, id, step))
}

pub fn revolve_axis(
    profile: SketchRegionRef,
    axis: AxisRef,
    id: RecordId,
    step: u32,
) -> OperationRecord {
    record(
        id,
        step,
        "Revolve",
        KnownOperation::Revolve(RevolveParams {
            profile: Some(profile),
            axis: Some(axis),
            angle_deg: Scalar::new(360.0),
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            extra: Default::default(),
        }),
    )
}

pub fn linear_pattern(body: BodyId, spacing: f64, id: RecordId, step: u32) -> OperationRecord {
    record(
        id,
        step,
        "Linear pattern",
        KnownOperation::LinearPattern(LinearPatternParams {
            source_body: Some(body),
            direction: Vec3::new_unchecked(1.0, 0.0, 0.0),
            spacing: Scalar::new(spacing),
            count: 2,
            fuse_result: false,
            result_policy_version: Some(2),
            extra: Default::default(),
        }),
    )
}

pub fn circular_pattern(body: BodyId, id: RecordId, step: u32) -> OperationRecord {
    record(
        id,
        step,
        "Circular pattern",
        KnownOperation::CircularPattern(CircularPatternParams {
            source_body: Some(body),
            axis_origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            axis_direction: Vec3::new_unchecked(0.0, 0.0, 1.0),
            angle_deg: Scalar::new(360.0),
            count: 3,
            fuse_result: false,
            result_policy_version: Some(2),
            extra: Default::default(),
        }),
    )
}

pub fn mirror(body: BodyId, id: RecordId, step: u32) -> OperationRecord {
    record(
        id,
        step,
        "Mirror body",
        KnownOperation::MirrorBody(MirrorBodyParams {
            source_body: Some(body),
            plane_point: Vec3::new_unchecked(0.0, 0.0, 0.0),
            plane_normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
            fuse_with_original: false,
            extra: Default::default(),
        }),
    )
}

pub fn boolean(bodies: &[BodyId], id: RecordId, step: u32) -> Result<OperationRecord, String> {
    let (target, tools) = bodies
        .split_first()
        .ok_or("Select target and tool bodies")?;
    let params = BooleanParams::new(BooleanOp::Union, *target, tools.to_vec(), false);
    params.validate()?;
    Ok(record(id, step, "Boolean", KnownOperation::Boolean(params)))
}

fn record(id: RecordId, step: u32, name: &str, op: KnownOperation) -> OperationRecord {
    OperationRecord::new(id, step, name, Operation::Known(op))
}
