use super::*;
use std::collections::{BTreeMap, HashMap};

pub(super) fn assert_upstream_topology(mesh: &MeshView<'_>) {
    let source = source_mesh_report();
    let mut vertices = HashMap::new();
    let mut edges = BTreeMap::<(usize, usize), (usize, i32)>::new();
    for index in 0..mesh.header().triangle_count as usize {
        let [a, b, c] = mesh.triangle(index).unwrap().map(|vertex| {
            let position = mesh.position(vertex as usize).unwrap();
            // Signed zero denotes one location; no geometric weld tolerance is used.
            let key = position.map(|value| if value == 0.0 { 0 } else { value.to_bits() });
            let next = vertices.len();
            *vertices.entry(key).or_insert(next)
        });
        assert!(a != b && b != c && c != a, "degenerate mesh triangle");
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let edge = edges.entry((from.min(to), from.max(to))).or_default();
            edge.0 += 1;
            edge.1 += if from < to { 1 } else { -1 };
        }
    }
    assert!(edges
        .values()
        .all(|&(count, winding)| count == 2 && winding == 0));
    let euler =
        vertices.len() as i64 - edges.len() as i64 + i64::from(mesh.header().triangle_count);
    eprintln!("native mesh Euler characteristic {euler}");
    assert_eq!(
        euler,
        source["observation"]["eulerCharacteristic"]
            .as_i64()
            .unwrap()
    );
}
