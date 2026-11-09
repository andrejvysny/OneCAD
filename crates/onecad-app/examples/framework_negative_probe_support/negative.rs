//! Private negative-side construction from the source's own left-side faces.
use super::*;
use solids::{box_solid, complement, cylinder, halfspace, sphere, Bounds};

async fn cell(b: &mut Builder<'_>, limits: &[(u64, f64)]) -> BodyId {
    let planes = geometry::source_planes();
    let lo = [-16., -100., -100.];
    let hi = [-13.2, 100., 100.];
    let target = box_solid(b, lo, hi).await;
    for &(id, sign) in limits {
        let tool = halfspace(b, planes[&id], sign, lo, hi).await;
        b.boolean(BooleanOp::Intersect, target, vec![tool]).await;
    }
    target
}

pub async fn sharp(b: &mut Builder<'_>) -> (BodyId, Bounds) {
    let target = cell(b, &[(65, 1.), (34, -1.), (38, 1.), (72, 1.)]).await;
    let lower = cell(b, &[(65, 1.), (38, 1.), (40, 1.), (41, -1.), (72, -1.)]).await;
    b.boolean(BooleanOp::Union, target, vec![lower]).await;
    (target, source_bounds())
}

pub(super) fn source_bounds() -> Bounds {
    let p = geometry::source_planes();
    let critical = geometry::triple_intersection(p[&65], p[&41], p[&72]);
    assert!(critical[0] < -13.2 && critical[0] > -16.);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (x, end, upper) in [
        (-13.2, 65, true),
        (-16., 65, true),
        (-13.2, 41, false),
        (critical[0], 41, false),
        (critical[0], 65, false),
        (-16., 65, false),
    ] {
        let y = |id| (p[&id].offset - p[&id].normal[0] * x) / p[&id].normal[1];
        for yy in [y(end), y(38)] {
            for id in if upper { [72, 34] } else { [40, 72] } {
                let plane = p[&id];
                let z =
                    (plane.offset - plane.normal[0] * x - plane.normal[1] * yy) / plane.normal[2];
                for (i, value) in [x, yy, z].into_iter().enumerate() {
                    lo[i] = lo[i].min(value);
                    hi[i] = hi[i].max(value);
                }
            }
        }
    }
    let diagonal = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt();
    (lo.map(|v| v - diagonal), hi.map(|v| v + diagonal))
}

pub(super) type CylinderSpec = ([f64; 3], [f64; 3], [f64; 3], [f64; 3], f64);

pub(super) fn spec(bounds: Bounds, index: usize) -> CylinderSpec {
    let (lo, hi) = bounds;
    // Source face 64 gives y1; faces 56, 62, 68, 69 and 70 give the axes.
    let (y0, y1, ys) = (28.07688212905, 31.00916730869, 30.29998857648);
    let (zl, zm, zh) = (1.700011423516, 2.800011423516, 3.920976627276);
    match index {
        0 => (
            [-13.5, y0, zh],
            [-13.2, y1, hi[2]],
            [-13.5, y0, zh],
            [0., 1., 0.],
            y1 - y0,
        ),
        1 => (
            [-13.5, ys, lo[2]],
            [-13.2, y1, zm],
            [-13.5, ys, zm],
            [0., 1., 0.],
            y1 - ys,
        ),
        2 => (
            [-13.5, y0, lo[2]],
            [-13.2, ys, zl],
            [-13.5, y0, zl],
            [0., 1., 0.],
            ys - y0,
        ),
        3 => (
            [-13.5, lo[1], zl],
            [-13.2, y0, zh],
            [-13.5, y0, zl],
            [0., 0., 1.],
            zh - zl,
        ),
        4 => (
            [-13.5, ys, zl],
            [-13.2, hi[1], zm],
            [-13.5, ys, zl],
            [0., 0., 1.],
            zm - zl,
        ),
        _ => panic!("invalid cylinder index"),
    }
}

async fn primitive(
    b: &mut Builder<'_>,
    bounds: Bounds,
    index: usize,
) -> (BodyId, [f64; 3], [f64; 3]) {
    if index < 5 {
        let (lo, hi, origin, axis, length) = spec(bounds, index);
        return (cylinder(b, origin, axis, length).await, lo, hi);
    }
    let (lo, hi) = bounds;
    let y0 = 28.07688212905;
    let (zl, zh) = (1.700011423516, 3.920976627276);
    let center_z = if index == 5 { zh } else { zl };
    let (bottom, top) = if index == 5 { (zh, hi[2]) } else { (lo[2], zl) };
    let shape = sphere(b, [-13.5, y0, center_z]).await;
    (shape, [-13.5, lo[1], bottom], [-13.2, y0, top])
}

pub async fn protection(b: &mut Builder<'_>, bounds: Bounds, index: usize) -> BodyId {
    let (round, lo, hi) = primitive(b, bounds, index).await;
    complement(b, lo, hi, round).await
}

pub async fn retain_piece(
    b: &mut Builder<'_>,
    target: BodyId,
    bounds: Bounds,
    index: usize,
    sharp: BodyId,
    original: BodyId,
    out: &std::path::Path,
) -> Vec<BodyId> {
    let (round, lo, hi) = primitive(b, bounds, index).await;
    let slab = box_solid(b, lo, hi).await;
    let mut bodies = b
        .boolean_results(BooleanOp::Intersect, target, vec![slab], false)
        .await;
    if index == 6 {
        capture_sphere_inputs(b, original, sharp, round, &bodies, out).await;
    }
    if matches!(index, 1 | 6) {
        let mut restricted = Vec::new();
        for body in bodies {
            restricted.extend(
                b.boolean_results(BooleanOp::Intersect, body, vec![sharp], true)
                    .await,
            );
        }
        bodies = restricted;
    }
    let count = bodies.len();
    let mut result = Vec::new();
    for (i, body) in bodies.into_iter().enumerate() {
        result.extend(
            b.boolean_results(BooleanOp::Cut, body, vec![round], i + 1 < count)
                .await,
        );
    }
    result
}

async fn capture_sphere_inputs(
    b: &Builder<'_>,
    original: BodyId,
    sharp: BodyId,
    sphere: BodyId,
    pieces: &[BodyId],
    out: &std::path::Path,
) {
    use sha2::{Digest, Sha256};
    let dir = out.join("sphere-retention-inputs");
    std::fs::create_dir(&dir).unwrap();
    let worker = b.h.state.live_worker().unwrap();
    let snapshot = api::preview_context(&b.h.state).await.unwrap().snapshot_id;
    let mut rows = Vec::new();
    for (index, body) in [original, sharp, sphere]
        .into_iter()
        .chain(pieces.iter().copied())
        .enumerate()
    {
        let path = dir.join(format!("input-{index}.brep"));
        let result = worker
            .export_geometry(
                path.to_str().unwrap(),
                &[body],
                "brep",
                false,
                Some(snapshot),
            )
            .await
            .unwrap();
        assert_eq!(result.solid_count, 1);
        rows.push(serde_json::json!({"bodyId":body.to_string(),"path":path,
            "sha256":format!("{:x}",Sha256::digest(std::fs::read(&path).unwrap())),"bytes":result.bytes_written}));
    }
    std::fs::write(dir.join("manifest.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "order":"B, S, Sphere, each P=B intersect slab", "snapshotId":snapshot,
        "sourceSha256":crate::SOURCE_SHA,"workerSha256":crate::WORKER_SHA,"files":rows,
        "limits":"Accepted production-worker operands; no containment or source-equivalence proof."})).unwrap()).unwrap();
}
