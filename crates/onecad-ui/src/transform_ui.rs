//! SolidCraft's triad edits OneCAD typed drafts; only explicit Apply authors history.
use crate::{
    app::App,
    gizmo::{Change, Drag, Gizmo, Triad},
    parameter_drag::{Lifecycle, Outcome},
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
use onecad_interaction::{Action, NumericParameter, ParameterAxis, ParameterKey, SessionPhase};
use onecad_scene::{DVec3, ViewSnapshot};

#[derive(Default)]
pub struct TransformGizmo {
    pub gizmo: Gizmo,
    baseline: Option<TransformBodyParams>,
    record: Option<RecordId>,
    lifecycle: Lifecycle,
    /// The last authored placement differs from the frozen baseline.
    moved: bool,
}

impl App {
    pub fn begin_transform(&mut self) {
        let Some((body, context, step, center)) = self.doc.display.as_ref().and_then(|display| {
            let body = self.doc.selected_body?;
            let bounds = display
                .scene
                .bodies()
                .iter()
                .find(|item| item.body_id() == body)?
                .bounds();
            Some((
                body,
                self.doc.interaction.context.clone()?,
                display.projection.applied_ops as u32,
                (bounds.min + bounds.max) * 0.5,
            ))
        }) else {
            return;
        };
        if !self.viewport_ready()
            || !self.doc.bridge.valid()
            || self.busy()
            || self.sketch_active()
            || crate::viewport_selection::sectioned(self)
        {
            return;
        }
        self.doc.gesture += 1;
        self.doc.transform_gizmo = TransformGizmo::default();
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
        let x = ParameterKey::from(NumericParameter::TransformTranslation(ParameterAxis::X));
        self.dispatch(Action::BeginOperation {
            record: Box::new(record),
            fields: vec![x],
            focus: Some(x),
            gesture: self.doc.gesture,
            context,
        });
    }
}

fn parameters(app: &App) -> Option<(RecordId, TransformBodyParams)> {
    if crate::viewport_selection::sectioned(app) {
        return None;
    }
    let tool = app.doc.interaction.tool.as_ref()?;
    if tool.phase != SessionPhase::Editing || !app.viewport_ready() || !app.doc.bridge.valid() {
        return None;
    }
    let Operation::Known(KnownOperation::TransformBody(params)) = tool.effective_op() else {
        return None;
    };
    let display = app.doc.display.as_ref()?;
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
        .doc
        .transform_gizmo
        .baseline
        .as_ref()
        .filter(|_| {
            app.doc.transform_gizmo.gizmo.active() && app.doc.transform_gizmo.record == Some(id)
        })
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
    app.doc.transform_gizmo.gizmo.active()
        || triad(app).is_some_and(|triad| Gizmo::contains_pointer(view, rect, triad, pointer))
}

pub fn show(app: &mut App, ui: &mut egui::Ui, view: &ViewSnapshot, rect: egui::Rect) -> bool {
    let Some((record, params)) = parameters(app) else {
        abandon(app, Outcome::Lost);
        app.doc.transform_gizmo = TransformGizmo::default();
        return false;
    };
    if app.doc.transform_gizmo.record != Some(record) {
        app.doc.transform_gizmo = TransformGizmo {
            record: Some(record),
            ..Default::default()
        };
    }
    let Some(triad) = triad(app) else {
        return false;
    };
    let step = snap_step(view.world_per_pixel(view.depth(triad.center)));
    let response = app
        .doc
        .transform_gizmo
        .gizmo
        .show(ui, view, rect, triad, step, &app.theme);
    // An earlier release in this batch ends its capture before the next one starts.
    for drag in response.finished.into_iter().chain(response.drag) {
        apply(app, drag, &params);
        ui.ctx().request_repaint();
    }
    if response.cancelled {
        abandon(app, Outcome::Lost);
    } else if !app.doc.transform_gizmo.gizmo.active() {
        app.doc.transform_gizmo.baseline = None;
    }
    response.owns_pointer
}

/// One capture's motion against its frozen baseline; nothing is authored
/// until the placement changes.
fn apply(app: &mut App, drag: Drag, params: &TransformBodyParams) {
    let gizmo = &mut app.doc.transform_gizmo;
    if drag.started {
        gizmo.baseline = Some(params.clone());
        gizmo.lifecycle = Lifecycle::default();
        gizmo.moved = false;
    }
    let moved = match drag.change {
        Change::Translate(delta) => delta != DVec3::ZERO,
        Change::Rotate { angle, .. } => angle != 0.0,
    };
    if let Some(base) = &gizmo.baseline
        && let Some((translate, axis, angle_degrees)) = changed(base, drag.change)
        && (moved || gizmo.lifecycle.begun())
    {
        gizmo.moved = moved;
        let mut lifecycle = std::mem::take(&mut gizmo.lifecycle);
        lifecycle.author(
            app,
            Action::TransformDraftChanged {
                translate,
                axis,
                angle_degrees,
            },
        );
        app.doc.transform_gizmo.lifecycle = lifecycle;
    }
    if drag.done {
        let gizmo = &mut app.doc.transform_gizmo;
        gizmo.baseline = None;
        let changed = std::mem::take(&mut gizmo.moved);
        std::mem::take(&mut gizmo.lifecycle).end(app, Outcome::Finish { changed });
    }
}

/// Ends a gizmo capture whose release or loss never reached `show`.
pub(crate) fn settle(app: &mut App, ctx: &egui::Context) {
    if !app.doc.transform_gizmo.gizmo.active() {
        return;
    }
    let (focused, down) = ctx.input(|i| (i.focused, i.pointer.primary_down()));
    if !focused || app.doc.interaction.tool.is_none() {
        abandon(app, Outcome::Lost);
    } else if !down {
        let gizmo = &mut app.doc.transform_gizmo;
        gizmo.gizmo.cancel();
        gizmo.baseline = None;
        let changed = std::mem::take(&mut gizmo.moved);
        std::mem::take(&mut gizmo.lifecycle).end(app, Outcome::Finish { changed });
    }
}

/// Ends any gizmo capture and restores the placement authored before it.
pub(crate) fn abandon(app: &mut App, outcome: Outcome) {
    let gizmo = &mut app.doc.transform_gizmo;
    gizmo.gizmo.cancel();
    gizmo.baseline = None;
    gizmo.moved = false;
    std::mem::take(&mut gizmo.lifecycle).end(app, outcome);
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
