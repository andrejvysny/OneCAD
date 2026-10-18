#![allow(dead_code)]
use onecad_scene::*;
use std::sync::Arc;

pub fn identity() -> SceneIdentity {
    SceneIdentity {
        session: 7,
        scene: 9,
    }
}

pub fn camera(projection: Projection) -> Camera {
    Camera {
        target: DVec3::ZERO,
        offset: DVec3::new(0.0, -10.0, 0.0),
        vertical_fov_degrees: 90.0,
        projection,
        ..Camera::default()
    }
}

pub fn view(projection: Projection, scale: f64) -> ViewSnapshot {
    ViewSnapshot::new(
        identity(),
        11,
        camera(projection),
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: scale,
        },
    )
    .unwrap()
}

pub fn scene(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
    edges: &[Vec<[f32; 3]>],
) -> SceneSnapshot {
    let body = body(vertices, triangles, edges);
    SceneSnapshot::new(identity(), vec![body], SceneLimits::default()).unwrap()
}

pub fn body(vertices: &[[f32; 3]], triangles: &[[u32; 3]], edges: &[Vec<[f32; 3]>]) -> SceneBody {
    let bytes = blob(vertices, triangles, edges);
    SceneBody::new(BodyId::new(), 3, Arc::from(bytes), MeshLimits::default()).unwrap()
}

/// Independent minimal MESH1 writer for analytical geometry tests.
pub fn blob(vertices: &[[f32; 3]], triangles: &[[u32; 3]], edges: &[Vec<[f32; 3]>]) -> Vec<u8> {
    let faces = usize::from(!triangles.is_empty());
    let mut sections = vec![
        (1, floats(vertices.iter().flatten().copied())),
        (3, words(triangles.iter().flatten().copied())),
        (
            4,
            if faces == 0 {
                vec![]
            } else {
                words([0, triangles.len() as u32])
            },
        ),
        (
            5,
            if faces == 0 {
                words([0])
            } else {
                words([0, 3])
            },
        ),
        (6, if faces == 0 { vec![] } else { b"f:1".to_vec() }),
    ];
    if !edges.is_empty() {
        add_edges(&mut sections, edges);
    }
    let start = 64 + sections.len() * 16;
    let mut bytes = vec![0u8; start];
    write_u32(&mut bytes, 0, 0x4d455348);
    bytes[4] = 1;
    if !edges.is_empty() {
        bytes[6] = 2;
    }
    for (off, value) in [
        (8, vertices.len()),
        (12, triangles.len()),
        (16, faces),
        (20, edges.len()),
        (24, edges.iter().map(Vec::len).sum()),
    ] {
        write_u32(&mut bytes, off, value as u32);
    }
    bytes[30..32].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    let points: Vec<_> = vertices.iter().chain(edges.iter().flatten()).collect();
    for axis in 0..3 {
        let lo = points.iter().map(|p| p[axis]).fold(0.0f32, f32::min);
        let hi = points.iter().map(|p| p[axis]).fold(0.0f32, f32::max);
        bytes[32 + axis * 4..36 + axis * 4].copy_from_slice(&lo.to_le_bytes());
        bytes[44 + axis * 4..48 + axis * 4].copy_from_slice(&hi.to_le_bytes());
    }
    for (index, (kind, data)) in sections.iter().enumerate() {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        let offset = bytes.len();
        write_u32(&mut bytes, 64 + index * 16, *kind);
        write_u32(&mut bytes, 72 + index * 16, offset as u32);
        write_u32(&mut bytes, 76 + index * 16, data.len() as u32);
        bytes.extend(data);
    }
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn add_edges(sections: &mut Vec<(u32, Vec<u8>)>, edges: &[Vec<[f32; 3]>]) {
    let mut point = 0u32;
    let mut ranges = Vec::new();
    let mut offsets = vec![0];
    let mut ids = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        ranges.extend([point, edge.len() as u32]);
        point += edge.len() as u32;
        ids.extend(format!("e:{index}").bytes());
        offsets.push(ids.len() as u32);
    }
    sections.extend([
        (7, words(ranges)),
        (8, floats(edges.iter().flatten().flatten().copied())),
        (9, words(offsets)),
        (10, ids),
    ]);
}
fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn words(values: impl IntoIterator<Item = u32>) -> Vec<u8> {
    values.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn floats(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_le_bytes).collect()
}
