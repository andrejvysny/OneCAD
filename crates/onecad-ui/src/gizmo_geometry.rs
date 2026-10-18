use super::{Change, Drag, Handle, LEN_PX, Triad};
use egui::{Pos2, Rect};
use onecad_scene::{DVec2, DVec3, ViewSnapshot};

pub(super) const DIRS: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];
const RING: f64 = 0.62;
pub(super) fn ring_basis(i: usize) -> Option<(DVec3, DVec3)> {
    match i {
        0 => Some((DVec3::Y, DVec3::Z)),
        1 => Some((DVec3::Z, DVec3::X)),
        2 => Some((DVec3::X, DVec3::Y)),
        _ => None,
    }
}
fn valid_rect(view: &ViewSnapshot, rect: Rect) -> bool {
    rect.is_finite()
        && rect.width() > 0.0
        && rect.height() > 0.0
        && (f64::from(rect.width()) - view.viewport().width).abs() < 0.01
        && (f64::from(rect.height()) - view.viewport().height).abs() < 0.01
}
pub(super) fn project(view: &ViewSnapshot, rect: Rect, point: DVec3) -> Option<Pos2> {
    let point = view.project(point)?.logical;
    let result = rect.min + egui::vec2(point.x as f32, point.y as f32);
    result.is_finite().then_some(result)
}
fn ray(view: &ViewSnapshot, rect: Rect, pos: Pos2) -> Option<onecad_scene::Ray> {
    let p = pos - rect.min;
    view.ray(DVec2::new(f64::from(p.x), f64::from(p.y))).ok()
}
pub(super) fn on_plane(
    view: &ViewSnapshot,
    rect: Rect,
    pos: Pos2,
    center: DVec3,
    normal: DVec3,
) -> Option<DVec3> {
    let ray = ray(view, rect, pos)?;
    let den = ray.direction.dot(normal);
    if !den.is_finite() || den.abs() < 1e-9 {
        return None;
    }
    let t = (center - ray.origin).dot(normal) / den;
    let point = ray.origin + ray.direction * t;
    (t >= 0.0 && point.is_finite() && view.project(point).is_some()).then_some(point)
}
pub(super) fn along(
    view: &ViewSnapshot,
    rect: Rect,
    pos: Pos2,
    center: DVec3,
    direction: DVec3,
) -> Option<f64> {
    let ray = ray(view, rect, pos)?;
    let w = center - ray.origin;
    let b = direction.dot(ray.direction);
    let den = 1.0 - b * b;
    if !den.is_finite() || den.abs() < 1e-6 {
        return None;
    }
    let t = (b * ray.direction.dot(w) - direction.dot(w)) / den;
    (t.is_finite() && view.project(center + direction * t).is_some()).then_some(t)
}
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}
fn in_quad(p: Pos2, q: &[Pos2]) -> bool {
    let mut sign = 0.0_f32;
    let mut area = 0.0_f32;
    for i in 0..q.len() {
        let (a, b) = (q[i], q[(i + 1) % q.len()]);
        area += a.x * b.y - a.y * b.x;
        let c = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if c.abs() < 1e-6 {
            continue;
        }
        if sign != 0.0 && c.signum() != sign {
            return false;
        }
        sign = c.signum();
    }
    area.abs() > 1e-3
}
pub(super) struct Geometry {
    pub arrows: Vec<(usize, Pos2, Pos2)>,
    pub planes: Vec<(usize, Vec<Pos2>)>,
    pub rings: Vec<(usize, Vec<Pos2>)>,
    pub ball: Pos2,
    pub translate: bool,
}
impl Geometry {
    pub fn new(view: &ViewSnapshot, rect: Rect, triad: Triad) -> Option<Self> {
        if !valid_rect(view, rect) || !triad.center.is_finite() {
            return None;
        }
        let ball = project(view, rect, triad.center)?;
        let length = view.world_per_pixel(view.depth(triad.center)) * LEN_PX;
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        let mut out = Self {
            arrows: vec![],
            planes: vec![],
            rings: vec![],
            ball,
            translate: triad.translate,
        };
        if triad.translate {
            out.translation_shapes(view, rect, triad.center, length);
        }
        if triad.rotate {
            for i in 0..3 {
                let (a, b) = ring_basis(i)?;
                // Keep each ring complete: omitted clip points must never bridge a gap.
                let points: Option<Vec<_>> = (0..=64)
                    .map(|k| {
                        let angle = f64::from(k) / 64.0 * std::f64::consts::TAU;
                        project(
                            view,
                            rect,
                            triad.center + (a * angle.cos() + b * angle.sin()) * RING * length,
                        )
                    })
                    .collect();
                if let Some(points) = points {
                    out.rings.push((i, points));
                }
            }
        }
        Some(out)
    }
    fn translation_shapes(&mut self, view: &ViewSnapshot, rect: Rect, center: DVec3, length: f64) {
        for (i, direction) in DIRS.iter().enumerate() {
            if let (Some(a), Some(b)) = (
                project(view, rect, center + direction * 0.18 * length),
                project(view, rect, center + direction * length),
            ) && a.distance(b) >= 1.0
            {
                self.arrows.push((i, a, b));
            }
            let Some((a, b)) = ring_basis(i) else {
                continue;
            };
            let points: Option<Vec<_>> = [(0.25, 0.25), (0.45, 0.25), (0.45, 0.45), (0.25, 0.45)]
                .into_iter()
                .map(|(x, y)| project(view, rect, center + (a * x + b * y) * length))
                .collect();
            if let Some(points) = points {
                self.planes.push((i, points));
            }
        }
    }
    pub fn hit(&self, p: Pos2) -> Option<Handle> {
        if !p.is_finite() {
            return None;
        }
        if self.translate && self.ball.distance(p) < 9.0 {
            return Some(Handle::Ball);
        }
        if let Some((i, ..)) = self
            .arrows
            .iter()
            .find(|(_, a, b)| seg_dist(p, *a, *b) < 6.0)
        {
            return Some(Handle::Axis(*i));
        }
        if let Some((i, _)) = self.planes.iter().find(|(_, q)| in_quad(p, q)) {
            return Some(Handle::Plane(*i));
        }
        self.rings
            .iter()
            .find(|(_, pts)| pts.windows(2).any(|w| seg_dist(p, w[0], w[1]) < 5.0))
            .map(|(i, _)| Handle::Ring(*i))
    }
}

pub(super) struct State {
    pub handle: Handle,
    view: ViewSnapshot,
    rect: Rect,
    triad: Triad,
    grab: DVec3,
    t0: f64,
    last: f64,
    turned: f64,
    change: Change,
}
impl State {
    pub fn matches(&self, view: &ViewSnapshot, rect: Rect, triad: Triad) -> bool {
        self.view.identity().scene == view.identity().scene
            && self.view.camera() == view.camera()
            && self.view.viewport() == view.viewport()
            && self.rect == rect
            && self.triad == triad
    }
    pub fn begin(
        view: &ViewSnapshot,
        rect: Rect,
        triad: Triad,
        handle: Handle,
        pos: Pos2,
    ) -> Option<Self> {
        Geometry::new(view, rect, triad)?;
        if !rect.contains(pos) {
            return None;
        }
        let mut out = Self {
            handle,
            view: *view,
            rect,
            triad,
            grab: triad.center,
            t0: 0.0,
            last: 0.0,
            turned: 0.0,
            change: Change::Translate(DVec3::ZERO),
        };
        out.grab_initial(pos)?;
        Some(out)
    }
    fn grab_initial(&mut self, pos: Pos2) -> Option<()> {
        let (view, rect, triad) = (&self.view, self.rect, self.triad);
        match self.handle {
            Handle::Axis(i) if triad.translate => {
                self.t0 = along(view, rect, pos, triad.center, *DIRS.get(i)?)?
            }
            Handle::Ring(i) if triad.rotate => {
                self.last = self.angle(pos, i)?;
                self.change = Change::Rotate {
                    axis: i,
                    angle: 0.0,
                };
            }
            Handle::Plane(i) if triad.translate => {
                self.grab = on_plane(view, rect, pos, triad.center, *DIRS.get(i)?)?
            }
            Handle::Ball if triad.translate => {
                self.grab = on_plane(view, rect, pos, triad.center, view.basis().back)?
            }
            _ => return None,
        }
        Some(())
    }
    fn angle(&self, pos: Pos2, i: usize) -> Option<f64> {
        let q = on_plane(&self.view, self.rect, pos, self.triad.center, *DIRS.get(i)?)?
            - self.triad.center;
        if q.length_squared() < 1e-18 {
            return None;
        }
        let (a, b) = ring_basis(i)?;
        Some(q.dot(b).atan2(q.dot(a)))
    }
    pub fn drag(
        &mut self,
        pos: Pos2,
        step: f64,
        free: bool,
        started: bool,
        done: bool,
    ) -> Option<Drag> {
        if !step.is_finite() || step < 0.0 {
            return None;
        }
        if let Some(change) = self.change_at(pos, step, free) {
            self.change = change;
        }
        Some(Drag {
            handle: self.handle,
            change: self.change,
            started,
            done,
        })
    }
    fn change_at(&mut self, pos: Pos2, step: f64, free: bool) -> Option<Change> {
        let center = self.triad.center;
        let translation = match self.handle {
            Handle::Axis(i) => {
                DIRS[i]
                    * snap(
                        along(&self.view, self.rect, pos, center, DIRS[i])? - self.t0,
                        step,
                        free,
                    )
            }
            Handle::Ring(i) => {
                let angle = self.angle(pos, i)?;
                let delta = (angle - self.last + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                self.turned += delta;
                self.last = angle;
                let angle = snap(self.turned, 5_f64.to_radians(), free);
                return angle
                    .is_finite()
                    .then_some(Change::Rotate { axis: i, angle });
            }
            handle => {
                let normal = if let Handle::Plane(i) = handle {
                    DIRS[i]
                } else {
                    self.view.basis().back
                };
                let d = on_plane(&self.view, self.rect, pos, center, normal)? - self.grab;
                DVec3::new(
                    snap(d.x, step, free),
                    snap(d.y, step, free),
                    snap(d.z, step, free),
                )
            }
        };
        translation
            .is_finite()
            .then_some(Change::Translate(translation))
    }
}
fn snap(value: f64, step: f64, free: bool) -> f64 {
    if free || step == 0.0 {
        value
    } else {
        (value / step).round() * step
    }
}
