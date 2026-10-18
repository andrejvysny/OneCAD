use super::*;

pub(super) fn sphere_scene() -> SceneSnapshot {
    let body = sphere_body([0.0; 3], 35.0);
    SceneSnapshot::new(
        SceneIdentity {
            session: 19,
            scene: 1,
        },
        vec![body],
        SceneLimits::default(),
    )
    .unwrap()
}

pub(super) fn sphere_body(center: [f32; 3], radius: f32) -> SceneBody {
    let (mut positions, indices) = sphere_geometry();
    let normals: Vec<_> = positions
        .iter()
        .map(|point| point.map(|v| v / 35.0))
        .collect();
    for point in &mut positions {
        for axis in 0..3 {
            point[axis] = center[axis] + point[axis] * radius / 35.0;
        }
    }
    SceneBody::new(
        BodyId::new(),
        1,
        Arc::from(sphere_blob(&positions, &normals, &indices)),
        onecad_scene::MeshLimits::default(),
    )
    .unwrap()
}

fn sphere_geometry() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    const SEGMENTS: u32 = 48;
    const RINGS: u32 = 24;
    let mut positions = vec![[0.0, 0.0, 35.0]];
    for ring in 1..RINGS {
        let phi = std::f32::consts::PI * ring as f32 / RINGS as f32;
        for segment in 0..SEGMENTS {
            let theta = std::f32::consts::TAU * segment as f32 / SEGMENTS as f32;
            positions.push([
                35.0 * phi.sin() * theta.cos(),
                35.0 * phi.sin() * theta.sin(),
                35.0 * phi.cos(),
            ]);
        }
    }
    let south = positions.len() as u32;
    positions.push([0.0, 0.0, -35.0]);
    let at = |ring, segment| 1 + ring * SEGMENTS + segment % SEGMENTS;
    let mut indices = Vec::new();
    for segment in 0..SEGMENTS {
        indices.push([0, at(0, segment), at(0, segment + 1)]);
        indices.push([south, at(RINGS - 2, segment + 1), at(RINGS - 2, segment)]);
        for ring in 0..RINGS - 2 {
            let [a, b, c, d] = [
                at(ring, segment),
                at(ring, segment + 1),
                at(ring + 1, segment),
                at(ring + 1, segment + 1),
            ];
            indices.extend([[a, c, b], [b, c, d]]);
        }
    }
    for triangle in &mut indices {
        let [a, b, c] = triangle.map(|index| glam::Vec3::from_array(positions[index as usize]));
        if (b - a).cross(c - a).dot(a + b + c) < 0.0 {
            triangle.swap(1, 2);
        }
    }
    (positions, indices)
}

fn sphere_blob(positions: &[[f32; 3]], normals: &[[f32; 3]], indices: &[[u32; 3]]) -> Vec<u8> {
    let sections = [
        (1u32, bytemuck::cast_slice(positions).to_vec()),
        (2, bytemuck::cast_slice(normals).to_vec()),
        (3, bytemuck::cast_slice(indices).to_vec()),
        (
            4,
            bytemuck::cast_slice(&[0u32, indices.len() as u32]).to_vec(),
        ),
        (5, bytemuck::cast_slice(&[0u32, 3]).to_vec()),
        (6, b"f:1".to_vec()),
    ];
    let mut bytes = vec![0u8; 64 + sections.len() * 16];
    word(&mut bytes, 0, 0x4d455348);
    bytes[4] = 1;
    bytes[6] = 1;
    word(&mut bytes, 8, positions.len() as u32);
    word(&mut bytes, 12, indices.len() as u32);
    word(&mut bytes, 16, 1);
    bytes[30..32].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    for axis in 0..3 {
        let lo = positions
            .iter()
            .map(|point| point[axis])
            .fold(f32::INFINITY, f32::min);
        let hi = positions
            .iter()
            .map(|point| point[axis])
            .fold(f32::NEG_INFINITY, f32::max);
        bytes[32 + axis * 4..36 + axis * 4].copy_from_slice(&lo.to_le_bytes());
        bytes[44 + axis * 4..48 + axis * 4].copy_from_slice(&hi.to_le_bytes());
    }
    for (index, (kind, data)) in sections.iter().enumerate() {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        let offset = bytes.len() as u32;
        word(&mut bytes, 64 + index * 16, *kind);
        word(&mut bytes, 72 + index * 16, offset);
        word(&mut bytes, 76 + index * 16, data.len() as u32);
        bytes.extend(data);
    }
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn sphere_is_closed_smooth_checked_single_face_without_topological_edges() {
    let scene = sphere_scene();
    let mesh = scene.bodies()[0].mesh().unwrap();
    assert_eq!(mesh.header().face_count, 1);
    assert_eq!(mesh.header().edge_count, 0);
    let candidates = crate::silhouette::candidates(&mesh).unwrap();
    assert!(!candidates.is_empty());
    let (positions, indices) = sphere_geometry();
    let mut edge_uses = std::collections::BTreeMap::new();
    for triangle in indices {
        for [a, b] in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            *edge_uses.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
        }
        let [a, b, c] = triangle.map(|index| glam::Vec3::from_array(positions[index as usize]));
        assert!((b - a).cross(c - a).dot(a + b + c) > 0.0);
    }
    assert!(edge_uses.values().all(|count| *count == 2));
}
