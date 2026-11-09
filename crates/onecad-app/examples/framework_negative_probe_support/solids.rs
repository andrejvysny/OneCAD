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

pub(super) async fn halfspace(
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
    if std::env::var_os("ONECAD_FRAMEWORK_LOFT_SHARP").is_some() {
        let upper = loft_cell(b, &p, [13.2, 16.0], true, 111).await;
        let lower_first = loft_cell(b, &p, [13.2, critical[0]], false, 83).await;
        let lower_second = loft_cell(b, &p, [critical[0], 16.0], false, 111).await;
        b.boolean(BooleanOp::Union, upper, vec![lower_first, lower_second])
            .await;
        return (upper, bounds);
    }
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

pub(super) async fn box_solid(b: &mut Builder<'_>, lo: [f64; 3], hi: [f64; 3]) -> BodyId {
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

pub(super) async fn cylinder(
    b: &mut Builder<'_>,
    origin: [f64; 3],
    axis: [f64; 3],
    length: f64,
) -> BodyId {
    let mut radius = 0.3;
    if std::env::var_os("ONECAD_FRAMEWORK_TANGENT_DATUM").is_some() {
        let candidates: Vec<_> = geometry::source_planes()
            .into_iter()
            .filter_map(|(id, plane)| {
                let normal = geometry::unit(plane.normal);
                let difference = std::array::from_fn(|i| origin[i] - plane.origin[i]);
                let distance = geometry::dot(normal, difference).abs();
                (geometry::dot(normal, axis).abs() < 1e-12 && (distance - radius).abs() < 1e-10)
                    .then_some((id, distance))
            })
            .collect();
        assert!(
            candidates.len() <= 1,
            "private native tangent datum is ambiguous"
        );
        if let Some((id, distance)) = candidates.first() {
            eprintln!("private native tangent datum plane {id}: source radius {radius}, derived {distance}; not literal STEP geometry");
            radius = *distance;
        }
    }
    let sketch = b.circle(radius);
    let x = [1., 0., 0.];
    let y = geometry::cross(axis, x);
    let profile = b.sketch(sketch, frame(origin, x, y, axis)).await;
    b.created("Protection cylinder", extrusion(profile, length))
        .await
}

pub(super) async fn sphere(b: &mut Builder<'_>, center: [f64; 3]) -> BodyId {
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

pub(super) async fn complement(
    b: &mut Builder<'_>,
    lo: [f64; 3],
    hi: [f64; 3],
    round: BodyId,
) -> BodyId {
    let target = box_solid(b, lo, hi).await;
    b.boolean(BooleanOp::Cut, target, vec![round]).await;
    target
}

type CylinderSpec = ([f64; 3], [f64; 3], [f64; 3], [f64; 3], f64);

fn cylinder_specs(bounds: Bounds) -> [CylinderSpec; 5] {
    let (lo, hi) = bounds;
    let (y0, y1, ys) = (28.07688212905, 31.00916730902, 30.29998857648);
    let (zl, zm, zh) = (1.700011423516, 2.800011423516, 3.920976627276);
    [
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
    ]
}

pub async fn protections(b: &mut Builder<'_>, bounds: Bounds) -> Vec<BodyId> {
    let mut output = Vec::new();
    for index in 0..7 {
        output.push(protection(b, bounds, index).await);
    }
    output
}

pub async fn protection(b: &mut Builder<'_>, bounds: Bounds, index: usize) -> BodyId {
    if index < 5 {
        let (a, z, origin, axis, length) = cylinder_specs(bounds)[index];
        let round = cylinder(b, origin, axis, length).await;
        return complement(b, a, z, round).await;
    }
    let (lo, hi) = bounds;
    let y0 = 28.07688212905;
    let (zl, zh) = (1.700011423516, 3.920976627276);
    let height = if index == 5 { zh } else { zl };
    let round = sphere(b, [13.5, y0, height]).await;
    let (bottom, top) = if index == 5 { (zh, hi[2]) } else { (lo[2], zl) };
    complement(b, [13.2, lo[1], bottom], [13.5, y0, top], round).await
}

pub async fn retain_piece(b: &mut Builder<'_>, target: BodyId, bounds: Bounds, index: usize) {
    let (lo, hi) = bounds;
    let y0 = 28.07688212905;
    let (zl, zh) = (1.700011423516, 3.920976627276);
    let (a, z, round) = if index < 5 {
        let (a, z, origin, axis, length) = cylinder_specs(bounds)[index];
        (a, z, cylinder(b, origin, axis, length).await)
    } else {
        let height = if index == 5 { zh } else { zl };
        let round = sphere(b, [13.5, y0, height]).await;
        let (bottom, top) = if index == 5 { (zh, hi[2]) } else { (lo[2], zl) };
        ([13.2, lo[1], bottom], [13.5, y0, top], round)
    };
    let slab = box_solid(b, a, z).await;
    b.boolean(BooleanOp::Intersect, target, vec![slab]).await;
    b.boolean(BooleanOp::Cut, target, vec![round]).await;
}

async fn loft_cell(
    b: &mut Builder<'_>,
    planes: &std::collections::BTreeMap<u64, geometry::Plane>,
    stations: [f64; 2],
    upper: bool,
    end_plane: u64,
) -> BodyId {
    let mut profiles = Vec::new();
    for station in stations {
        let points = geometry::sections(planes, station, upper, end_plane);
        let sketch = b.polygon(&points);
        profiles.push(
            b.sketch(
                sketch,
                frame([station, 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 0.]),
            )
            .await,
        );
    }
    b.created(
        "Source-plane ruled cell",
        Operation::Known(KnownOperation::Loft(LoftParams {
            profiles,
            is_solid: true,
            is_ruled: true,
            boolean_mode: BooleanMode::NewBody,
            extra: Default::default(),
        })),
    )
    .await
}
