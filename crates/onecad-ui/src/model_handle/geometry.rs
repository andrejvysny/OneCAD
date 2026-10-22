use crate::gizmo::{along, arrow, project};
use egui::{Pos2, Rect, Stroke};
use onecad_scene::{DVec2, DVec3, ViewSnapshot};

#[derive(Clone, Copy, Debug)]
pub(super) enum Geometry {
    Axis {
        base: DVec3,
        direction: DVec3,
        scale: f64,
        extension: f64,
    },
    Ring {
        center: DVec3,
        axis: DVec3,
        radial: DVec3,
        radius: f64,
    },
}
impl Geometry {
    pub(super) fn axis(base: DVec3, direction: DVec3, scale: f64) -> Self {
        Self::Axis {
            base,
            direction,
            scale,
            extension: 1.0,
        }
    }
    pub(super) fn ring(origin: DVec3, axis: DVec3, seed: DVec3) -> Option<Self> {
        let axis = axis.try_normalize()?;
        let center = origin + axis * (seed - origin).dot(axis);
        let offset = seed - center;
        Some(Self::Ring {
            center,
            axis,
            radial: offset.try_normalize()?,
            radius: offset.length(),
        })
    }
    pub(super) fn base(self) -> DVec3 {
        match self {
            Self::Axis { base, .. } => base,
            Self::Ring { center, .. } => center,
        }
    }
    pub(super) fn angle(self) -> bool {
        matches!(self, Self::Ring { .. })
    }
    pub(super) fn coordinate(self, view: &ViewSnapshot, rect: Rect, pos: Pos2) -> Option<f64> {
        match self {
            Self::Axis {
                base,
                direction,
                scale,
                ..
            } => Some(along(view, rect, pos, base, direction)? / scale),
            Self::Ring {
                center,
                axis,
                radial,
                ..
            } => {
                let p = pos - rect.min;
                let ray = view.ray(DVec2::new(f64::from(p.x), f64::from(p.y))).ok()?;
                let denominator = ray.direction.dot(axis);
                if denominator.abs() < 1e-6 {
                    return None;
                }
                let t = (center - ray.origin).dot(axis) / denominator;
                if t < 0.0 {
                    return None;
                }
                let offset = ray.origin + ray.direction * t - center;
                (offset.length_squared() > 1e-12).then_some(
                    offset
                        .dot(axis.cross(radial))
                        .atan2(offset.dot(radial))
                        .to_degrees(),
                )
            }
        }
    }
    fn arm(self, view: &ViewSnapshot) -> f64 {
        view.world_per_pixel(view.depth(self.base())) * 40.0
    }
    pub(super) fn points(
        self,
        view: &ViewSnapshot,
        rect: Rect,
        value: f64,
    ) -> Option<(Pos2, Pos2)> {
        let tip = match self {
            Self::Axis {
                base,
                direction,
                scale,
                extension,
            } => base + direction * (extension * self.arm(view) + value * scale),
            Self::Ring {
                center,
                axis,
                radial,
                radius,
            } => {
                center
                    + glam::DQuat::from_axis_angle(axis, value.to_radians())
                        * radial
                        * radius.max(self.arm(view))
            }
        };
        Some((project(view, rect, self.base())?, project(view, rect, tip)?))
    }
    pub(super) fn claims(self, view: &ViewSnapshot, rect: Rect, value: f64, pos: Pos2) -> bool {
        rect.contains(pos)
            && self
                .points(view, rect, value)
                .is_some_and(|(base, tip)| crate::extrude_handle::hit(pos, base, tip))
            && self.coordinate(view, rect, pos).is_some()
    }
    pub(super) fn avoid(
        self,
        view: &ViewSnapshot,
        rect: Rect,
        value: f64,
        covers: &[Rect],
    ) -> Self {
        let Self::Axis {
            base,
            direction,
            scale,
            ..
        } = self
        else {
            return self;
        };
        let Some(origin) = project(view, rect, base) else {
            return self;
        };
        let arm = self.arm(view);
        let Some(end) = project(view, rect, base + direction * arm) else {
            return self;
        };
        let axis = (end - origin).normalized();
        if (end - origin).length() < 1.0 {
            return self;
        }
        let Some(range) = line_interval(origin, axis, rect.shrink(14.0)) else {
            return self;
        };
        let mut visible = vec![range];
        for cover in covers {
            if let Some(blocked) = line_interval(origin, axis, *cover) {
                visible = subtract(visible, blocked);
            }
        }
        let wanted = self
            .points(view, rect, value)
            .map_or(40.0, |(_, tip)| (tip - origin).dot(axis));
        let nearest = visible
            .into_iter()
            .map(|(a, b)| wanted.clamp(a, b))
            .min_by(|a, b| (a - wanted).abs().total_cmp(&(b - wanted).abs()));
        let Some(nearest) = nearest else {
            return self;
        };
        let Some(coordinate) = self.coordinate(view, rect, origin + axis * nearest) else {
            return self;
        };
        Self::Axis {
            base,
            direction,
            scale,
            extension: (coordinate - value) * scale / arm,
        }
    }
    pub(super) fn paint_plane(
        self,
        ui: &egui::Ui,
        view: &ViewSnapshot,
        rect: Rect,
        value: f64,
        color: egui::Color32,
    ) {
        let Self::Axis {
            base, direction, ..
        } = self
        else {
            return;
        };
        let center = base + direction * value;
        let u = direction.any_orthonormal_vector() * self.arm(view);
        let v = direction.cross(u);
        let points: Option<Vec<_>> = [
            center - u - v,
            center + u - v,
            center + u + v,
            center - u + v,
            center - u - v,
        ]
        .into_iter()
        .map(|p| project(view, rect, p))
        .collect();
        if let Some(points) = points {
            ui.painter()
                .with_clip_rect(rect)
                .add(egui::Shape::line(points, Stroke::new(1.0, color)));
        }
    }
    pub(super) fn paint(
        self,
        ui: &egui::Ui,
        rect: Rect,
        view: &ViewSnapshot,
        value: f64,
        theme: &crate::theme::Theme,
        hot: bool,
    ) -> Option<Pos2> {
        let (_, tip) = self.points(view, rect, value)?;
        let painter = ui.painter().with_clip_rect(rect);
        let color = if hot {
            theme.accent.lerp_to_gamma(egui::Color32::WHITE, 0.35)
        } else {
            theme.accent
        };
        self.paint_guides(&painter, view, rect, value, hot, color);
        painter.circle(
            tip,
            if hot { 8.0 } else { 6.0 },
            color,
            Stroke::new(1.5, theme.border_strong),
        );
        Some(tip)
    }
    fn paint_guides(
        self,
        painter: &egui::Painter,
        view: &ViewSnapshot,
        rect: Rect,
        value: f64,
        hot: bool,
        color: egui::Color32,
    ) {
        let Some((base, tip)) = self.points(view, rect, value) else {
            return;
        };
        if let Self::Ring {
            center,
            axis,
            radial,
            radius,
        } = self
        {
            let radius = radius.max(self.arm(view));
            let points: Option<Vec<_>> = (0..=64)
                .map(|i| {
                    project(
                        view,
                        rect,
                        center
                            + glam::DQuat::from_axis_angle(
                                axis,
                                std::f64::consts::TAU * f64::from(i) / 64.0,
                            ) * radial
                                * radius,
                    )
                })
                .collect();
            if let Some(points) = points {
                painter.add(egui::Shape::line(
                    points,
                    Stroke::new(if hot { 2.5 } else { 1.5 }, color),
                ));
            }
            painter.line_segment([base, tip], Stroke::new(1.0, color));
        } else {
            arrow(painter, base, tip, color, hot);
            // A witness marks the actual dimension; the extension keeps small dimensions grabbable.
            if let Self::Axis {
                base,
                direction,
                scale,
                ..
            } = self
                && let Some(witness) = project(view, rect, base + direction * value * scale)
            {
                painter.circle_stroke(witness, 3.0, Stroke::new(1.0, color));
            }
        }
    }
}

fn line_interval(origin: Pos2, axis: egui::Vec2, rect: Rect) -> Option<(f32, f32)> {
    let (mut low, mut high) = (f32::NEG_INFINITY, f32::INFINITY);
    for (p, d, min, max) in [
        (origin.x, axis.x, rect.left(), rect.right()),
        (origin.y, axis.y, rect.top(), rect.bottom()),
    ] {
        if d.abs() < 1e-6 {
            if p < min || p > max {
                return None;
            }
        } else {
            let (a, b) = ((min - p) / d, (max - p) / d);
            low = low.max(a.min(b));
            high = high.min(a.max(b));
        }
    }
    (low <= high).then_some((low, high))
}
fn subtract(ranges: Vec<(f32, f32)>, blocked: (f32, f32)) -> Vec<(f32, f32)> {
    let mut visible = Vec::new();
    for (low, high) in ranges {
        if high < blocked.0 || low > blocked.1 {
            visible.push((low, high));
            continue;
        }
        if low < blocked.0 - 1.0 {
            visible.push((low, blocked.0 - 1.0));
        }
        if high > blocked.1 + 1.0 {
            visible.push((blocked.1 + 1.0, high));
        }
    }
    visible
}
