//! Anchors resolve checked topology identities; no ordinal or body-center fallback for faces.
use crate::profiles::Profile;
use onecad_core::document::record::OperationRecord;

pub(super) struct Context<'a> {
    pub scene: &'a onecad_scene::SceneSnapshot,
    pub records: &'a [OperationRecord],
    pub profiles: &'a [Profile],
    pub profile_anchors: &'a [crate::profile_capture::Anchor],
}
use onecad_core::{
    document::{
        record::KnownOperation,
        refs::{AxisRef, ElementKind, ElementRef},
    },
    ids::{BodyId, ElementId},
    math::Vec3,
};
use onecad_scene::{DVec3, SceneBody};

pub(super) fn vector(v: Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}
pub(super) fn core(v: DVec3) -> Vec3 {
    Vec3::new_unchecked(v.x, v.y, v.z)
}
pub(super) fn body<'a>(display: &Context<'a>, id: BodyId) -> Option<&'a SceneBody> {
    display
        .scene
        .bodies()
        .iter()
        .find(|body| body.body_id() == id)
}
pub(super) fn center(display: &Context<'_>, id: BodyId) -> Option<DVec3> {
    let bounds = body(display, id)?.bounds();
    Some((bounds.min + bounds.max) * 0.5)
}
pub(super) fn face_ref(display: &Context<'_>, reference: &ElementRef) -> Option<(DVec3, DVec3)> {
    let primary = reference.primary.as_ref()?;
    (primary.kind == ElementKind::Face).then_some(())?;
    face(display, primary.body, &primary.element)
}
pub(super) fn face(
    display: &Context<'_>,
    id: BodyId,
    element: &ElementId,
) -> Option<(DVec3, DVec3)> {
    let mesh = body(display, id)?.mesh().ok()?;
    let ordinal = (0..mesh.header().face_count as usize)
        .find(|&i| mesh.face_id(i) == Some(element.0.as_str()))?;
    face_geometry(&mesh, ordinal)
}
fn face_geometry(
    mesh: &onecad_protocol::mesh::MeshView<'_>,
    ordinal: usize,
) -> Option<(DVec3, DVec3)> {
    let range = mesh.face_range(ordinal)?;
    let (mut weighted, mut normal, mut area) = (DVec3::ZERO, DVec3::ZERO, 0.0);
    for index in range.first..range.first + range.count {
        let [a, b, c] = triangle_points(mesh, index as usize)?;
        let cross = (b - a).cross(c - a);
        let weight = cross.length();
        weighted += (a + b + c) / 3.0 * weight;
        normal += cross;
        area += weight;
    }
    (area > 1e-12).then_some((weighted / area, normal.try_normalize()?))
}
pub(super) fn edge(display: &Context<'_>, reference: &ElementRef) -> Option<(DVec3, DVec3)> {
    let primary = reference.primary.as_ref()?;
    (primary.kind == ElementKind::Edge).then_some(())?;
    edge_geometry(display, primary.body, primary.element.as_str())
}
pub(super) fn edge_geometry(
    display: &Context<'_>,
    id: BodyId,
    element: &str,
) -> Option<(DVec3, DVec3)> {
    let mesh = body(display, id)?.mesh().ok()?;
    let i = (0..mesh.header().edge_count as usize).find(|&i| mesh.edge_id(i) == Some(element))?;
    let range = mesh.edge_range(i)?;
    if range.count < 2 {
        return None;
    }
    let middle = range.first + (range.count - 1) / 2;
    let a = DVec3::from_array(mesh.edge_point(middle as usize)?.map(f64::from));
    let b = DVec3::from_array(mesh.edge_point(middle as usize + 1)?.map(f64::from));
    let tangent = (b - a).try_normalize()?;
    let mut incident: Vec<_> = mesh
        .edge_incident_faces(i)
        .iter()
        .map(|i| i as usize)
        .collect();
    if incident.is_empty() {
        let start = DVec3::from_array(mesh.edge_point(range.first as usize)?.map(f64::from));
        let end_index = if range.count > 2 {
            range.first + range.count / 2
        } else {
            range.first + 1
        };
        let end = DVec3::from_array(mesh.edge_point(end_index as usize)?.map(f64::from));
        incident = (0..mesh.header().face_count as usize)
            .filter(|&face| face_contains(&mesh, face, start, end))
            .collect();
    }
    let mut normal = DVec3::ZERO;
    for face in incident {
        normal += local_normal(&mesh, face, (a + b) * 0.5)?;
    }
    let normal = (normal - tangent * normal.dot(tangent)).try_normalize()?;
    Some(((a + b) * 0.5, normal))
}
fn triangle_points(mesh: &onecad_protocol::mesh::MeshView<'_>, index: usize) -> Option<[DVec3; 3]> {
    let [a, b, c] = mesh.triangle(index)?;
    let point = |i| {
        mesh.position(i as usize)
            .map(|p| DVec3::from_array(p.map(f64::from)))
    };
    Some([point(a)?, point(b)?, point(c)?])
}
fn face_contains(
    mesh: &onecad_protocol::mesh::MeshView<'_>,
    face: usize,
    a: DVec3,
    b: DVec3,
) -> bool {
    let Some(range) = mesh.face_range(face) else {
        return false;
    };
    let (mut has_a, mut has_b) = (false, false);
    for i in range.first..range.first + range.count {
        if let Some(points) = triangle_points(mesh, i as usize) {
            has_a |= points.iter().any(|&p| p.distance_squared(a) < 1e-8);
            has_b |= points.iter().any(|&p| p.distance_squared(b) < 1e-8);
        }
        if has_a && has_b {
            return true;
        }
    }
    false
}
fn local_normal(
    mesh: &onecad_protocol::mesh::MeshView<'_>,
    face: usize,
    point: DVec3,
) -> Option<DVec3> {
    let range = mesh.face_range(face)?;
    let mut nearest: Option<(f64, DVec3)> = None;
    for index in range.first..range.first + range.count {
        let [a, b, c] = triangle_points(mesh, index as usize)?;
        let distance = ((a + b + c) / 3.0).distance_squared(point);
        if nearest.is_none_or(|(previous, _)| distance < previous) {
            nearest = (b - a)
                .cross(c - a)
                .try_normalize()
                .map(|normal| (distance, normal));
        }
    }
    Some(nearest?.1)
}
pub(super) fn axis(display: &Context<'_>, axis: &AxisRef) -> Option<(DVec3, DVec3)> {
    match axis {
        AxisRef::Element {
            body: id,
            edge: element,
            ..
        } => {
            let mesh = body(display, *id)?.mesh().ok()?;
            let i = (0..mesh.header().edge_count as usize)
                .find(|&i| mesh.edge_id(i) == Some(element.0.as_str()))?;
            let range = mesh.edge_range(i)?;
            let a = DVec3::from_array(mesh.edge_point(range.first as usize)?.map(f64::from));
            let b = DVec3::from_array(
                mesh.edge_point((range.first + range.count - 1) as usize)?
                    .map(f64::from),
            );
            Some((a, (b - a).try_normalize()?))
        }
        AxisRef::SketchLine { sketch, line, .. } => {
            let params = display.records.iter().find_map(|r| match &r.op {
                onecad_core::document::record::Operation::Known(KnownOperation::Sketch(p))
                    if p.sketch == *sketch =>
                {
                    Some(p)
                }
                _ => None,
            })?;
            let line = params
                .entities
                .iter()
                .find(|e| e["id"].as_str() == Some(line.to_string().as_str()))?;
            let point = |key: &str| {
                let id = line.get(key)?.as_str()?;
                let p = params
                    .entities
                    .iter()
                    .find(|e| e["id"].as_str() == Some(id))?;
                let at = p.get("at")?.as_array()?;
                Some(
                    vector(params.plane.origin)
                        + vector(params.plane.x_axis) * at.first()?.as_f64()?
                        + vector(params.plane.y_axis) * at.get(1)?.as_f64()?,
                )
            };
            let (a, b) = (point("p0Ref")?, point("p1Ref")?);
            Some((a, (b - a).try_normalize()?))
        }
    }
}
