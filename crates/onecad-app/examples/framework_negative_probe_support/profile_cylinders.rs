//! Native quarter-circle protector profiles; dimensions come from negative::spec.
use super::*;
use serde_json::json;
use std::f64::consts::{FRAC_PI_2, PI};

const RADIUS: f64 = 0.3;
const REPLAY_FACTOR: f64 = 128.0;

#[derive(Debug)]
struct Profile {
    origin: [f64; 3],
    axis: [f64; 3],
    length: f64,
    sign: f64,
    height: f64,
    residuals: Vec<f64>,
}
fn replay_bound(a: f64, b: f64) -> f64 {
    REPLAY_FACTOR * f64::EPSILON * a.abs().max(b.abs()).max(1.0)
}
fn compare(a: f64, b: f64, residuals: &mut Vec<f64>) -> Result<(), &'static str> {
    if !a.is_finite() || !b.is_finite() {
        return Err("non-finite coordinate");
    }
    let residual = (a - b).abs();
    residuals.push(residual);
    if residual > replay_bound(a, b) {
        return Err("bounds do not replay");
    }
    Ok(())
}
fn plan(spec: negative::CylinderSpec) -> Result<Profile, &'static str> {
    let (lo, hi, origin, axis, length) = spec;
    if (0..3).any(|i| {
        !lo[i].is_finite() || !hi[i].is_finite() || !origin[i].is_finite() || lo[i] >= hi[i]
    }) {
        return Err("non-finite or inverted bounds");
    }
    if !length.is_finite() || length <= 0.0 {
        return Err("invalid length");
    }
    let (axial, transverse, factor) = if axis == [0.0, 1.0, 0.0] {
        (1, 2, -1.0)
    } else if axis == [0.0, 0.0, 1.0] {
        (2, 1, 1.0)
    } else {
        return Err("unsupported axis");
    };
    let mut residuals = Vec::new();
    compare(lo[0], origin[0], &mut residuals)?;
    compare(hi[0], origin[0] + RADIUS, &mut residuals)?;
    compare(lo[axial], origin[axial], &mut residuals)?;
    compare(hi[axial], origin[axial] + length, &mut residuals)?;
    let (side, height) = if (lo[transverse] - origin[transverse]).abs()
        <= replay_bound(lo[transverse], origin[transverse])
    {
        compare(lo[transverse], origin[transverse], &mut residuals)?;
        (1.0, hi[transverse] - origin[transverse])
    } else {
        compare(hi[transverse], origin[transverse], &mut residuals)?;
        (-1.0, origin[transverse] - lo[transverse])
    };
    let far = if side > 0.0 {
        hi[transverse]
    } else {
        lo[transverse]
    };
    compare(far, origin[transverse] + side * height, &mut residuals)?;
    if !height.is_finite() || height < RADIUS {
        return Err("height smaller than radius");
    }
    let sign = side * factor;
    let (start, end, start_xy, end_xy) = if sign > 0.0 {
        (0.0_f64, FRAC_PI_2, [RADIUS, 0.0], [0.0, RADIUS])
    } else {
        (-FRAC_PI_2, 0.0_f64, [0.0, -RADIUS], [RADIUS, 0.0])
    };
    for (angle, expected) in [(start, start_xy), (end, end_xy)] {
        compare(RADIUS * angle.cos(), expected[0], &mut residuals)?;
        compare(RADIUS * angle.sin(), expected[1], &mut residuals)?;
    }
    Ok(Profile {
        origin,
        axis,
        length,
        sign,
        height,
        residuals,
    })
}
fn expected_volume(p: &Profile) -> f64 {
    (RADIUS * p.height - PI * RADIUS * RADIUS / 4.0) * p.length
}
fn make_sketch(b: &mut Builder<'_>, p: &Profile) -> Sketch {
    let mut sketch = b.empty_sketch();
    let center = b.entity();
    sketch
        .add_entity(SketchEntity::point(
            center,
            Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    let arc = b.entity();
    let (start, end) = if p.sign > 0.0 {
        (0.0, FRAC_PI_2)
    } else {
        (-FRAC_PI_2, 0.0)
    };
    sketch
        .add_entity(SketchEntity::arc(arc, center, RADIUS, start, end, false).unwrap())
        .unwrap();
    let points = if p.sign > 0.0 {
        [
            [0.0, RADIUS],
            [0.0, p.height],
            [RADIUS, p.height],
            [RADIUS, 0.0],
        ]
    } else {
        [
            [RADIUS, 0.0],
            [RADIUS, -p.height],
            [0.0, -p.height],
            [0.0, -RADIUS],
        ]
    };
    let ids: Vec<_> = points
        .iter()
        .map(|xy| {
            let id = b.entity();
            sketch
                .add_entity(SketchEntity::point(
                    id,
                    Vec2::new(xy[0], xy[1]).unwrap(),
                    false,
                    false,
                ))
                .unwrap();
            id
        })
        .collect();
    for pair in ids.windows(2) {
        let id = b.entity();
        sketch
            .add_entity(SketchEntity::line(id, pair[0], pair[1], false))
            .unwrap();
    }
    sketch
}
fn record(out: &std::path::Path, name: &str, value: &serde_json::Value) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(out.join(name))
        .unwrap();
    serde_json::to_writer(&mut file, value).unwrap();
    writeln!(file).unwrap();
    file.flush().unwrap();
}
pub(super) async fn build(
    b: &mut Builder<'_>,
    bounds: solids::Bounds,
    index: usize,
    out: &std::path::Path,
) -> BodyId {
    let profile =
        plan(negative::spec(bounds, index)).expect("source cylinder protector profile bounds");
    let expected = expected_volume(&profile);
    let spec = negative::spec(bounds, index);
    record(
        out,
        "profile-plans.jsonl",
        &json!({"index":index,"lo":spec.0,"hi":spec.1,"origin":profile.origin,"axis":profile.axis,
        "length":profile.length,"sign":profile.sign,"height":profile.height,"cornerReplayResidualsMm":profile.residuals,
        "cornerReplayBound":"128 * f64::EPSILON * max(|a|, |b|, 1)","radiusMm":RADIUS,
        "expectedVolumeMm3":expected,"formula":"(r*H - pi*r*r/4)*length",
        "volumeComparison":"unqualified until native achieved integration error is measured"}),
    );
    let sketch = make_sketch(b, &profile);
    let x = [1.0, 0.0, 0.0];
    let y = geometry::cross(profile.axis, x);
    let region = b
        .sketch(sketch, frame(profile.origin, x, y, profile.axis))
        .await;
    let body = b
        .created(
            "Native quarter-circle protector",
            extrusion(region, profile.length),
        )
        .await;
    let query = api::query_mass_properties(
        &b.h.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    );
    let mass = tokio::time::timeout(std::time::Duration::from_secs(10), query)
        .await
        .unwrap()
        .unwrap();
    record(
        out,
        "profile-observations.jsonl",
        &json!({"index":index,"bodyId":body.to_string(),
        "expectedVolumeMm3":expected,"observedVolumeMm3":mass.volume,"residualMm3":mass.volume-expected,
        "mass":mass,"classification":"not qualified: public mass API reports no integration error"}),
    );
    body
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> negative::CylinderSpec {
        (
            [-13.5, 1.0, 2.0],
            [-13.2, 3.0, 4.0],
            [-13.5, 1.0, 2.0],
            [0.0, 1.0, 0.0],
            2.0,
        )
    }
    #[test]
    fn positive_y_axis_upper_maps_to_negative_v() {
        let p = plan(sample()).unwrap();
        assert_eq!(p.sign, -1.0);
        assert_eq!(p.height, 2.0);
        assert!(expected_volume(&p) > 0.0);
    }
    #[test]
    fn actual_five_source_specs_satisfy_profile_contract() {
        let bounds = negative::source_bounds();
        let signs = [-1.0, 1.0, 1.0, -1.0, 1.0];
        for (index, sign) in signs.into_iter().enumerate() {
            let p = plan(negative::spec(bounds, index)).unwrap();
            assert_eq!(p.sign, sign);
            assert!(expected_volume(&p) > 0.0);
        }
    }
    #[test]
    fn unsupported_axis_is_refused() {
        let mut s = sample();
        s.3 = [1.0, 0.0, 0.0];
        assert!(plan(s).is_err());
    }
    #[test]
    fn wrong_box_width_is_refused() {
        let mut s = sample();
        s.1[0] = -13.19;
        assert!(plan(s).is_err());
    }
    #[test]
    fn wrong_axial_length_is_refused() {
        let mut s = sample();
        s.4 = 1.0;
        assert!(plan(s).is_err());
    }
    #[test]
    fn insufficient_height_is_refused() {
        let mut s = sample();
        s.1[2] = 2.2;
        assert!(plan(s).is_err());
    }
}
