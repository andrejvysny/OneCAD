//! The look variants and every tunable parameter, turned into the frame uniform.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::camera::{spherical, Camera};
use crate::scene::Scene;
use crate::tokens::{Palette, Theme};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameU {
    pub view_proj: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub shadow_vp: [[f32; 4]; 4],
    pub proj: [f32; 4],
    pub eye: [f32; 4],
    pub viewport: [f32; 4],
    pub env: [f32; 4],
    pub key: [f32; 4],
    pub fill: [f32; 4],
    pub hemi_sky: [f32; 4],
    pub hemi_ground: [f32; 4],
    pub body: [f32; 4],
    pub material: [f32; 4],
    pub edge: [f32; 4],
    pub lines: [f32; 4],
    pub canvas: [f32; 4],
    pub grid_minor: [f32; 4],
    pub grid_major: [f32; 4],
    pub grid: [f32; 4],
    pub ground: [f32; 4],
    pub shadow_rect: [f32; 4],
    pub contact: [f32; 4],
    pub ao: [f32; 4],
    pub flags: [f32; 4],
}

/// How the quality look draws silhouettes of curved faces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sil {
    /// In the body shader, where the smooth normal turns edge-on (SolidCraft `fs_solid`).
    Fragment,
    /// As wide lines along tessellation edges whose two triangles straddle the terminator.
    Geometric,
}

pub struct Look {
    pub quality: bool,
    pub theme: Theme,
    pub palette: Palette,
    pub sil: Sil,
}

/// Device pixels per CSS pixel the 2560x1600 frame stands for (a 1280x800 viewport at 2x).
pub const DPR: f32 = 2.0;

// ---- baseline: today's values (src/viewport/engine) ----
/// bodyMaterials.ts BODY_EDGE_WIDTH = cssLineWidth(1.25).
const BASE_EDGE_PX: f32 = 1.25 * DPR;
/// GridPlane.ts LineBasicMaterial: 1 device px.
const BASE_GRID_PX: f32 = 1.0;
/// bodyMaterials.ts roughness 0.5, metalness 0.
const BASE_ROUGH: f32 = 0.5;
/// lightRig.ts
const KEY_AZ: f32 = 40.0;
const KEY_EL: f32 = 35.0;
const KEY_EL_FLOOR: f32 = 20.0;
const FILL_AZ: f32 = -75.0;
const FILL_EL: f32 = -15.0;

// ---- quality ----
const Q_EDGE_PX: f32 = 1.1 * DPR;
const Q_SIL_PX: f32 = 1.1 * DPR;
const Q_GRID_MINOR_PX: f32 = 1.0;
const Q_GRID_MAJOR_PX: f32 = 1.5;
const Q_ROUGH: f32 = 0.42;
/// Key light aligned with the studio key softbox (ibl.wgsl `panel(d, 40, 52, ...)`).
const Q_KEY_EL: f32 = 52.0;
const Q_AO_RADIUS: f32 = 0.03; // × scene diagonal
const Q_AO_POWER: f32 = 1.8;

/// `GridPlane.ts` chooseGridStep: minor = snapToDecade(distance / 25), major = 10 × minor.
pub fn grid_step(distance: f32) -> f32 {
    let v = (distance.max(1.0) / 25.0).max(1e-6);
    let pow = 10f32.powf(v.log10().floor());
    let n = v / pow;
    let m = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    m * pow
}
/// `GridPlane.ts` HALF_CELLS.
pub const GRID_HALF_CELLS: i32 = 100;

/// Contact-shadow map placement (see `contact_params`).
pub struct Contact {
    pub vp: Mat4,
    pub rect: [f32; 4],
    pub params: [f32; 4],
}

pub const HEIGHT_SIZE: u32 = 1024;
pub const OCC_SIZE: u32 = 256;

pub fn contact_params(scene: &Scene, ground_z: f32) -> Contact {
    let ext = scene.bbox_max - scene.bbox_min;
    let e = ext.x.max(ext.y).max(1e-3);
    let c = (scene.bbox_min + scene.bbox_max) * 0.5;
    let half = 0.85 * e;
    let h_soft = 0.5 * e;
    let h_tight = 0.03 * e;
    let z0 = ground_z.min(scene.bbox_min.z) - 1e-3 * e;
    let vp = Mat4::from_cols(
        glam::Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 1.0 / h_soft, 0.0),
        glam::Vec4::new(-c.x / half, -c.y / half, -z0 / h_soft, 1.0),
    );
    let texel = 2.0 * half / OCC_SIZE as f32;
    Contact {
        vp,
        rect: [c.x - half, c.y - half, 1.0 / (2.0 * half), 1.0 / (2.0 * half)],
        // tight σ, soft σ (occlusion-map texels), soft/tight height ratio, tight weight
        params: [0.012 * e / texel, 0.12 * e / texel, h_soft / h_tight, 0.5],
    }
}

fn dir(az_deg: f32, el_deg: f32) -> Vec3 {
    spherical(az_deg.to_radians(), el_deg.to_radians())
}

pub fn frame_uniform(scene: &Scene, cam: &Camera, look: &Look, size: [u32; 2]) -> FrameU {
    let p = &look.palette;
    let (w, h) = (size[0] as f32, size[1] as f32);
    let yaw_deg = cam.yaw.to_degrees();
    let pitch_deg = cam.pitch.to_degrees();
    let dark = look.theme == Theme::Dark;
    let minor = grid_step(cam.distance);
    let major = minor * 10.0;
    let gc = (cam.target.truncate() / major).round() * major;
    let ground_z = 0.0;
    let contact = contact_params(scene, ground_z);
    let diag = (scene.bbox_max - scene.bbox_min).length();
    let ao_r = Q_AO_RADIUS * diag;
    let falloff_range = 0.615 * ao_r;
    let falloff_from = ao_r * (1.0 - 0.615);
    let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
    let mut u = FrameU {
        view_proj: cam.view_proj().to_cols_array_2d(),
        view: cam.view.to_cols_array_2d(),
        shadow_vp: contact.vp.to_cols_array_2d(),
        proj: [(cam.fov_y * 0.5).tan() * cam.aspect, (cam.fov_y * 0.5).tan(), crate::camera::NEAR, 0.0],
        eye: v4(cam.eye, 1.0),
        viewport: [w, h, 1.0 / w, 1.0 / h],
        env: [cam.yaw.cos(), cam.yaw.sin(), 1.0, 1.0],
        key: [0.0; 4],
        fill: [0.0; 4],
        hemi_sky: [1.0, 1.0, 1.0, 0.0],
        hemi_ground: p.canvas.linear4(),
        body: [0.0; 4],
        material: [0.0, 0.04, 0.0, 0.0],
        edge: p.body_edge.linear4(),
        lines: [0.0; 4],
        canvas: p.canvas.linear4(),
        grid_minor: p.grid_minor.linear4(),
        grid_major: p.grid_major.linear4(),
        grid: [minor, gc.x, gc.y, GRID_HALF_CELLS as f32 * minor],
        ground: [ground_z, Q_GRID_MINOR_PX, Q_GRID_MAJOR_PX, if dark { 1.0 } else { 0.55 }],
        shadow_rect: contact.rect,
        contact: contact.params,
        ao: [ao_r, -1.0 / falloff_range, falloff_from / falloff_range + 1.0, Q_AO_POWER],
        flags: [0.0; 4],
    };
    let base = p.body_fill.linear();
    if look.quality {
        u.key = v4(dir(yaw_deg + KEY_AZ, Q_KEY_EL), 1.0);
        u.body = [base[0], base[1], base[2], Q_ROUGH];
        u.env = [cam.yaw.cos(), cam.yaw.sin(), 1.0, if dark { 0.95 } else { 0.78 }];
        u.lines = [Q_EDGE_PX, Q_SIL_PX, 1.0, Q_GRID_MINOR_PX];
        u.material[2] = if look.sil == Sil::Fragment { 1.0 } else { 0.0 };
        u.flags = [1.0, 1.0, 1.0, 1.0];
    } else {
        // ViewportEngine.ts LIGHT_LEVELS (webgl) and lightRig.ts.
        let (env, key, fill, hemi) = if dark { (0.3, 2.1, 0.7, 0.15) } else { (0.35, 1.9, 0.6, 0.25) };
        let key_el = (pitch_deg + KEY_EL).clamp(KEY_EL_FLOOR, 89.0);
        u.key = v4(dir(yaw_deg + KEY_AZ, key_el), key);
        u.fill = v4(dir(yaw_deg + FILL_AZ, (pitch_deg + FILL_EL).clamp(-89.0, 89.0)), fill);
        u.hemi_sky[3] = hemi;
        u.body = [base[0], base[1], base[2], BASE_ROUGH];
        u.env = [cam.yaw.cos(), cam.yaw.sin(), env, 1.0];
        u.lines = [BASE_EDGE_PX, 0.0, 0.0, BASE_GRID_PX];
    }
    u
}

/// Today's GridPlane geometry: 201 lines each way on Z = 0, minor lines faded toward the clear
/// colour by (1 - |i|/HALF_CELLS)², majors every 10th line, re-centred on the target snapped
/// to a major line. Instances: a, b, linear rgba.
pub fn baseline_grid(cam: &Camera, p: &Palette) -> Vec<[f32; 10]> {
    let minor = grid_step(cam.distance);
    let major = minor * 10.0;
    let o = (cam.target.truncate() / major).round() * major;
    let ext = GRID_HALF_CELLS as f32 * minor;
    let (clear, minor_c, major_c) = (p.canvas.linear(), p.grid_minor.linear(), p.grid_major.linear());
    let mut out = Vec::new();
    for i in -GRID_HALF_CELLS..=GRID_HALF_CELLS {
        let c = i as f32 * minor;
        // three lerps THREE.Color values, which live in the linear working space.
        let l = if i % 10 == 0 {
            major_c
        } else {
            let f = 1.0 - i.abs() as f32 / GRID_HALF_CELLS as f32;
            let t = f * f;
            [0, 1, 2].map(|k| clear[k] + (minor_c[k] - clear[k]) * t)
        };
        out.push([o.x - ext, o.y + c, 0.0, o.x + ext, o.y + c, 0.0, l[0], l[1], l[2], 1.0]);
        out.push([o.x + c, o.y - ext, 0.0, o.x + c, o.y + ext, 0.0, l[0], l[1], l[2], 1.0]);
    }
    out
}
