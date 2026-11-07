use super::*;

pub type Bounds = ([f64; 3], [f64; 3]);

fn box_corners(lo: [f64; 3], hi: [f64; 3]) -> Vec<[f64; 3]> {
    let mut corners = Vec::new();
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                corners.push([x, y, z]);
            }
        }
    }
    corners
}

async fn halfspace(
    b: &mut Builder<'_>,
    plane: geometry::Plane,
    sign: f64,
    lo: [f64; 3],
    hi: [f64; 3],
) -> BodyId {
    let n = geometry::unit(plane.normal).map(|v| v * sign);
    let reference = if n[2].abs() < 0.9 {
        [0., 0., 1.]
    } else {
        [0., 1., 0.]
    };
    let x = geometry::unit(geometry::cross(reference, n));
    let y = geometry::unit(geometry::cross(n, x));
    let corners = box_corners(lo, hi);
    let diagonal = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt();
    let projections: Vec<_> = corners
        .iter()
        .map(|p| {
            let relative = std::array::from_fn(|i| p[i] - plane.origin[i]);
            [
                geometry::dot(relative, x),
                geometry::dot(relative, y),
                geometry::dot(relative, n),
            ]
        })
        .collect();
    let extent = |i: usize| {
        (
            projections
                .iter()
                .map(|p| p[i])
                .fold(f64::INFINITY, f64::min)
                - diagonal,
            projections
                .iter()
                .map(|p| p[i])
                .fold(f64::NEG_INFINITY, f64::max)
                + diagonal,
        )
    };
    let (u0, u1) = extent(0);
    let (v0, v1) = extent(1);
    let (_, depth) = extent(2);
    let sketch = b.polygon(&[[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
    let profile = b.sketch(sketch, frame(plane.origin, x, y, n)).await;
    b.created("Source-plane halfspace prism", extrusion(profile, depth))
        .await
}

async fn cell(
    b: &mut Builder<'_>,
    planes: &std::collections::BTreeMap<u64, geometry::Plane>,
    limits: &[(u64, f64)],
) -> BodyId {
    let lo = [13.2, -100., -100.];
    let hi = [16., 100., 100.];
    let target = box_solid(b, lo, hi).await;
    for (id, sign) in limits {
        let prism = halfspace(b, planes[id], *sign, lo, hi).await;
        b.boolean(BooleanOp::Intersect, target, vec![prism]).await;
    }
    target
}

pub async fn sharp(b: &mut Builder<'_>) -> (BodyId, Bounds) {
    let p = geometry::source_planes();
    let critical = geometry::triple_intersection(p[&111], p[&83], p[&81]);
    assert!(critical[0] > 13.2 && critical[0] < 16.0);
    let upper = [13.2, 16.0].map(|x| (x, geometry::sections(&p, x, true, 111)));
    let lower_first = [13.2, critical[0]].map(|x| (x, geometry::sections(&p, x, false, 83)));
    let lower_second = [critical[0], 16.0].map(|x| (x, geometry::sections(&p, x, false, 111)));
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (x, points) in upper.iter().chain(&lower_first).chain(&lower_second) {
        for point in points {
            for (i, value) in [*x, point[0], point[1]].into_iter().enumerate() {
                lo[i] = lo[i].min(value);
                hi[i] = hi[i].max(value);
            }
        }
    }
    let diag = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt();
    let bounds = (lo.map(|v| v - diag), hi.map(|v| v + diag));
    let target = cell(b, &p, &[(111, -1.), (90, -1.), (86, 1.), (81, 1.)]).await;
    let lower = cell(
        b,
        &p,
        &[(111, -1.), (86, 1.), (84, 1.), (83, -1.), (81, -1.)],
    )
    .await;
    b.boolean(BooleanOp::Union, target, vec![lower]).await;
    (target, bounds)
}

async fn box_solid(b: &mut Builder<'_>, lo: [f64; 3], hi: [f64; 3]) -> BodyId {
    assert!((0..3).all(|i| hi[i] > lo[i]));
    let sketch = b.polygon(&[
        [lo[0], lo[1]],
        [hi[0], lo[1]],
        [hi[0], hi[1]],
        [lo[0], hi[1]],
    ]);
    let profile = b
        .sketch(
            sketch,
            frame([0., 0., lo[2]], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]),
        )
        .await;
    b.created("Protection box", extrusion(profile, hi[2] - lo[2]))
        .await
}

async fn cylinder(b: &mut Builder<'_>, origin: [f64; 3], axis: [f64; 3], length: f64) -> BodyId {
    let sketch = b.circle(0.3);
    let x = [1., 0., 0.];
    let y = geometry::cross(axis, x);
    let profile = b.sketch(sketch, frame(origin, x, y, axis)).await;
    b.created("Protection cylinder", extrusion(profile, length))
        .await
}

async fn sphere(b: &mut Builder<'_>, center: [f64; 3]) -> BodyId {
    let mut sketch = b.empty_sketch();
    let c = b.entity();
    let low = b.entity();
    let high = b.entity();
    let arc = b.entity();
    let line = b.entity();
    for (id, p) in [(c, [0., 0.]), (low, [0., -0.3]), (high, [0., 0.3])] {
        sketch
            .add_entity(SketchEntity::point(
                id,
                Vec2::new(p[0], p[1]).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    sketch
        .add_entity(
            SketchEntity::arc(
                arc,
                c,
                0.3,
                -std::f64::consts::FRAC_PI_2,
                std::f64::consts::FRAC_PI_2,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    sketch
        .add_entity(SketchEntity::line(line, high, low, false))
        .unwrap();
    let sid = sketch.id;
    let profile = b
        .sketch(
            sketch,
            frame(center, [1., 0., 0.], [0., 0., 1.], [0., -1., 0.]),
        )
        .await;
    b.created(
        "Protection sphere",
        Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: Some(profile),
            angle_deg: Scalar::new(360.),
            axis: Some(AxisRef::SketchLine {
                sketch: sid,
                line,
                extra: Default::default(),
            }),
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            extra: Default::default(),
        })),
    )
    .await
}

async fn complement(b: &mut Builder<'_>, lo: [f64; 3], hi: [f64; 3], round: BodyId) -> BodyId {
    let target = box_solid(b, lo, hi).await;
    b.boolean(BooleanOp::Cut, target, vec![round]).await;
    target
}

pub async fn protections(b: &mut Builder<'_>, bounds: Bounds) -> Vec<BodyId> {
    let (lo, hi) = bounds;
    let (y0, y1, ys) = (28.07688212905, 31.00916730902, 30.29998857648);
    let (zl, zm, zh) = (1.700011423516, 2.800011423516, 3.920976627276);
    let specs = [
        (
            [13.2, y0, zh],
            [13.5, y1, hi[2]],
            [13.5, y0, zh],
            [0., 1., 0.],
            y1 - y0,
        ),
        (
            [13.2, ys, lo[2]],
            [13.5, y1, zm],
            [13.5, ys, zm],
            [0., 1., 0.],
            y1 - ys,
        ),
        (
            [13.2, y0, lo[2]],
            [13.5, ys, zl],
            [13.5, y0, zl],
            [0., 1., 0.],
            ys - y0,
        ),
        (
            [13.2, lo[1], zl],
            [13.5, y0, zh],
            [13.5, y0, zl],
            [0., 0., 1.],
            zh - zl,
        ),
        (
            [13.2, ys, zl],
            [13.5, hi[1], zm],
            [13.5, ys, zl],
            [0., 0., 1.],
            zm - zl,
        ),
    ];
    let mut output = Vec::new();
    for (a, z, origin, axis, length) in specs {
        let round = cylinder(b, origin, axis, length).await;
        output.push(complement(b, a, z, round).await);
    }
    let top = sphere(b, [13.5, y0, zh]).await;
    output.push(complement(b, [13.2, lo[1], zh], [13.5, y0, hi[2]], top).await);
    let bottom = sphere(b, [13.5, y0, zl]).await;
    output.push(complement(b, [13.2, lo[1], lo[2]], [13.5, y0, zl], bottom).await);
    output
}
