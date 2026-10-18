//! Z-up turntable camera. Constants and conventions follow OneCAD's
//! `src/viewport/engine/CadOrbitControls.ts` / `CameraRig.ts` (yaw about world Z, pitch
//! clamped short of the poles, eye = target + distance * dir(yaw, pitch), FOV 76°).

use glam::{Mat4, Vec3, Vec4};

pub const ORBIT_SPEED: f32 = 0.008;
pub const PAN_SENS: f32 = 1.4;
pub const FOV_DEG: f32 = 76.0;
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 1e-3;
pub const ISO_YAW: f32 = -std::f32::consts::FRAC_PI_4;
/// OneCAD `cameraFit.ts` `CAMERA_FIT_MARGIN`.
pub const FIT_MARGIN: f32 = 1.15;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    /// Scene radius, for near/far planes.
    pub radius: f32,
}

impl Camera {
    /// Iso view of the box `lo..hi` in a viewport of `aspect`, framed like OneCAD's
    /// `computeCameraFit` (perspective branch): the smallest distance at which every box
    /// corner projects inside the viewport shrunk by `FIT_MARGIN`.
    pub fn fit(lo: [f32; 3], hi: [f32; 3], aspect: f32) -> Self {
        let (lo, hi) = (Vec3::from(lo), Vec3::from(hi));
        let center = (lo + hi) * 0.5;
        let radius = 0.5 * (hi - lo).length();
        let mut cam = Camera {
            target: center,
            yaw: ISO_YAW,
            pitch: (1.0 / std::f32::consts::SQRT_2).atan(),
            distance: 1.0,
            radius,
        };
        let back = cam.dir();
        let right = Vec3::Z.cross(back).normalize();
        let up = back.cross(right);
        let tan_y = (FOV_DEG.to_radians() * 0.5).tan();
        let tan_x = tan_y * aspect.max(1e-3);
        let mut d: f32 = 0.0;
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let rel = corner - center;
            let (x, y, z) = (rel.dot(right), rel.dot(up), rel.dot(back));
            d = d.max(z + FIT_MARGIN * (x.abs() / tan_x).max(y.abs() / tan_y));
        }
        cam.distance = d.max(radius * 0.05);
        cam
    }

    pub fn dir(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.cos(),
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
        )
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.dir() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Z)
    }

    /// Projection with depth 0 near / 1 far (wgpu convention).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        let near = (self.distance - 4.0 * self.radius).max(self.distance * 0.01);
        let far = self.distance + 4.0 * self.radius;
        Mat4::perspective_rh(FOV_DEG.to_radians(), aspect.max(1e-3), near, far)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * ORBIT_SPEED;
        self.pitch = (self.pitch + dy * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn pan(&mut self, dx: f32, dy: f32, viewport_h: f32) {
        let v = self.view();
        let right = Vec3::new(v.x_axis.x, v.y_axis.x, v.z_axis.x);
        let up = Vec3::new(v.x_axis.y, v.y_axis.y, v.z_axis.y);
        let scale = self.distance * PAN_SENS / viewport_h.max(1.0);
        self.target += right * (-dx * scale) + up * (dy * scale);
    }

    /// Dolly by `factor` (< 1 moves closer) toward the point under the cursor on the plane
    /// through the target facing the camera (OneCAD's zoom-to-cursor).
    pub fn zoom_at(&mut self, factor: f32, ndc: [f32; 2], aspect: f32) {
        let factor = factor.clamp(0.2, 5.0);
        if let Some(cursor) = self.ray_on_target_plane(ndc, aspect) {
            self.target = cursor + (self.target - cursor) * factor;
        }
        self.distance = (self.distance * factor).clamp(self.radius * 0.02, self.radius * 200.0);
    }

    fn ray_on_target_plane(&self, ndc: [f32; 2], aspect: f32) -> Option<Vec3> {
        let inv = self.view_proj(aspect).inverse();
        let p0 = inv * Vec4::new(ndc[0], ndc[1], 0.0, 1.0);
        let p1 = inv * Vec4::new(ndc[0], ndc[1], 1.0, 1.0);
        let (a, b) = (p0.truncate() / p0.w, p1.truncate() / p1.w);
        let d = (b - a).normalize_or_zero();
        let n = self.dir();
        let denom = d.dot(n);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (self.target - a).dot(n) / denom;
        (t > 0.0).then(|| a + d * t)
    }

    /// World point → position in a viewport of `size` points (origin top-left); `None` behind
    /// the eye.
    pub fn world_to_screen(&self, p: Vec3, size: [f32; 2]) -> Option<[f32; 2]> {
        let c = self.view_proj(size[0] / size[1].max(1.0)) * p.extend(1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let (x, y) = (c.x / c.w, c.y / c.w);
        Some([(x * 0.5 + 0.5) * size[0], (0.5 - y * 0.5) * size[1]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_is_z_and_target_projects_to_center() {
        let c = Camera::fit([-9.0, -8.0, -7.0], [11.0, 12.0, 13.0], 4.0 / 3.0);
        let s = c
            .world_to_screen(c.target, [800.0, 600.0])
            .expect("in front");
        assert!((s[0] - 400.0).abs() < 1e-3 && (s[1] - 300.0).abs() < 1e-3);
        // A point above the target projects above it on screen (Z up).
        let above = c
            .world_to_screen(c.target + Vec3::Z, [800.0, 600.0])
            .expect("in front");
        assert!(above[1] < 300.0);
    }

    #[test]
    fn fit_keeps_every_corner_inside_the_margin() {
        let (lo, hi) = ([-27.6, -32.8, -0.06], [27.9, 27.6, 61.1]);
        let size = [1040.0, 842.0];
        let c = Camera::fit(lo, hi, size[0] / size[1]);
        let mut touches = false;
        for i in 0..8 {
            let p = Vec3::new(
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            );
            let s = c.world_to_screen(p, size).expect("in front");
            let nx = (s[0] / size[0] * 2.0 - 1.0).abs();
            let ny = (s[1] / size[1] * 2.0 - 1.0).abs();
            assert!(
                nx <= 1.0 / FIT_MARGIN + 1e-3 && ny <= 1.0 / FIT_MARGIN + 1e-3,
                "{i}: {nx} {ny}"
            );
            touches |= (nx.max(ny) - 1.0 / FIT_MARGIN).abs() < 1e-3;
        }
        assert!(touches, "the fit is tight: some corner sits on the margin");
    }

    #[test]
    fn pitch_is_clamped() {
        let mut c = Camera::fit([0.0; 3], [1.0; 3], 1.0);
        c.orbit(0.0, 1e6);
        assert!(c.pitch < std::f32::consts::FRAC_PI_2);
    }
}
