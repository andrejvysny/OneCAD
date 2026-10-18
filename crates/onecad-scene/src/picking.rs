use crate::{
    BodyId, CaptureIdentity, DVec2, DVec3, MeshView, Projection, Ray, SceneBody, SceneError,
    SceneSnapshot, ViewSnapshot,
};

pub const EDGE_PICK_LOGICAL_PIXELS: f64 = 8.0;
pub const EDGE_DEPTH_SLACK_LOGICAL_PIXELS: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    Face,
    Edge,
}

/// Snapshot-scoped topology identity and anchor; never a persistent triangle ID.
#[derive(Clone, Copy, Debug)]
pub struct PickHit<'a> {
    pub body_id: BodyId,
    pub mesh_revision: u64,
    pub capture: CaptureIdentity,
    pub kind: PickKind,
    /// MESH1 ID bytes verbatim: TopoKey or promoted ElementId, per mesh flags.
    pub topology_id: &'a str,
    pub ids_have_element_ids: bool,
    pub world_position: DVec3,
    pub normal: Option<DVec3>,
    pub ray_distance: f64,
    pub depth: f64,
}

/// Synchronous bounded brute-force authority. Visibility uses positive camera
/// depth; the two-pixel slack is converted at the nearest face's actual depth.
/// The capture must be the exact capture used by rendering/overlays.
pub fn pick<'a>(
    scene: &'a SceneSnapshot,
    view: &ViewSnapshot,
    capture: CaptureIdentity,
    logical: DVec2,
) -> Result<Option<PickHit<'a>>, SceneError> {
    if scene.identity() != view.identity().scene || capture != view.identity() {
        return Err(SceneError::CaptureMismatch);
    }
    let ray = view.ray(logical)?;
    let mut face: Option<PickHit<'a>> = None;
    for body in scene.bodies() {
        let mesh = body.mesh()?;
        pick_faces(body, &mesh, view, ray, &mut face);
    }
    let mut edge: Option<PickHit<'a>> = None;
    for body in scene.bodies() {
        let mesh = body.mesh()?;
        pick_edges(body, &mesh, view, ray, logical, face.as_ref(), &mut edge);
    }
    Ok(edge.or(face))
}

fn pick_faces<'a>(
    body: &'a SceneBody,
    mesh: &MeshView<'a>,
    view: &ViewSnapshot,
    ray: Ray,
    best: &mut Option<PickHit<'a>>,
) {
    for face in 0..mesh.header().face_count as usize {
        let Some(span) = mesh.face_range(face) else {
            continue;
        };
        for triangle in span.first..span.first + span.count {
            let Some(indices) = mesh.triangle(triangle as usize) else {
                continue;
            };
            let vertices = indices.map(|i| {
                mesh.position(i as usize)
                    .map(|p| DVec3::from_array(p.map(f64::from)))
            });
            let [Some(a), Some(b), Some(c)] = vertices else {
                continue;
            };
            let Some((distance, normal)) = triangle_hit(ray, a, b, c) else {
                continue;
            };
            let world = ray.origin + ray.direction * distance;
            let depth = view.depth(world);
            if depth < view.camera().near || depth > view.camera().far {
                continue;
            }
            let Some(id) = mesh.face_id(face) else {
                continue;
            };
            let hit = make_hit(
                body,
                mesh,
                view,
                PickKind::Face,
                id,
                world,
                Some(normal),
                distance,
            );
            if is_nearer(&hit, best.as_ref()) {
                *best = Some(hit);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn pick_edges<'a>(
    body: &'a SceneBody,
    mesh: &MeshView<'a>,
    view: &ViewSnapshot,
    ray: Ray,
    logical: DVec2,
    face: Option<&PickHit<'a>>,
    best: &mut Option<PickHit<'a>>,
) {
    for edge in 0..mesh.header().edge_count as usize {
        let Some(span) = mesh.edge_range(edge) else {
            continue;
        };
        for point in span.first..(span.first + span.count).saturating_sub(1) {
            let (Some(a), Some(b)) = (
                mesh.edge_point(point as usize),
                mesh.edge_point(point as usize + 1),
            ) else {
                continue;
            };
            let Some(world) = closest_edge(
                view,
                logical,
                DVec3::from_array(a.map(f64::from)),
                DVec3::from_array(b.map(f64::from)),
            ) else {
                continue;
            };
            let depth = view.depth(world);
            if let Some(front) = face {
                let slack = EDGE_DEPTH_SLACK_LOGICAL_PIXELS * view.world_per_pixel(front.depth);
                if depth > front.depth + slack {
                    continue;
                }
            }
            let Some(id) = mesh.edge_id(edge) else {
                continue;
            };
            let distance = (world - ray.origin).dot(ray.direction);
            let hit = make_hit(body, mesh, view, PickKind::Edge, id, world, None, distance);
            if is_nearer(&hit, best.as_ref()) {
                *best = Some(hit);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn make_hit<'a>(
    body: &SceneBody,
    mesh: &MeshView<'a>,
    view: &ViewSnapshot,
    kind: PickKind,
    topology_id: &'a str,
    world_position: DVec3,
    normal: Option<DVec3>,
    ray_distance: f64,
) -> PickHit<'a> {
    PickHit {
        body_id: body.body_id(),
        mesh_revision: body.mesh_revision(),
        capture: view.identity(),
        kind,
        topology_id,
        ids_have_element_ids: mesh.header().ids_have_element_ids(),
        world_position,
        normal,
        ray_distance,
        depth: view.depth(world_position),
    }
}

fn is_nearer(hit: &PickHit<'_>, best: Option<&PickHit<'_>>) -> bool {
    let Some(best) = best else {
        return true;
    };
    hit.depth
        .total_cmp(&best.depth)
        .then_with(|| hit.body_id.cmp(&best.body_id))
        .then_with(|| hit.topology_id.cmp(best.topology_id))
        .is_lt()
}

fn triangle_hit(ray: Ray, a: DVec3, b: DVec3, c: DVec3) -> Option<(f64, DVec3)> {
    let (ab, ac) = (b - a, c - a);
    let normal = ab.cross(ac);
    let p = ray.direction.cross(ac);
    let determinant = ab.dot(p);
    if determinant.abs() <= 1e-14 * ab.length() * ac.length() {
        return None;
    }
    let inverse = 1.0 / determinant;
    let t = ray.origin - a;
    let u = t.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(ab);
    let v = ray.direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = ac.dot(q) * inverse;
    (distance.is_finite() && distance >= 0.0).then(|| (distance, normal.normalize()))
}

fn closest_edge(view: &ViewSnapshot, cursor: DVec2, a: DVec3, b: DVec3) -> Option<DVec3> {
    let (a, b) = clip_depth(view, a, b)?;
    let (pa, pb) = (view.project_unclipped(a)?, view.project_unclipped(b)?);
    let delta = pb.logical - pa.logical;
    if !delta.is_finite() {
        return None;
    }
    let scale = delta.abs().max_element();
    let alpha = if scale > 0.0 {
        let scaled = delta / scale;
        (((cursor - pa.logical) / scale).dot(scaled) / scaled.length_squared()).clamp(0.0, 1.0)
    } else if pa.depth <= pb.depth {
        0.0
    } else {
        1.0
    };
    let screen_distance = (cursor - pa.logical.lerp(pb.logical, alpha)).length_squared();
    if !screen_distance.is_finite() || screen_distance > EDGE_PICK_LOGICAL_PIXELS.powi(2) {
        return None;
    }
    // A linear screen parameter is not a linear world parameter in perspective.
    let parameter = match view.camera().projection {
        Projection::Perspective => {
            (alpha / pb.depth) / ((1.0 - alpha) / pa.depth + alpha / pb.depth)
        }
        Projection::Orthographic => alpha,
    };
    let world = a.lerp(b, parameter);
    world.is_finite().then_some(world)
}

fn clip_depth(view: &ViewSnapshot, a: DVec3, b: DVec3) -> Option<(DVec3, DVec3)> {
    let (da, db) = (view.depth(a), view.depth(b));
    let (near, far) = (view.camera().near, view.camera().far);
    if (da < near && db < near) || (da > far && db > far) {
        return None;
    }
    if da == db {
        return Some((a, b));
    }
    let t_near = (near - da) / (db - da);
    let t_far = (far - da) / (db - da);
    let lo = t_near.min(t_far).max(0.0);
    let hi = t_near.max(t_far).min(1.0);
    if lo > hi {
        return None;
    }
    Some((a.lerp(b, lo), a.lerp(b, hi)))
}
