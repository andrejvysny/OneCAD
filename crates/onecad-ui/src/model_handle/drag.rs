use super::{Kind, Target};
use crate::parameter_drag::Lifecycle;
use egui::{Pos2, Rect};
use onecad_interaction::{NumericParameter as N, ParameterKey};
use onecad_scene::{Camera, ViewSnapshot, Viewport};

pub(super) struct Drag {
    pub target: Target,
    pub value: f64,
    pub sent: Option<String>,
    pub lifecycle: Lifecycle,
    press: Pos2,
    last: f64,
    delta: f64,
    baseline: f64,
    accepted: bool,
    pub free: bool,
    pub frame: (Camera, Viewport, Rect),
}
impl Drag {
    pub(super) fn start(
        target: Target,
        press: Pos2,
        view: &ViewSnapshot,
        rect: Rect,
        free: bool,
    ) -> Option<Self> {
        let last = target.geometry.coordinate(view, rect, press)?;
        let value = target.value;
        Some(Self {
            target,
            value,
            sent: None,
            lifecycle: Lifecycle::default(),
            press,
            last,
            delta: 0.0,
            baseline: value,
            accepted: false,
            free,
            frame: (view.camera(), view.viewport(), rect),
        })
    }
    pub(super) fn track(&mut self, pos: Pos2, view: &ViewSnapshot, rect: Rect) {
        let Some(coordinate) = self.target.geometry.coordinate(view, rect, pos) else {
            return;
        };
        let delta = coordinate - self.last;
        self.delta += if self.target.geometry.angle() {
            (delta + 180.0).rem_euclid(360.0) - 180.0
        } else {
            delta
        };
        self.last = coordinate;
        self.accepted |= crate::parameter_drag::beyond_threshold(
            self.press,
            pos,
            crate::parameter_drag::THRESHOLD_PX,
        );
        if !self.accepted {
            return;
        }
        let step = if self.target.geometry.angle() {
            5.0
        } else {
            crate::transform_ui::snap_step(
                view.world_per_pixel(view.depth(self.target.geometry.base())),
            )
        };
        let raw = self.baseline + self.delta;
        let snapped = if self.free {
            raw
        } else {
            (raw / step).round() * step
        };
        let next = bounded(&self.target.kind, snapped);
        if next.is_finite() {
            self.value = next;
            if next != snapped {
                self.baseline = next;
                self.delta = 0.0;
            }
        }
    }
    pub(super) fn text(&self) -> String {
        let text = crate::sketch_numeric::format::format_measure(self.value);
        if crate::sketch_numeric::format::format_measure(self.target.value) == text {
            self.target.original.clone()
        } else {
            text
        }
    }
    pub(super) fn changed(&self) -> bool {
        self.accepted
            && match self.target.kind {
                Kind::Scalar(_) => self.text() != self.target.original,
                Kind::Mirror(_) => self.value.abs() >= 0.005,
            }
    }
    pub(super) fn pending(&self) -> Option<String> {
        if !self.accepted {
            return None;
        }
        let text = match self.target.kind {
            Kind::Scalar(_) => self.text(),
            Kind::Mirror(_) => crate::sketch_numeric::format::format_measure(self.value),
        };
        let original = match self.target.kind {
            Kind::Scalar(_) => self.target.original.as_str(),
            Kind::Mirror(_) => "0",
        };
        (text != self.sent.as_deref().unwrap_or(original)).then_some(text)
    }
}
fn bounded(kind: &Kind, value: f64) -> f64 {
    match kind {
        Kind::Scalar(ParameterKey::Scalar(parameter)) => match parameter {
            N::RevolveAngle => value.clamp(0.5, 360.0),
            N::CircularPatternAngle => value.clamp(1.0, 360.0),
            N::FilletRadius | N::ChamferDistance | N::ShellThickness => value.max(0.1),
            N::HoleDiameter | N::HoleDepth | N::GearHeight | N::GearModule => value.max(0.01),
            _ => value,
        },
        _ => value,
    }
}
