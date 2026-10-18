//! Z-up orbit camera. World is right-handed with +Z up; nothing in the scene is ever rotated to
//! fake a different up axis (CLAUDE.md viewport invariant).

use glam::{Mat4, Vec3, Vec4};

pub const FOV_Y_DEG: f32 = 40.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    /// Rotation about +Z, radians.
    pub yaw: f32,
    /// Elevation above the XY plane, radians, clamped short of the poles.
    pub pitch: f32,
    pub ortho: bool,
}

impl Camera {
    pub fn fit(min: Vec3, max: Vec3) -> Self {
        Self::fit_aspect(min, max, 1.0)
    }

    /// Frames the bounding sphere of `min..max` in a viewport of `aspect` = width / height.
    pub fn fit_aspect(min: Vec3, max: Vec3, aspect: f32) -> Self {
        let radius = (max - min).length() * 0.5;
        let half_y = FOV_Y_DEG.to_radians() * 0.5;
        let half_x = (half_y.tan() * aspect).atan();
        let distance = radius / half_y.min(half_x).sin() * 1.05;
        Camera {
            target: (min + max) * 0.5,
            distance,
            yaw: (-55f32).to_radians(),
            pitch: 28f32.to_radians(),
            ortho: false,
        }
    }

    pub fn eye(&self) -> Vec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        self.target + self.distance * Vec3::new(cp * cy, cp * sy, sp)
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Z)
    }

    /// Reversed-Z projection (near maps to 1, far to 0).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        let fov = FOV_Y_DEG.to_radians();
        if self.ortho {
            let h = self.distance * (fov * 0.5).tan();
            let w = h * aspect;
            Mat4::orthographic_rh(-w, w, -h, h, self.distance * 20.0, -self.distance * 20.0)
        } else {
            Mat4::perspective_infinite_reverse_rh(fov, aspect, (self.distance * 0.01).max(0.01))
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Orbit by a drag delta in logical pixels.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        const SPEED: f32 = 0.008;
        self.yaw -= dx * SPEED;
        self.pitch = (self.pitch + dy * SPEED).clamp(-1.55, 1.55);
    }

    /// Pan so the point under the cursor follows it; `h` is the viewport height in logical px.
    pub fn pan(&mut self, dx: f32, dy: f32, h: f32) {
        let world_per_px = 2.0 * self.distance * (FOV_Y_DEG.to_radians() * 0.5).tan() / h.max(1.0);
        let view = self.view();
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        self.target += (-dx * right + dy * up) * world_per_px;
    }

    /// Zoom by `factor` (<1 moves closer) toward the cursor at logical (`x`, `y`) in a `w`×`h`
    /// viewport, keeping the point under the cursor on the target plane fixed.
    pub fn zoom_at(&mut self, factor: f32, x: f32, y: f32, w: f32, h: f32) {
        let factor = factor.clamp(0.2, 5.0);
        let before = self.distance;
        self.distance = (self.distance * factor).clamp(0.5, 50_000.0);
        let k = 1.0 - self.distance / before;
        // Shift the target toward the cursor ray's point at the target depth.
        let ndc_x = (x / w.max(1.0)) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y / h.max(1.0)) * 2.0;
        let half_h = before * (FOV_Y_DEG.to_radians() * 0.5).tan();
        let view = self.view();
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        let off = right * ndc_x * half_h * (w / h.max(1.0)) + up * ndc_y * half_h;
        self.target += off * k;
    }

    /// World point to logical pixel position in a `w`×`h` viewport; `None` when behind the eye.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<(f32, f32)> {
        let c = self.view_proj(w / h.max(1.0)) * Vec4::new(p.x, p.y, p.z, 1.0);
        if c.w <= 1e-6 {
            return None;
        }
        Some(((c.x / c.w * 0.5 + 0.5) * w, (0.5 - c.y / c.w * 0.5) * h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_is_up_on_screen() {
        let c = Camera::fit(Vec3::splat(-10.0), Vec3::splat(10.0));
        let (_, y0) = c.project(Vec3::ZERO, 800.0, 600.0).unwrap();
        let (_, y1) = c.project(Vec3::new(0.0, 0.0, 5.0), 800.0, 600.0).unwrap();
        assert!(y1 < y0, "+Z must project upward on screen");
        let (x, y) = c.project(c.target, 800.0, 600.0).unwrap();
        assert!((x - 400.0).abs() < 1e-3 && (y - 300.0).abs() < 1e-3);
    }
}
