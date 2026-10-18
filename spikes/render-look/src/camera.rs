//! The iso "home" camera of today's viewport (CadOrbitControls ISO_YAW / ISO_PITCH, CameraRig
//! fov, cameraFit margin), Z-up, with a reversed-Z infinite-far perspective projection.

use glam::{Mat4, Vec3, Vec4};

/// `CadOrbitControls.ts`: ISO_YAW = -π/4, ISO_PITCH = atan(1/√2).
pub const ISO_YAW: f32 = -std::f32::consts::FRAC_PI_4;
pub fn iso_pitch() -> f32 {
    (1.0f32 / std::f32::consts::SQRT_2).atan()
}
/// `CameraRig.ts`: vertical fov 76°, near 0.1 mm.
pub const FOV_Y_DEG: f32 = 76.0;
pub const NEAR: f32 = 0.1;
/// `cameraFit.ts`: CAMERA_FIT_MARGIN.
const FIT_MARGIN: f32 = 1.15;

pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    pub aspect: f32,
    pub view: Mat4,
    /// Reversed-Z, infinite far: depth 1 at the near plane, → 0 at infinity.
    pub proj: Mat4,
}

/// `sphericalToOffset`: yaw about world Z, pitch from the XY plane.
pub fn spherical(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin())
}

impl Camera {
    /// Frame `bmin..bmax` from the iso direction like `computeCameraFit` (persp branch, full
    /// viewport as the safe rect).
    pub fn iso_fit(bmin: Vec3, bmax: Vec3, aspect: f32) -> Camera {
        let (yaw, pitch) = (ISO_YAW, iso_pitch());
        let fov_y = FOV_Y_DEG.to_radians();
        let tan_y = (fov_y * 0.5).tan();
        let tan_x = tan_y * aspect;
        let back = spherical(yaw, pitch);
        let right = Vec3::Z.cross(back).normalize();
        let up = back.cross(right);
        let center = (bmin + bmax) * 0.5;
        let mut required = 0.5f32;
        let mut max_z = f32::NEG_INFINITY;
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { bmin.x } else { bmax.x },
                if i & 2 == 0 { bmin.y } else { bmax.y },
                if i & 4 == 0 { bmin.z } else { bmax.z },
            ) - center;
            let (x, y, z) = (c.dot(right), c.dot(up), c.dot(back));
            max_z = max_z.max(z);
            required = required
                .max((x + tan_x * z) / tan_x)
                .max((-x + tan_x * z) / tan_x)
                .max((y + tan_y * z) / tan_y)
                .max((-y + tan_y * z) / tan_y);
        }
        let distance = required.max(max_z + NEAR) * FIT_MARGIN;
        let eye = center + back * distance;
        let view = Mat4::look_at_rh(eye, center, Vec3::Z);
        let f = 1.0 / tan_y;
        let proj = Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, NEAR, 0.0),
        );
        Camera { eye, target: center, yaw, pitch, distance, fov_y, aspect, view, proj }
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
}
