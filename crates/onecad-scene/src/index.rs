//! OneCAD-owned deterministic median BVHs. No renderer or engine dependencies.
use crate::{Bounds, DVec2, DVec3, MeshView, Ray, ViewSnapshot};
use std::mem::size_of;

const LEAF: usize = 8;

#[derive(Debug)]
pub(crate) struct Primitive {
    pub topology: u32,
    pub element: u32,
    bounds: Bounds,
}

#[derive(Debug)]
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}

#[derive(Debug, Default)]
pub(crate) struct PickIndex {
    primitives: Vec<Primitive>,
    nodes: Vec<Node>,
}

impl PickIndex {
    pub fn allocation_bytes(count: usize) -> Option<usize> {
        count
            .checked_mul(size_of::<Primitive>())?
            .checked_add(node_count(count).checked_mul(size_of::<Node>())?)
    }

    pub fn build(mesh: &MeshView<'_>, edges: bool, count: usize) -> Self {
        let mut primitives = Vec::with_capacity(count);
        let topologies = if edges {
            mesh.header().edge_count
        } else {
            mesh.header().face_count
        };
        for topology in 0..topologies {
            let span = if edges {
                mesh.edge_range(topology as usize)
            } else {
                mesh.face_range(topology as usize)
            };
            let Some(span) = span else { continue };
            let end = span.first + span.count.saturating_sub(u32::from(edges));
            for element in span.first..end {
                if let Some(primitive) = primitive(mesh, topology, element, edges) {
                    primitives.push(primitive);
                }
            }
        }
        let mut index = Self {
            primitives,
            nodes: Vec::with_capacity(node_count(count)),
        };
        if !index.primitives.is_empty() {
            index.split(0, index.primitives.len());
        }
        index
    }

    fn split(&mut self, start: usize, end: usize) -> usize {
        let mut bounds = self.primitives[start].bounds;
        for p in &self.primitives[start + 1..end] {
            bounds.min = bounds.min.min(p.bounds.min);
            bounds.max = bounds.max.max(p.bounds.max);
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            end,
            children: None,
        });
        if end - start > LEAF {
            let extent = bounds.max - bounds.min;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            self.primitives[start..end].sort_unstable_by(|a, b| {
                (a.bounds.min[axis] + a.bounds.max[axis])
                    .total_cmp(&(b.bounds.min[axis] + b.bounds.max[axis]))
                    .then_with(|| a.topology.cmp(&b.topology))
                    .then_with(|| a.element.cmp(&b.element))
            });
            let middle = start + (end - start) / 2;
            let left = self.split(start, middle);
            let right = self.split(middle, end);
            self.nodes[index].children = Some((left, right));
        }
        index
    }

    pub fn visit(&self, accepts: impl Fn(Bounds) -> bool, mut visit: impl FnMut(&Primitive)) {
        if !self.nodes.is_empty() {
            self.visit_node(0, &accepts, &mut visit);
        }
    }

    fn visit_node(
        &self,
        index: usize,
        accepts: &impl Fn(Bounds) -> bool,
        visit: &mut impl FnMut(&Primitive),
    ) {
        let node = &self.nodes[index];
        if !accepts(node.bounds) {
            return;
        }
        if let Some((left, right)) = node.children {
            self.visit_node(left, accepts, visit);
            self.visit_node(right, accepts, visit);
        } else {
            for primitive in &self.primitives[node.start..node.end] {
                visit(primitive);
            }
        }
    }
}

fn primitive(mesh: &MeshView<'_>, topology: u32, element: u32, edges: bool) -> Option<Primitive> {
    let points = if edges {
        [
            mesh.edge_point(element as usize),
            mesh.edge_point(element as usize + 1),
            mesh.edge_point(element as usize),
        ]
    } else {
        mesh.triangle(element as usize)?
            .map(|i| mesh.position(i as usize))
    };
    let [a, b, c] =
        [points[0]?, points[1]?, points[2]?].map(|p| DVec3::from_array(p.map(f64::from)));
    Some(Primitive {
        topology,
        element,
        bounds: Bounds {
            min: a.min(b).min(c),
            max: a.max(b).max(c),
        },
    })
}

fn node_count(count: usize) -> usize {
    if count == 0 {
        0
    } else if count <= LEAF {
        1
    } else {
        1 + node_count(count / 2) + node_count(count - count / 2)
    }
}

pub(crate) fn ray_bounds(ray: Ray, bounds: Bounds) -> bool {
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for axis in 0..3 {
        if ray.direction[axis] == 0.0 {
            if ray.origin[axis] < bounds.min[axis] || ray.origin[axis] > bounds.max[axis] {
                return false;
            }
        } else {
            let a = (bounds.min[axis] - ray.origin[axis]) / ray.direction[axis];
            let b = (bounds.max[axis] - ray.origin[axis]) / ray.direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    true
}

pub(crate) fn screen_bounds(
    view: &ViewSnapshot,
    cursor: DVec2,
    tolerance: f64,
    bounds: Bounds,
) -> bool {
    let (mut near, mut far) = (f64::INFINITY, f64::NEG_INFINITY);
    for corner in bounds.corners() {
        let depth = view.depth(corner);
        near = near.min(depth);
        far = far.max(depth);
    }
    if far < view.camera().near || near > view.camera().far {
        return false;
    }
    // A box crossing the near plane may project without a finite enclosing rectangle.
    if near <= view.camera().near {
        return true;
    }
    let (mut min, mut max) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for corner in bounds.corners() {
        let Some(projected) = view.project_unclipped(corner) else {
            return true;
        };
        min = min.min(projected.logical);
        max = max.max(projected.logical);
    }
    let aperture = DVec2::splat(tolerance);
    cursor.cmpge(min - aperture).all() && cursor.cmple(max + aperture).all()
}
