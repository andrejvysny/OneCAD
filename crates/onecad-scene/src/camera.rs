use crate::{DMat4, DVec2, DVec3, SceneError, SceneIdentity};

pub const CAMERA_NEAR: f64 = 0.1;
pub const CAMERA_FAR: f64 = 100_000.0;
pub const CAMERA_FOV_DEGREES: f64 = 76.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    Orthographic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: DVec3,
    /// Target-to-eye vector; its length controls orthographic apparent size.
    pub offset: DVec3,
    /// Horizontal heading retained at the Z pole, projected onto world XY.
    pub pole_right: DVec3,
    /// Rotation of the view's right/up basis about its target-to-eye axis.
    pub roll_radians: f64,
    pub projection: Projection,
    pub vertical_fov_degrees: f64,
    pub near: f64,
    pub far: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: DVec3::ZERO,
            offset: DVec3::new(400.0, -400.0, 300.0),
            pole_right: DVec3::X,
            roll_radians: 0.0,
            projection: Projection::Perspective,
            vertical_fov_degrees: CAMERA_FOV_DEGREES,
            near: CAMERA_NEAR,
            far: CAMERA_FAR,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Logical pixels, independent of device scale.
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBasis {
    pub right: DVec3,
    pub up: DVec3,
    pub back: DVec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureIdentity {
    pub scene: SceneIdentity,
    pub view: u64,
    camera: Camera,
    viewport: Viewport,
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: DVec3,
    pub direction: DVec3,
}

#[derive(Clone, Copy, Debug)]
pub struct ProjectedPoint {
    pub logical: DVec2,
    /// Positive distance along the camera's forward axis.
    pub depth: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ViewSnapshot {
    capture: CaptureIdentity,
    basis: CameraBasis,
    view_matrix: DMat4,
    projection_matrix: DMat4,
}

impl ViewSnapshot {
    pub fn new(
        scene: SceneIdentity,
        view: u64,
        camera: Camera,
        viewport: Viewport,
    ) -> Result<Self, SceneError> {
        validate(camera, viewport)?;
        let basis = camera.basis()?;
        let eye = camera.target + camera.offset;
        let world = DMat4::from_cols(
            basis.right.extend(0.0),
            basis.up.extend(0.0),
            basis.back.extend(0.0),
            eye.extend(1.0),
        );
        let aspect = viewport.width / viewport.height;
        let half = camera.offset.length() * (camera.vertical_fov_degrees.to_radians() / 2.0).tan();
        let projection_matrix = match camera.projection {
            Projection::Perspective => DMat4::perspective_rh(
                camera.vertical_fov_degrees.to_radians(),
                aspect,
                camera.near,
                camera.far,
            ),
            Projection::Orthographic => DMat4::orthographic_rh(
                -half * aspect,
                half * aspect,
                -half,
                half,
                camera.near,
                camera.far,
            ),
        };
        let result = Self {
            capture: CaptureIdentity {
                scene,
                view,
                camera,
                viewport,
            },
            basis,
            view_matrix: world.inverse(),
            projection_matrix,
        };
        if !result.view_matrix.is_finite() || !projection_matrix.is_finite() {
            return Err(SceneError::InvalidView("non-finite matrices"));
        }
        Ok(result)
    }
    pub fn identity(&self) -> CaptureIdentity {
        self.capture
    }
    pub fn camera(&self) -> Camera {
        self.capture.camera
    }
    pub fn viewport(&self) -> Viewport {
        self.capture.viewport
    }
    pub fn basis(&self) -> CameraBasis {
        self.basis
    }
    pub fn view_matrix(&self) -> DMat4 {
        self.view_matrix
    }
    /// Right-handed projection with depth in [0, 1], suitable for wgpu.
    pub fn projection_matrix(&self) -> DMat4 {
        self.projection_matrix
    }
    pub fn view_projection(&self) -> DMat4 {
        self.projection_matrix * self.view_matrix
    }
    pub fn eye(&self) -> DVec3 {
        self.camera().target + self.camera().offset
    }
    pub fn depth(&self, point: DVec3) -> f64 {
        (self.eye() - point).dot(self.basis.back)
    }
    pub fn world_per_pixel(&self, depth: f64) -> f64 {
        let c = self.camera();
        let distance = match c.projection {
            Projection::Perspective => depth,
            Projection::Orthographic => c.offset.length(),
        };
        2.0 * distance * (c.vertical_fov_degrees.to_radians() / 2.0).tan() / self.viewport().height
    }
    pub fn project(&self, point: DVec3) -> Option<ProjectedPoint> {
        let depth = self.depth(point);
        if depth < self.camera().near || depth > self.camera().far {
            return None;
        }
        self.project_unclipped(point)
    }
    // Segment clipping can land a few ulps outside the plane after interpolation.
    // It must project that endpoint rather than discard the entire visible line.
    pub(crate) fn project_unclipped(&self, point: DVec3) -> Option<ProjectedPoint> {
        let depth = self.depth(point);
        if !point.is_finite() || depth <= 0.0 {
            return None;
        }
        let clip = self.view_projection() * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let v = self.viewport();
        let logical = DVec2::new(
            (ndc.x + 1.0) * v.width / 2.0,
            (1.0 - ndc.y) * v.height / 2.0,
        );
        logical
            .is_finite()
            .then_some(ProjectedPoint { logical, depth })
    }
    pub fn ray(&self, logical: DVec2) -> Result<Ray, SceneError> {
        let v = self.viewport();
        if !logical.is_finite()
            || logical.x < 0.0
            || logical.y < 0.0
            || logical.x > v.width
            || logical.y > v.height
        {
            return Err(SceneError::InvalidView("cursor outside logical viewport"));
        }
        let x = 2.0 * logical.x / v.width - 1.0;
        let y = 1.0 - 2.0 * logical.y / v.height;
        let half = self.camera().offset.length()
            * (self.camera().vertical_fov_degrees.to_radians() / 2.0).tan();
        let shift = self.basis.right * x * half * v.width / v.height + self.basis.up * y * half;
        Ok(match self.camera().projection {
            Projection::Perspective => Ray {
                origin: self.eye(),
                direction: (shift - self.camera().offset)
                    .try_normalize()
                    .ok_or(SceneError::InvalidView("unrepresentable camera ray"))?,
            },
            Projection::Orthographic => Ray {
                origin: self.eye() + shift,
                direction: -self.basis.back,
            },
        })
    }
}

impl Camera {
    pub fn basis(self) -> Result<CameraBasis, SceneError> {
        let back = self
            .offset
            .try_normalize()
            .ok_or(SceneError::InvalidView("zero camera offset"))?;
        let mut right = DVec3::new(-self.offset.y, self.offset.x, 0.0);
        if right.length_squared() < 1e-20 {
            right = DVec3::new(self.pole_right.x, self.pole_right.y, 0.0);
        }
        let right = right
            .try_normalize()
            .ok_or(SceneError::InvalidView("invalid pole heading"))?;
        let up = back.cross(right).normalize();
        if !self.roll_radians.is_finite() {
            return Err(SceneError::InvalidView("non-finite camera roll"));
        }
        let (sin, cos) = self.roll_radians.sin_cos();
        let (right, up) = (right * cos + up * sin, up * cos - right * sin);
        if !right.is_finite() || !up.is_finite() || !back.is_finite() {
            return Err(SceneError::InvalidView("invalid basis"));
        }
        Ok(CameraBasis { right, up, back })
    }
}

fn validate(c: Camera, v: Viewport) -> Result<(), SceneError> {
    if !c.target.is_finite()
        || !c.offset.is_finite()
        || !c.pole_right.is_finite()
        || !c.roll_radians.is_finite()
        || !c.offset.length().is_finite()
        || c.offset.length() <= 0.0
        || !c.vertical_fov_degrees.is_finite()
        || c.vertical_fov_degrees <= 0.0
        || c.vertical_fov_degrees >= 180.0
        || !c.near.is_finite()
        || !c.far.is_finite()
        || c.near <= 0.0
        || c.far <= c.near
    {
        return Err(SceneError::InvalidView("invalid camera parameters"));
    }
    if !v.width.is_finite()
        || !v.height.is_finite()
        || !v.scale_factor.is_finite()
        || v.width <= 0.0
        || v.height <= 0.0
        || v.scale_factor <= 0.0
        || !(v.width * v.scale_factor).is_finite()
        || !(v.height * v.scale_factor).is_finite()
    {
        return Err(SceneError::InvalidView(
            "invalid viewport dimensions or scale",
        ));
    }
    Ok(())
}
