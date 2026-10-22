//! Quality look adapted from the recovered render-look spike, using captured scene matrices.
use crate::{mesh::BodyStyle, RenderError};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use onecad_scene::{Bounds, Projection, ViewSnapshot};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameU {
    pub view_proj: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
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
    pub overlay: [f32; 4],
    pub section: [f32; 4],
    pub cap: [f32; 4],
    pub display: [f32; 4],
    pub support_origin: [f32; 4],
    pub support_x: [f32; 4],
    pub support_y: [f32; 4],
}

/// Transient grid placement, independent of the world-ground contact shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridFrame {
    pub origin: onecad_scene::DVec3,
    pub x_axis: onecad_scene::DVec3,
    pub y_axis: onecad_scene::DVec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SilhouetteMode {
    Geometric,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderStyle {
    pub theme: Theme,
    pub silhouettes: SilhouetteMode,
    pub support_grid: Option<GridFrame>,
    pub grid_visible: bool,
}
impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            silhouettes: SilhouetteMode::Geometric,
            support_grid: None,
            grid_visible: true,
        }
    }
}

fn resolve_token(theme: Theme, name: &str) -> Result<[f32; 3], RenderError> {
    let css = include_str!("../../../assets/native/tokens.css");
    let base = css
        .split_once("@theme {")
        .ok_or(RenderError::Mesh("theme tokens"))?
        .1
        .split("\n}")
        .next()
        .unwrap_or("");
    let dark = css
        .split_once(":root[data-theme=\"dark\"] {")
        .map(|(_, s)| s.split("\n}").next().unwrap_or(""))
        .unwrap_or("");
    let text = if theme == Theme::Dark {
        token_value(dark, name).or_else(|| token_value(base, name))
    } else {
        token_value(base, name)
    }
    .ok_or(RenderError::Mesh("missing color token"))?;
    let hex = u32::from_str_radix(text.trim_start_matches('#'), 16)
        .map_err(|_| RenderError::Mesh("invalid color token"))?;
    Ok([16, 8, 0].map(|shift| {
        let c = ((hex >> shift) & 255) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }))
}

fn token(theme: Theme, name: &str) -> Result<[f32; 3], RenderError> {
    const NAMES: [&str; 9] = [
        "--color-body-fill",
        "--color-canvas",
        "--color-viewport-grid-minor",
        "--color-viewport-grid-major",
        "--color-sel-text",
        "--color-body-edge",
        "--color-body-edge-wire",
        "--color-accent",
        "--color-viewport-hover",
    ];
    type Palette = Result<Vec<[f32; 3]>, RenderError>;
    static LIGHT: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
    static DARK: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
    let palette = if theme == Theme::Dark { &DARK } else { &LIGHT };
    let colors = palette.get_or_init(|| {
        NAMES
            .iter()
            .map(|name| resolve_token(theme, name))
            .collect()
    });
    let index = NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .ok_or(RenderError::Mesh("missing color token"))?;
    colors
        .as_ref()
        .map(|colors| colors[index])
        .map_err(|_| RenderError::Mesh("invalid color palette"))
}

/// GPU depth is reversed independently of CPU picking/projection semantics.
pub(crate) fn reversed_depth() -> Mat4 {
    Mat4::from_cols(
        glam::Vec4::X,
        glam::Vec4::Y,
        -glam::Vec4::Z,
        glam::Vec4::new(0.0, 0.0, 1.0, 1.0),
    )
}

pub(crate) fn with_options(
    mut frame: FrameU,
    options: crate::RenderOptions,
    profile: crate::RenderProfile,
) -> FrameU {
    let ratio = options.ssaa / frame.display[1];
    frame.viewport[0] *= ratio;
    frame.viewport[1] *= ratio;
    frame.viewport[2] /= ratio;
    frame.viewport[3] /= ratio;
    frame.lines[0] *= ratio;
    frame.lines[1] *= ratio;
    frame.ground[1] *= ratio;
    frame.ground[2] *= ratio;
    frame.section = options.section;
    frame.cap = options.cap;
    frame.display = [
        if options.display_mode == crate::DisplayMode::Xray {
            options.xray_opacity
        } else {
            1.0
        },
        options.ssaa,
        options.xray_hidden_edge_opacity,
        f32::from(options.display_mode == crate::DisplayMode::Xray),
    ];
    frame.material[2] = 0.0;
    frame.material[3] = f32::from(options.display_mode == crate::DisplayMode::Wireframe);
    if options.display_mode == crate::DisplayMode::Wireframe && frame.overlay[3] == 0.0 {
        frame.edge = frame.fill;
    }
    if !options.display_mode.studio() {
        frame.flags[1] = 0.0;
        frame.flags[2] = 0.0;
        frame.contact[3] = 0.0;
    }
    if !profile.ambient_occlusion() {
        // The AO attachment is a 1x1 placeholder; bodies must not read it.
        frame.flags[1] = 0.0;
    }
    frame
}

pub(crate) fn uniform(
    view: &ViewSnapshot,
    bounds: Option<Bounds>,
    style: RenderStyle,
    body: BodyStyle,
    preview: bool,
) -> Result<FrameU, RenderError> {
    let mut frame = FrameU::zeroed();
    frame.display = [1.0, 1.0, 0.0, 1.0];
    set_camera(&mut frame, view, style);
    set_lighting(&mut frame, style)?;
    set_ground(&mut frame, view, bounds, style)?;
    set_body_style(&mut frame, view, style, body, preview)?;
    if bytemuck::cast_slice::<FrameU, f32>(&[frame])
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(RenderError::Mesh("camera exceeds f32 renderer range"));
    }
    Ok(frame)
}

fn rgba(rgb: [f32; 3], alpha: f32) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], alpha]
}

fn set_camera(frame: &mut FrameU, view: &ViewSnapshot, style: RenderStyle) {
    let camera = view.camera();
    let vp = view.viewport();
    let yaw = (camera.offset.y as f32).atan2(camera.offset.x as f32);
    let width = (vp.width * vp.scale_factor).round() as f32;
    let height = (vp.height * vp.scale_factor).round() as f32;
    frame.view_proj = (reversed_depth() * view.view_projection().as_mat4()).to_cols_array_2d();
    frame.view = view.view_matrix().as_mat4().to_cols_array_2d();
    frame.inverse_projection = (reversed_depth() * view.projection_matrix().as_mat4())
        .inverse()
        .to_cols_array_2d();
    frame.eye = rgba(
        view.eye().as_vec3().to_array(),
        if camera.projection == Projection::Perspective {
            1.0
        } else {
            0.0
        },
    );
    frame.viewport = [width, height, 1.0 / width, 1.0 / height];
    frame.env = [
        yaw.cos(),
        yaw.sin(),
        1.0,
        if style.theme == Theme::Dark {
            0.95
        } else {
            0.78
        },
    ];
    let pitch = 52.0f32.to_radians();
    let azimuth = yaw + 40.0f32.to_radians();
    frame.key = [
        azimuth.cos() * pitch.cos(),
        azimuth.sin() * pitch.cos(),
        pitch.sin(),
        1.0,
    ];
}

fn set_lighting(frame: &mut FrameU, style: RenderStyle) -> Result<(), RenderError> {
    frame.body = rgba(token(style.theme, "--color-body-fill")?, 0.42);
    frame.material = [0.0, 0.04, 0.0, 0.0];
    frame.canvas = rgba(token(style.theme, "--color-canvas")?, 1.0);
    frame.flags = [1.0, 1.0, 1.0, 1.0];
    Ok(())
}

fn set_ground(
    frame: &mut FrameU,
    view: &ViewSnapshot,
    bounds: Option<Bounds>,
    style: RenderStyle,
) -> Result<(), RenderError> {
    let minor = grid_step(view.camera().offset.length() as f32);
    let target = view.camera().target.as_vec3();
    let center = (target.truncate() / (minor * 10.0)).round() * (minor * 10.0);
    let (matrix, rect, params, height_scale) = contact_params(bounds);
    frame.shadow_vp = matrix.to_cols_array_2d();
    frame.shadow_rect = rect;
    frame.contact = params;
    frame.proj[0] = height_scale;
    frame.grid_minor = rgba(token(style.theme, "--color-viewport-grid-minor")?, 1.0);
    frame.grid_major = rgba(token(style.theme, "--color-viewport-grid-major")?, 1.0);
    frame.grid = [minor, center.x, center.y, 100.0 * minor];
    if let Some(support) = style.support_grid {
        let target = view.camera().target - support.origin;
        let center_x = (target.dot(support.x_axis) / f64::from(minor * 10.0)).round()
            * f64::from(minor * 10.0);
        let center_y = (target.dot(support.y_axis) / f64::from(minor * 10.0)).round()
            * f64::from(minor * 10.0);
        frame.support_origin = support.origin.as_vec3().extend(1.0).to_array();
        frame.support_x = support.x_axis.as_vec3().extend(center_x as f32).to_array();
        frame.support_y = support.y_axis.as_vec3().extend(center_y as f32).to_array();
    }
    if !style.grid_visible {
        frame.support_origin[3] = -1.0;
    }
    frame.ground = [
        0.0,
        1.0,
        1.5,
        if style.theme == Theme::Dark {
            1.0
        } else {
            0.55
        },
    ];
    let diagonal = bounds
        .map(|b| (b.max - b.min).length() as f32)
        .unwrap_or(1.0)
        .max(1e-3);
    let radius = diagonal * 0.03;
    let falloff = radius * 0.615;
    frame.ao = [
        radius,
        -1.0 / falloff,
        (radius - falloff) / falloff + 1.0,
        1.8,
    ];
    Ok(())
}

/// Ghost face opacity of modeling-operation previews.
pub(crate) const PREVIEW_FACE_ALPHA: f32 = 0.18;
/// Occluded preview edges stay faint, like Fusion's ghost edges.
pub(crate) const PREVIEW_HIDDEN_EDGE_ALPHA: f32 = 0.3;

fn set_body_style(
    frame: &mut FrameU,
    view: &ViewSnapshot,
    style: RenderStyle,
    body: BodyStyle,
    preview: bool,
) -> Result<(), RenderError> {
    let scale = view.viewport().scale_factor as f32;
    frame.fill = rgba(token(style.theme, "--color-body-edge-wire")?, 1.0);
    frame.edge = rgba(
        token(
            style.theme,
            if body.selected {
                "--color-sel-text"
            } else {
                "--color-body-edge"
            },
        )?,
        1.0,
    );
    frame.lines = [
        if body.selected { 3.0 } else { 1.1 } * scale,
        1.1 * scale,
        1.0,
        1.0,
    ];
    if body.selected {
        frame.overlay = rgba(token(style.theme, "--color-accent")?, 0.45);
    }
    if preview {
        frame.overlay = rgba(token(style.theme, "--color-accent")?, PREVIEW_FACE_ALPHA);
        frame.edge = rgba(token(style.theme, "--color-accent")?, 1.0);
        frame.lines[0] = 1.5 * scale;
        frame.lines[1] = 1.5 * scale;
    }
    Ok(())
}

/// Minor grid spacing drawn for a camera distance; sketch grid snapping shares it.
pub fn grid_step(distance: f32) -> f32 {
    let v = (distance.max(1.0) / 25.0).max(1e-6);
    let power = 10.0f32.powf(v.log10().floor());
    let n = v / power;
    let step = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    step * power
}

fn contact_params(bounds: Option<Bounds>) -> (Mat4, [f32; 4], [f32; 4], f32) {
    let (lo, hi) = bounds
        .map(|b| (b.min.as_vec3(), b.max.as_vec3()))
        .unwrap_or((Vec3::ZERO, Vec3::ONE));
    let ext = hi - lo;
    let e = ext.x.max(ext.y).max(1e-3);
    let center = (lo + hi) * 0.5;
    let half = 0.85 * e;
    let z0 = 0.0f32.min(lo.z) - 1e-3 * e;
    let soft = 0.5 * e;
    // Keep every wall in the stencil depth range, including walls above shadow reach.
    let full_height = soft.max(hi.z - z0 + 1e-3 * e);
    let tight = 0.03 * e;
    let texel = 2.0 * half / 256.0;
    let matrix = Mat4::from_cols(
        glam::Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 1.0 / full_height, 0.0),
        glam::Vec4::new(-center.x / half, -center.y / half, -z0 / full_height, 1.0),
    );
    (
        matrix,
        [
            center.x - half,
            center.y - half,
            1.0 / (2.0 * half),
            1.0 / (2.0 * half),
        ],
        [0.012 * e / texel, 0.12 * e / texel, soft / tight, 0.5],
        full_height / soft,
    )
}

fn token_value<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    block.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        (key == name).then_some(value.trim().trim_end_matches(';').trim())
    })
}

pub(crate) fn highlight_uniform(
    mut frame: FrameU,
    view: &ViewSnapshot,
    style: RenderStyle,
    tone: crate::highlights::HighlightTone,
) -> Result<FrameU, RenderError> {
    use crate::highlights::HighlightTone;
    let (fill, edge, alpha) = match tone {
        HighlightTone::Selected => ("--color-accent", "--color-sel-text", 0.55),
        HighlightTone::Hover => ("--color-viewport-hover", "--color-viewport-hover", 0.45),
    };
    frame.overlay = rgba(token(style.theme, fill)?, alpha);
    frame.edge = rgba(token(style.theme, edge)?, 1.0);
    frame.lines[0] = 3.0 * view.viewport().scale_factor as f32 * frame.display[1];
    Ok(frame)
}
