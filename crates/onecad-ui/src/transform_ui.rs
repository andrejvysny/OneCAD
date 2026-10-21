//! SolidCraft's triad edits OneCAD typed drafts; only explicit Apply authors history.
use crate::{
    app::App,
    gizmo::{Change, Gizmo, Triad},
};
use onecad_core::{
    document::{
        record::{
            KnownOperation, Operation, OperationRecord, TransformBodyParams, TransformRotation,
        },
        variables::Scalar,
    },
    ids::RecordId,
    math::Vec3,
};
use onecad_interaction::{Action, DraftStatus, NumericParameter, ParameterAxis, SessionPhase};
use onecad_scene::{DVec3, ViewSnapshot};

#[derive(Default)]
pub struct TransformGizmo {
    pub gizmo: Gizmo,
    baseline: Option<TransformBodyParams>,
    record: Option<RecordId>,
}

impl App {
    pub fn begin_transform(&mut self) {
        let Some((body, context, step, center)) = self.display.as_ref().and_then(|display| {
            let body = self.selected_body?;
            let bounds = display
                .scene
                .bodies()
                .iter()
                .find(|item| item.body_id() == body)?
                .bounds();
            Some((
                body,
                self.interaction.context.clone()?,
                display.projection.applied_ops as u32,
                (bounds.min + bounds.max) * 0.5,
            ))
        }) else {
            return;
        };
        if !self.viewport_ready()
            || !self.bridge.valid()
            || self.busy()
            || self.sketch_active()
            || crate::viewport_selection::sectioned(self)
        {
            return;
        }
        self.gesture += 1;
        self.transform_gizmo = TransformGizmo::default();
        let params = TransformBodyParams {
            targets: vec![body],
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: TransformRotation {
                center: Vec3::new_unchecked(center.x, center.y, center.z),
                ..Default::default()
            },
            copy: false,
            extra: Default::default(),
        };
        let record = OperationRecord::new(
            RecordId::new(),
            step,
            "Move / Rotate",
            Operation::Known(KnownOperation::TransformBody(params)),
        );
        self.dispatch(Action::BeginOperation {
            record: Box::new(record),
            parameter: NumericParameter::TransformTranslation(ParameterAxis::X),
            gesture: self.gesture,
            context,
        });
    }
}

fn parameters(app: &App) -> Option<(RecordId, TransformBodyParams)> {
    if crate::viewport_selection::sectioned(app) {
        return None;
    }
    let tool = app.interaction.tool.as_ref()?;
    if tool.phase != SessionPhase::Editing || !app.viewport_ready() || !app.bridge.valid() {
        return None;
    }
    let Operation::Known(KnownOperation::TransformBody(mut params)) = tool.record.op.clone() else {
        return None;
    };
    if let DraftStatus::Valid(scalar) = &tool.draft_status {
        match tool.parameter {
            NumericParameter::TransformTranslation(axis) => {
                let index = match axis {
                    ParameterAxis::X => 0,
                    ParameterAxis::Y => 1,
                    ParameterAxis::Z => 2,
                };
                params.translate[index] = scalar.clone();
            }
            NumericParameter::TransformRotationAngle => params.rotate.angle_deg = scalar.clone(),
            _ => return None,
        }
    }
    let display = app.display.as_ref()?;
    if params.targets.iter().any(|id| {
        !display
            .scene
            .bodies()
            .iter()
            .any(|body| body.body_id() == *id)
    }) {
        return None;
    }
    Some((tool.tag.record_id, params))
}

pub fn triad(app: &App) -> Option<Triad> {
    let (id, current) = parameters(app)?;
    let params = app
        .transform_gizmo
        .baseline
        .as_ref()
        .filter(|_| app.transform_gizmo.gizmo.active() && app.transform_gizmo.record == Some(id))
        .unwrap_or(&current);
    Some(Triad {
        center: DVec3::new(
            params.rotate.center.x,
            params.rotate.center.y,
            params.rotate.center.z,
        ) + DVec3::from_array(params.translate.each_ref().map(|value| value.value)),
        translate: true,
        rotate: true,
    })
}

pub fn owns_pointer(app: &App, view: &ViewSnapshot, rect: egui::Rect, pointer: egui::Pos2) -> bool {
    app.transform_gizmo.gizmo.active()
        || triad(app).is_some_and(|triad| Gizmo::contains_pointer(view, rect, triad, pointer))
}

pub fn show(app: &mut App, ui: &mut egui::Ui, view: &ViewSnapshot, rect: egui::Rect) -> bool {
    let Some((record, params)) = parameters(app) else {
        app.transform_gizmo = TransformGizmo::default();
        return false;
    };
    if app.transform_gizmo.record != Some(record) {
        app.transform_gizmo = TransformGizmo {
            record: Some(record),
            ..Default::default()
        };
    }
    let Some(triad) = triad(app) else {
        return false;
    };
    let step = snap_step(view.world_per_pixel(view.depth(triad.center)));
    let response = app
        .transform_gizmo
        .gizmo
        .show(ui, view, rect, triad, step, &app.theme);
    if let Some(drag) = response.drag {
        if drag.started {
            app.transform_gizmo.baseline = Some(params);
        }
        if let Some(base) = &app.transform_gizmo.baseline
            && let Some((translate, axis, angle_degrees)) = changed(base, drag.change)
        {
            app.dispatch(Action::TransformDraftChanged {
                translate,
                axis,
                angle_degrees,
            });
            ui.ctx().request_repaint();
        }
        if drag.done {
            app.transform_gizmo.baseline = None;
        }
    } else if !app.transform_gizmo.gizmo.active() {
        app.transform_gizmo.baseline = None;
    }
    response.owns_pointer
}

pub(crate) fn snap_step(world_per_pixel: f64) -> f64 {
    let desired = world_per_pixel * 10.0;
    if !desired.is_finite() || desired <= 0.0 {
        return 1.0;
    }
    10_f64.powf(desired.log10().floor())
}

fn changed(base: &TransformBodyParams, change: Change) -> Option<([f64; 3], [f64; 3], f64)> {
    let mut translation = base.translate.each_ref().map(|value| value.value);
    let mut axis = [base.rotate.axis.x, base.rotate.axis.y, base.rotate.axis.z];
    let mut angle = base.rotate.angle_deg.value;
    match change {
        Change::Translate(delta) => {
            translation = (DVec3::from_array(translation) + delta).to_array();
        }
        Change::Rotate {
            axis: index,
            angle: delta,
        } => {
            let direction = [DVec3::X, DVec3::Y, DVec3::Z].get(index).copied()?;
            let original = if angle == 0.0 {
                glam::DQuat::IDENTITY
            } else {
                glam::DQuat::from_axis_angle(
                    DVec3::from_array(axis).try_normalize()?,
                    angle.to_radians(),
                )
            };
            let rotation = glam::DQuat::from_axis_angle(direction, delta) * original;
            let (direction, radians) = rotation.normalize().to_axis_angle();
            axis = direction.to_array();
            angle = radians.to_degrees();
        }
    }
    (translation.iter().chain(&axis).all(|v| v.is_finite()) && angle.is_finite()).then_some((
        translation,
        axis,
        angle,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> TransformBodyParams {
        TransformBodyParams {
            targets: vec![onecad_core::ids::BodyId::new()],
            translate: [Scalar::new(2.0), Scalar::new(3.0), Scalar::new(4.0)],
            rotate: Default::default(),
            copy: false,
            extra: Default::default(),
        }
    }
    #[test]
    fn drag_deltas_always_use_frozen_baseline() {
        let base = base();
        assert_eq!(
            changed(&base, Change::Translate(DVec3::X)).unwrap().0,
            [3.0, 3.0, 4.0]
        );
        assert_eq!(
            changed(&base, Change::Translate(DVec3::X * 2.0)).unwrap().0,
            [4.0, 3.0, 4.0]
        );
        assert!(changed(&base, Change::Translate(DVec3::splat(f64::NAN))).is_none());
    }
    #[test]
    fn ring_composes_with_existing_rotation_in_degrees() {
        let mut base = base();
        base.rotate.angle_deg = Scalar::new(90.0);
        let (_, axis, angle) = changed(
            &base,
            Change::Rotate {
                axis: 0,
                angle: std::f64::consts::FRAC_PI_2,
            },
        )
        .unwrap();
        let actual = glam::DQuat::from_axis_angle(DVec3::from_array(axis), angle.to_radians());
        let expected = glam::DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2)
            * glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
        assert!((actual * DVec3::X - expected * DVec3::X).length() < 1e-12);
    }
}
