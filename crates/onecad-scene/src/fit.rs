use crate::{Bounds, Camera, DVec3, Projection, SceneError, Viewport};

pub const CAMERA_MIN_DISTANCE: f64 = 0.5;
pub const CAMERA_MAX_DISTANCE: f64 = 50_000.0;
pub const CAMERA_FIT_MARGIN: f64 = 1.15;

#[derive(Clone, Copy, Debug)]
pub struct SafeRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraFit {
    pub target: DVec3,
    pub distance: f64,
}

/// Match the existing cameraFit contract: project all eight corners into the
/// measured work area and refuse a fit outside distance or clip constraints.
pub fn fit_camera(
    bounds: Bounds,
    camera: Camera,
    viewport: Viewport,
    safe_rect: Option<SafeRect>,
) -> Result<CameraFit, SceneError> {
    Bounds::new(bounds.min, bounds.max)?;
    // Reuse capture validation rather than permitting invalid fit-only cameras.
    let view = crate::ViewSnapshot::new(
        crate::SceneIdentity {
            session: 0,
            scene: 0,
        },
        0,
        camera,
        viewport,
    )?;
    let ndc = safe_ndc(viewport, safe_rect)?;
    let distance = required_distance(bounds, &view, ndc)?;
    let tan_y = (camera.vertical_fov_degrees.to_radians() / 2.0).tan();
    let tan_x = tan_y * viewport.width / viewport.height;
    let center = bounds.min / 2.0 + bounds.max / 2.0;
    let (cx, cy) = ((ndc[0] + ndc[1]) / 2.0, (ndc[2] + ndc[3]) / 2.0);
    let basis = view.basis();
    let target = center - basis.right * cx * tan_x * distance - basis.up * cy * tan_y * distance;
    Ok(CameraFit { target, distance })
}

fn required_distance(
    bounds: Bounds,
    view: &crate::ViewSnapshot,
    ndc: [f64; 4],
) -> Result<f64, SceneError> {
    let [left, right, bottom, top] = ndc;
    let camera = view.camera();
    let basis = view.basis();
    let tan_y = (camera.vertical_fov_degrees.to_radians() / 2.0).tan();
    let tan_x = tan_y * view.viewport().width / view.viewport().height;
    let center = bounds.min / 2.0 + bounds.max / 2.0;
    let (cx, cy) = ((left + right) / 2.0, (bottom + top) / 2.0);
    let (mut required, mut min_z, mut max_z) =
        (CAMERA_MIN_DISTANCE, f64::INFINITY, f64::NEG_INFINITY);
    for corner in bounds.corners() {
        let relative = corner - center;
        let (x, y, z) = (
            relative.dot(basis.right),
            relative.dot(basis.up),
            relative.dot(basis.back),
        );
        max_z = max_z.max(z);
        min_z = min_z.min(z);
        let (xr, xl, yt, yb) = match camera.projection {
            Projection::Perspective => (
                x + right * tan_x * z,
                -x - left * tan_x * z,
                y + top * tan_y * z,
                -y - bottom * tan_y * z,
            ),
            Projection::Orthographic => (x, -x, y, -y),
        };
        required = required
            .max(xr / ((right - cx) * tan_x))
            .max(xl / ((cx - left) * tan_x))
            .max(yt / ((top - cy) * tan_y))
            .max(yb / ((cy - bottom) * tan_y));
    }
    fit_distance(required, min_z, max_z, camera)
}

fn fit_distance(required: f64, min_z: f64, max_z: f64, camera: Camera) -> Result<f64, SceneError> {
    let distance = (required.max(max_z + camera.near) * CAMERA_FIT_MARGIN).max(CAMERA_MIN_DISTANCE);
    if !distance.is_finite() || distance > CAMERA_MAX_DISTANCE || distance - min_z > camera.far {
        return Err(SceneError::InvalidView(
            "bounds cannot fit camera clip and distance limits",
        ));
    }
    Ok(distance)
}

fn safe_ndc(v: Viewport, rect: Option<SafeRect>) -> Result<[f64; 4], SceneError> {
    let Some(r) = rect else {
        return Ok([-1.0, 1.0, -1.0, 1.0]);
    };
    if ![r.x, r.y, r.width, r.height].iter().all(|x| x.is_finite())
        || r.width <= 0.0
        || r.height <= 0.0
    {
        return Err(SceneError::InvalidView("invalid fit work area"));
    }
    let (l, rgt, t, b) = (
        r.x.max(0.0),
        (r.x + r.width).min(v.width),
        r.y.max(0.0),
        (r.y + r.height).min(v.height),
    );
    if rgt <= l || b <= t {
        return Err(SceneError::InvalidView("empty fit work area"));
    }
    Ok([
        2.0 * l / v.width - 1.0,
        2.0 * rgt / v.width - 1.0,
        1.0 - 2.0 * b / v.height,
        1.0 - 2.0 * t / v.height,
    ])
}
