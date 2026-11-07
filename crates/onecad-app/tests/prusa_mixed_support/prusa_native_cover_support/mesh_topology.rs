use super::*;
use std::collections::{BTreeMap, HashMap};

pub(super) fn assert_upstream_topology(mesh: &MeshView<'_>, report: &serde_json::Value) {
    let mut vertices = HashMap::new();
    let mut edges = BTreeMap::<(usize, usize), (usize, i32)>::new();
    let mut triangles = Vec::with_capacity(mesh.header().triangle_count as usize);
    for index in 0..mesh.header().triangle_count as usize {
        let [a, b, c] = mesh.triangle(index).unwrap().map(|vertex| {
            let position = mesh.position(vertex as usize).unwrap();
            let key = position.map(|value| if value == 0.0 { 0 } else { value.to_bits() });
            let next = vertices.len();
            *vertices.entry(key).or_insert(next)
        });
        assert!(a != b && b != c && c != a, "degenerate mesh triangle");
        triangles.push([a, b, c]);
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let edge = edges.entry((from.min(to), from.max(to))).or_default();
            edge.0 += 1;
            edge.1 += if from < to { 1 } else { -1 };
        }
    }
    assert!(
        edges
            .values()
            .all(|&(count, winding)| count == 2 && winding == 0),
        "mesh edges must be closed with opposite winding"
    );
    let euler =
        vertices.len() as i64 - edges.len() as i64 + i64::from(mesh.header().triangle_count);
    assert_eq!(
        euler,
        report["observation"]["eulerCharacteristic"]
            .as_i64()
            .unwrap()
    );
    let components = component_count(&triangles);
    assert_eq!(
        components,
        report["observation"]["connectedComponents"]
            .as_u64()
            .unwrap() as usize
    );
    eprintln!("native mesh topology: {components} component(s), Euler {euler}");
}

fn component_count(triangles: &[[usize; 3]]) -> usize {
    let mut parent: Vec<_> = (0..triangles.len()).collect();
    let mut owners = HashMap::<(usize, usize), usize>::new();
    for (index, &[a, b, c]) in triangles.iter().enumerate() {
        for (from, to) in [(a, b), (b, c), (c, a)] {
            if let Some(&other) = owners.get(&(to, from)) {
                let current_root = root(&mut parent, index);
                let other_root = root(&mut parent, other);
                parent[current_root] = other_root;
            }
            owners.insert((from, to), index);
        }
    }
    (0..triangles.len())
        .filter(|&i| root(&mut parent, i) == i)
        .count()
}

fn root(parent: &mut [usize], mut vertex: usize) -> usize {
    while parent[vertex] != vertex {
        parent[vertex] = parent[parent[vertex]];
        vertex = parent[vertex];
    }
    vertex
}

#[cfg(test)]
mod tests {
    use super::component_count;

    #[test]
    fn closed_tetrahedron_has_one_edge_connected_component() {
        let tetra = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]];
        assert_eq!(component_count(&tetra), 1);
    }

    #[test]
    fn closed_tetrahedra_touching_only_at_vertex_remain_separate() {
        let tetrahedra = [
            [0, 2, 1],
            [0, 1, 3],
            [1, 2, 3],
            [2, 0, 3],
            [0, 5, 4],
            [0, 4, 6],
            [4, 5, 6],
            [5, 0, 6],
        ];
        assert_eq!(component_count(&tetrahedra), 2);
    }
}
