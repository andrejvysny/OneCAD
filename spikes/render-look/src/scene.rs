//! A fixture directory merged into draw-ready arrays: one vertex/index set for every body
//! (positions uploaded verbatim, Z-up, no axis swap), feature-edge segments from EDGE_POSITIONS,
//! and silhouette candidates (interior edges of curved faces with both adjacent triangle normals).

use std::collections::HashMap;
use std::path::Path;

use bytemuck::{Pod, Zeroable};

use crate::mesh1::Mesh1;

/// Feature-edge segment instance: endpoints a, b.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Segment {
    pub a: [f32; 3],
    pub b: [f32; 3],
}

/// Silhouette candidate instance: an interior edge of a face and the geometric normals of the
/// two triangles sharing it. The vertex shader keeps it only where one faces the eye and the
/// other faces away.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SilEdge {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub n0: [f32; 3],
    pub n1: [f32; 3],
}

pub struct Scene {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub segments: Vec<Segment>,
    pub silhouettes: Vec<SilEdge>,
    pub bbox_min: glam::Vec3,
    pub bbox_max: glam::Vec3,
    pub bodies: usize,
    pub faces: usize,
    pub edges: usize,
}

fn tri_normal(p: &[[f32; 3]], t: [u32; 3]) -> glam::Vec3 {
    let v = |i: u32| glam::Vec3::from(p[i as usize]);
    (v(t[1]) - v(t[0])).cross(v(t[2]) - v(t[0])).normalize_or_zero()
}

/// Weld key: positions quantised to 0.1 µm, so a periodic face's seam (duplicated vertices
/// with equal positions) does not break the silhouette polyline.
fn weld_key(p: [f32; 3]) -> [i64; 3] {
    p.map(|c| (c as f64 * 1e4).round() as i64)
}

/// (first triangle, its endpoints, second triangle, triangle count) of a welded edge.
type EdgeUse = (usize, [u32; 2], usize, u32);

fn face_silhouettes(m: &Mesh1, first: u32, count: u32, out: &mut Vec<SilEdge>) {
    let tris = &m.indices[first as usize..(first + count) as usize];
    let mut canon: HashMap<[i64; 3], u32> = HashMap::new();
    let mut id = |v: u32| {
        let n = canon.len() as u32;
        *canon.entry(weld_key(m.positions[v as usize])).or_insert(n)
    };
    // edge (canonical a < b) -> (tri, endpoints, second tri, count)
    let mut edges: HashMap<(u32, u32), EdgeUse> = HashMap::new();
    for (k, t) in tris.iter().enumerate() {
        let c = [id(t[0]), id(t[1]), id(t[2])];
        for j in 0..3 {
            let (ia, ib) = (c[j], c[(j + 1) % 3]);
            if ia == ib {
                continue;
            }
            let key = (ia.min(ib), ia.max(ib));
            let e = edges.entry(key).or_insert((k, [t[j], t[(j + 1) % 3]], usize::MAX, 0));
            if e.3 == 1 {
                e.2 = k;
            }
            e.3 += 1;
        }
    }
    // HashMap order is random per process; sort so the candidate buffer is reproducible.
    let mut edges: Vec<_> = edges.into_iter().collect();
    edges.sort_unstable_by_key(|(k, _)| *k);
    for (_, (ta, ends, tb, n)) in edges {
        if n != 2 {
            continue;
        }
        let na = tri_normal(&m.positions, tris[ta]);
        let nb = tri_normal(&m.positions, tris[tb]);
        // Coplanar neighbours can never straddle the terminator: skip (planar faces drop out).
        if na.dot(nb) > 0.99999 {
            continue;
        }
        out.push(SilEdge { a: m.positions[ends[0] as usize], b: m.positions[ends[1] as usize], n0: na.into(), n1: nb.into() });
    }
}

impl Scene {
    pub fn load(dir: &Path) -> Result<Scene, String> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "mesh1"))
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(format!("{}: no .mesh1 files", dir.display()));
        }
        let mut s = Scene {
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            segments: Vec::new(),
            silhouettes: Vec::new(),
            bbox_min: glam::Vec3::splat(f32::INFINITY),
            bbox_max: glam::Vec3::splat(f32::NEG_INFINITY),
            bodies: files.len(),
            faces: 0,
            edges: 0,
        };
        for f in &files {
            let m = Mesh1::load(f)?;
            let base = s.positions.len() as u32;
            s.indices.extend(m.indices.iter().flatten().map(|i| i + base));
            s.positions.extend_from_slice(&m.positions);
            s.normals.extend_from_slice(&m.normals);
            s.bbox_min = s.bbox_min.min(m.bbox_min.into());
            s.bbox_max = s.bbox_max.max(m.bbox_max.into());
            for &(first, count) in &m.face_ranges {
                face_silhouettes(&m, first, count, &mut s.silhouettes);
            }
            s.faces += m.face_ranges.len();
            s.edges += m.edges.len();
            for poly in &m.edges {
                for w in poly.windows(2) {
                    s.segments.push(Segment { a: w[0], b: w[1] });
                }
            }
        }
        Ok(s)
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Triangles whose winding (CCW geometric normal) agrees with their vertex normals: the
    /// condition for shading back-facing fragments from their other side via `front_facing`.
    pub fn winding_agreement(&self) -> (usize, usize) {
        let mut agree = 0;
        for t in self.indices.chunks_exact(3) {
            let g = tri_normal(&self.positions, [t[0], t[1], t[2]]);
            let n: glam::Vec3 = t.iter().map(|&i| glam::Vec3::from(self.normals[i as usize])).sum();
            if g.dot(n) > 0.0 {
                agree += 1;
            }
        }
        (agree, self.triangles())
    }
}
