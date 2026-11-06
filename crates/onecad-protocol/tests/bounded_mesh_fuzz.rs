//! Bounded, reproducible MESH1 admission mutations over real worker-produced seeds.
use onecad_protocol::mesh::{MeshLimits, MeshView};

#[test]
fn bounded_fuzz_checked_mesh_admission() {
    let started = std::time::Instant::now();
    let seeds: [&[u8]; 2] = [
        include_bytes!("../../../fixtures/render/edge-policy/box.mesh1"),
        include_bytes!("../../../fixtures/render/edge-policy/cylinder.mesh1"),
    ];
    let mut state = 0x4220_0c01_4d45_5348u64;
    for seed in seeds {
        MeshView::decode(seed, MeshLimits::default()).unwrap();
        for iteration in 0..4096 {
            let mut bytes = seed.to_vec();
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let offset = state as usize % bytes.len();
            if iteration % 3 == 0 {
                bytes.truncate(offset);
            } else {
                bytes[offset] ^= ((state >> 32) as u8) | 1;
            }
            if let Ok(mesh) = MeshView::decode(&bytes, MeshLimits::default()) {
                for row in 0..mesh.header().triangle_count as usize {
                    assert!(mesh.triangle(row).is_some());
                }
                for row in 0..mesh.header().edge_count as usize {
                    assert!(mesh.edge_range(row).is_some());
                }
            }
        }
    }
    eprintln!(
        "bounded-mesh-fuzz seed=42200c014d455348 cases=8192 elapsedMs={}",
        started.elapsed().as_millis()
    );
}
