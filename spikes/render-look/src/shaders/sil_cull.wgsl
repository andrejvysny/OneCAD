// Silhouette compaction: keep the candidate edges whose two triangles straddle the terminator
// for this eye, append them as plain segments and count them into an indirect draw. The main
// pass then draws only the few thousand visible silhouette segments instead of running the
// line vertex shader over every candidate.

struct Args {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
};

// Candidates: 12 floats each (a, b, n0, n1), tightly packed like scene::SilEdge.
@group(1) @binding(0) var<storage, read> cand: array<f32>;
// Output segments: 6 floats each (a, b), the edge instance layout.
@group(1) @binding(1) var<storage, read_write> segs: array<f32>;
@group(1) @binding(2) var<storage, read_write> args: Args;

fn v3(i: u32) -> vec3<f32> {
    return vec3<f32>(cand[i], cand[i + 1u], cand[i + 2u]);
}

@compute @workgroup_size(64)
fn cs_sil(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * 65535u * 64u;
    if (i >= arrayLength(&cand) / 12u) {
        return;
    }
    let b = i * 12u;
    let a = v3(b);
    let c = v3(b + 3u);
    let e = u.eye.xyz - (a + c) * 0.5;
    if (dot(v3(b + 6u), e) * dot(v3(b + 9u), e) > 0.0) {
        return;
    }
    let k = atomicAdd(&args.instance_count, 1u) * 6u;
    segs[k] = a.x;
    segs[k + 1u] = a.y;
    segs[k + 2u] = a.z;
    segs[k + 3u] = c.x;
    segs[k + 4u] = c.y;
    segs[k + 5u] = c.z;
}
