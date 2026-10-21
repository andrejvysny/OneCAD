use glam::DVec4;
use onecad_scene::{DVec2, DVec3, ViewSnapshot};

pub(super) struct ProjectedRay {
    pub start: DVec2,
    pub end: DVec2,
    pub origin: Option<DVec2>,
}

pub(super) fn project_ray(view: &ViewSnapshot, origin: DVec3, axis: DVec3) -> Option<ProjectedRay> {
    let matrix = view.view_projection();
    let start = matrix * origin.extend(1.0);
    let direction = matrix * axis.extend(0.0);
    if !start.is_finite() || !direction.is_finite() {
        return None;
    }
    let (near, far) = clip_interval(start, direction)?;
    let first = logical(view, start + direction * near)?;
    let last = logical(
        view,
        if far.is_finite() {
            start + direction * far
        } else {
            direction
        },
    )?;
    let origin = view
        .project(origin)
        .map(|point| point.logical)
        .filter(|point| {
            point.x >= 0.0
                && point.x <= view.viewport().width
                && point.y >= 0.0
                && point.y <= view.viewport().height
        });
    Some(ProjectedRay {
        start: first,
        end: last,
        origin,
    })
}

fn clip_interval(start: DVec4, direction: DVec4) -> Option<(f64, f64)> {
    // Clip the mathematical positive ray in homogeneous space, without inventing
    // a distant world endpoint or projecting points behind the camera.
    let planes = |point: DVec4| {
        [
            point.w + point.x,
            point.w - point.x,
            point.w + point.y,
            point.w - point.y,
            point.z,
            point.w - point.z,
        ]
    };
    let mut near: f64 = 0.0;
    let mut far = f64::INFINITY;
    for (offset, slope) in planes(start).into_iter().zip(planes(direction)) {
        if slope.abs() < 1e-14 {
            if offset < 0.0 {
                return None;
            }
        } else if slope > 0.0 {
            near = near.max(-offset / slope);
        } else {
            far = far.min(-offset / slope);
        }
        if near > far {
            return None;
        }
    }
    Some((near, far))
}

fn logical(view: &ViewSnapshot, point: DVec4) -> Option<DVec2> {
    if point.w <= 0.0 {
        return None;
    }
    let viewport = view.viewport();
    let logical = DVec2::new(
        (point.x / point.w + 1.0) * viewport.width * 0.5,
        (1.0 - point.y / point.w) * viewport.height * 0.5,
    );
    logical
        .is_finite()
        .then(|| logical.clamp(DVec2::ZERO, DVec2::new(viewport.width, viewport.height)))
}

#[cfg(test)]
mod tests;
