use super::*;
use std::f64::consts::{FRAC_PI_2, PI};

fn p(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}
fn near(a: f64, b: f64) {
    assert!(
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
        "{a} != {b}"
    );
}
fn same(a: Vec2, b: Vec2) {
    near(a.x, b.x);
    near(a.y, b.y);
}
fn position(arc: &AnalyticArc, role: CurvePosition) -> Vec2 {
    let angle = match role {
        CurvePosition::Start => arc.start_angle,
        CurvePosition::End => arc.end_angle,
        _ => panic!("endpoint role required"),
    };
    p(
        arc.center.x + arc.radius * angle.cos(),
        arc.center.y + arc.radius * angle.sin(),
    )
}
fn verify(arc: &AnalyticArc, start: Vec2, end: Vec2) {
    assert!(arc.center.is_finite());
    assert!(arc.radius.is_finite() && arc.radius >= MIN_SIZE);
    assert!(arc.start_angle.is_finite() && arc.end_angle.is_finite());
    assert!(arc.sweep > 0.0 && arc.sweep < TAU);
    near(arc.end_angle - arc.start_angle, arc.sweep);
    for (role, expected) in [(arc.travel_start_role, start), (arc.travel_end_role, end)] {
        let error = difference(position(arc, role), expected).unwrap();
        assert!(length(error).unwrap() / arc.radius < 1e-9);
    }
    near(length(arc.exit_tangent).unwrap(), 1.0);
}
fn contains(arc: &AnalyticArc, point: Vec2) {
    near(
        length(difference(point, arc.center).unwrap()).unwrap(),
        arc.radius,
    );
    let angle = (point.y - arc.center.y).atan2(point.x - arc.center.x);
    assert!((angle - arc.start_angle).rem_euclid(TAU) <= arc.sweep + 1e-12);
}

#[test]
fn center_start_end_projects_cursor_without_changing_radius() {
    let arc = center_start_end(p(2.0, 3.0), p(7.0, 3.0), p(2.0, 100.0)).unwrap();
    same(arc.center, p(2.0, 3.0));
    near(arc.radius, 5.0);
    near(arc.sweep, FRAC_PI_2);
    assert!(!arc.clockwise);
    verify(&arc, p(7.0, 3.0), p(2.0, 8.0));
    same(arc.exit_tangent, p(-1.0, 0.0));
}

#[test]
fn center_start_end_keeps_counterclockwise_major_arc() {
    let arc = center_start_end(p(0.0, 0.0), p(10.0, 0.0), p(0.0, -2.0)).unwrap();
    near(arc.sweep, 3.0 * FRAC_PI_2);
    verify(&arc, p(10.0, 0.0), p(0.0, -10.0));
}

#[test]
fn three_point_frozen_cases_retain_all_points_and_selected_sweep() {
    for (start, end, through) in [
        (p(0.0, 0.0), p(10.0, 0.0), p(5.0, 5.0)),
        (p(0.0, 0.0), p(10.0, 0.0), p(5.0, -5.0)),
        (p(0.0, 0.0), p(10.0, 0.0), p(1.0, 0.2)),
        (p(0.0, 0.0), p(10.0, 0.0), p(9.0, -0.1)),
        (p(-4.0, 3.0), p(12.0, -7.0), p(2.0, 30.0)),
        (p(-4.0, 3.0), p(12.0, -7.0), p(2.0, -30.0)),
        (p(1000.0, 1000.0), p(-1000.0, 900.0), p(0.0, 1500.0)),
    ] {
        let arc = through_points(start, end, through).unwrap();
        verify(&arc, start, end);
        contains(&arc, through);
    }
}

#[test]
fn three_point_clockwise_swap_and_major_arc_are_explicit() {
    let upper = through_points(p(0.0, 0.0), p(10.0, 0.0), p(5.0, 5.0)).unwrap();
    assert!(upper.clockwise);
    assert_eq!(upper.travel_start_role, CurvePosition::End);
    assert_eq!(upper.travel_end_role, CurvePosition::Start);
    near(upper.sweep, PI);
    same(upper.center, p(5.0, 0.0));
    let lower = through_points(p(0.0, 0.0), p(10.0, 0.0), p(5.0, -5.0)).unwrap();
    assert!(!lower.clockwise);
    let major = through_points(p(10.0, 0.0), p(0.0, 10.0), p(-10.0, 0.0)).unwrap();
    assert!(major.clockwise);
    near(major.sweep, 3.0 * FRAC_PI_2);
    contains(&major, p(-10.0, 0.0));
}

#[test]
fn three_point_circumcenter_is_order_independent() {
    let points = [p(-3.0, 1.0), p(4.0, 7.0), p(9.0, -2.0)];
    let first = through_points(points[0], points[1], points[2]).unwrap();
    for (a, b, c) in [(1, 2, 0), (2, 0, 1), (2, 1, 0)] {
        let other = through_points(points[a], points[b], points[c]).unwrap();
        same(other.center, first.center);
        near(other.radius, first.radius);
    }
}

#[test]
fn tangent_half_turns_preserve_travel_and_swap_roles() {
    for sign in [1.0, -1.0] {
        let arc = tangent_to(p(0.0, 0.0), p(1.0, 0.0), p(0.0, sign * 10.0)).unwrap();
        same(arc.center, p(0.0, sign * 5.0));
        near(arc.radius, 5.0);
        near(arc.sweep, PI);
        assert_eq!(arc.clockwise, sign < 0.0);
        verify(&arc, p(0.0, 0.0), p(0.0, sign * 10.0));
        same(arc.exit_tangent, p(-1.0, 0.0));
    }
}

#[test]
fn tangent_frozen_cases_match_anchor_direction_and_exit_tangent() {
    for (anchor, tangent, end) in [
        (p(0.0, 0.0), p(1.0, 0.0), p(3.0, 7.0)),
        (p(0.0, 0.0), p(1.0, 0.0), p(3.0, -7.0)),
        (p(-5.0, 2.0), p(0.6, 0.8), p(30.0, -12.0)),
        (p(-5.0, 2.0), p(-0.6, 0.8), p(-30.0, -12.0)),
        (p(120.0, -40.0), p(0.0, 1.0), p(90.0, -80.0)),
    ] {
        let arc = tangent_to(anchor, tangent, end).unwrap();
        verify(&arc, anchor, end);
        let radial = unit(
            difference(anchor, arc.center).unwrap(),
            ArcGeometryError::ZeroTangent,
        )
        .unwrap();
        let sign = if arc.clockwise { -1.0 } else { 1.0 };
        same(p(-radial.y * sign, radial.x * sign), tangent);
        let exit_radial = unit(
            difference(end, arc.center).unwrap(),
            ArcGeometryError::ZeroTangent,
        )
        .unwrap();
        same(
            arc.exit_tangent,
            p(-exit_radial.y * sign, exit_radial.x * sign),
        );
    }
}

#[test]
fn tangent_magnitude_is_irrelevant_even_for_extreme_finite_scales() {
    let expected = tangent_to(p(0.0, 0.0), p(1.0, 0.0), p(3.0, 7.0)).unwrap();
    for scale in [250.0, 1e308, 1e-308, f64::from_bits(1)] {
        assert_eq!(
            tangent_to(p(0.0, 0.0), p(scale, 0.0), p(3.0, 7.0)).unwrap(),
            expected
        );
    }
}

#[test]
fn tangent_chaining_uses_actual_exit_tangent() {
    let first = tangent_to(p(0.0, 0.0), p(1.0, 0.0), p(20.0, 12.0)).unwrap();
    let second = tangent_to(p(20.0, 12.0), first.exit_tangent, p(35.0, -4.0)).unwrap();
    let normal1 = unit(
        difference(first.center, p(20.0, 12.0)).unwrap(),
        ArcGeometryError::ZeroTangent,
    )
    .unwrap();
    let normal2 = unit(
        difference(second.center, p(20.0, 12.0)).unwrap(),
        ArcGeometryError::ZeroTangent,
    )
    .unwrap();
    near(cross(normal1, normal2), 0.0);
    assert!(cross(first.exit_tangent, p(20.0, 12.0)).abs() > 1.0);
}

#[test]
fn duplicate_collinear_and_below_minimum_geometry_refuse() {
    for (a, b, c) in [
        (p(0.0, 0.0), p(0.0, 0.0), p(1.0, 1.0)),
        (p(0.0, 0.0), p(1.0, 1.0), p(2.0, 2.0)),
        (p(0.0, 0.0), p(10.0, 0.0), p(5.0, 1e-10)),
        (p(0.0, 0.0), p(1e-10, 0.0), p(0.0, 1e-10)),
    ] {
        assert!(through_points(a, b, c).is_err());
    }
    assert!(center_start_end(p(0.0, 0.0), p(1e-10, 0.0), p(0.0, 1.0)).is_err());
}

#[test]
fn zero_full_sweep_and_center_cursor_refuse() {
    for end in [p(1.0, 0.0), p(10.0, 0.0), p(0.0, 0.0)] {
        assert!(center_start_end(p(0.0, 0.0), p(1.0, 0.0), end).is_err());
    }
    assert!(tangent_to(p(0.0, 0.0), p(0.0, 0.0), p(1.0, 1.0)).is_err());
    for end in [p(0.0, 0.0), p(10.0, 0.0), p(-10.0, 0.0)] {
        assert!(tangent_to(p(0.0, 0.0), p(1.0, 0.0), end).is_err());
    }
}

#[test]
fn nonfinite_and_overflow_inputs_never_escape_as_results() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            center_start_end(p(bad, 0.0), p(1.0, 0.0), p(0.0, 1.0)),
            Err(ArcGeometryError::NonFinite)
        );
        assert_eq!(
            through_points(p(0.0, 0.0), p(1.0, bad), p(0.0, 1.0)),
            Err(ArcGeometryError::NonFinite)
        );
        assert_eq!(
            tangent_to(p(0.0, 0.0), p(1.0, bad), p(0.0, 1.0)),
            Err(ArcGeometryError::NonFinite)
        );
    }
    assert!(through_points(p(-1e308, 0.0), p(1e308, 0.0), p(0.0, 1.0)).is_err());
    assert!(through_points(p(0.0, 0.0), p(1e308, 0.0), p(5e307, 1e-9)).is_err());
    assert!(tangent_to(p(0.0, 0.0), p(1.0, 0.0), p(1e308, 1e-9)).is_err());
}

#[test]
fn scaled_local_fit_handles_large_and_translated_coordinates() {
    let arc = through_points(p(1e150, 0.0), p(0.0, 1e150), p(-1e150, 0.0)).unwrap();
    near(arc.radius, 1e150);
    verify(&arc, p(1e150, 0.0), p(0.0, 1e150));
    contains(&arc, p(-1e150, 0.0));
    let offset = 1e12;
    let translated = through_points(
        p(offset + 5.0, offset),
        p(offset - 5.0, offset),
        p(offset, offset + 5.0),
    )
    .unwrap();
    same(translated.center, p(offset, offset));
    near(translated.radius, 5.0);
}

#[test]
fn near_collinear_fit_refuses_when_determinant_loses_relative_precision() {
    assert_eq!(
        through_points(p(0.0, 0.0), p(1e150, 1e150), p(2e150, 2e150 + 1e135)),
        Err(ArcGeometryError::IllConditioned),
    );
}

#[test]
fn generated_circle_cases_preserve_travel_roles_across_scales() {
    for radius in [1e-6, 1.0, 1e100] {
        for index in 0..8 {
            for sweep in [0.25, 1.5 * PI] {
                for sign in [-1.0, 1.0] {
                    let angle = f64::from(index) * PI / 4.0;
                    let at = |a: f64| p(radius * a.cos(), radius * a.sin());
                    let start = at(angle);
                    let end = at(angle + sign * sweep);
                    let through = at(angle + sign * sweep * 0.5);
                    let arc = through_points(start, end, through).unwrap();
                    verify(&arc, start, end);
                    contains(&arc, through);
                    assert_eq!(arc.clockwise, sign < 0.0);
                    near(arc.sweep, sweep);
                    assert!(length(arc.center).unwrap() / radius < 1e-12);
                    near(arc.radius / radius, 1.0);
                }
            }
        }
    }
}

#[test]
fn tangent_behind_anchor_keeps_major_arc() {
    let arc = tangent_to(p(0.0, 0.0), p(1.0, 0.0), p(-3.0, 1.0)).unwrap();
    verify(&arc, p(0.0, 0.0), p(-3.0, 1.0));
    assert!(arc.sweep > PI);
    assert!(!arc.clockwise);
}

#[test]
fn selected_arc_extent_overflow_refuses_even_when_endpoints_are_finite() {
    assert_eq!(
        center_start_end(p(1.5e308, 0.0), p(1.5e308, -5e307), p(1.5e308, 1.0)),
        Err(ArcGeometryError::CoordinateOverflow),
    );
    let safe = center_start_end(p(1.5e308, 0.0), p(1.5e308, 5e307), p(1.5e308, -1.0)).unwrap();
    assert!(safe.radius.is_finite());
}
