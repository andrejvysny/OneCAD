//! Deterministic stroke fixtures: straight, noisy line, quarter arc,
//! semicircle, zig-zag, tiny stroke, closed loop and hysteresis.
use super::*;

fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}

fn recognize(points: &[Vec2]) -> (Recognition, StrokeRecognizer) {
    let mut recognizer = StrokeRecognizer::new(2.0);
    for point in points {
        recognizer.push(*point);
    }
    (recognizer.finish(), recognizer)
}

fn line(n: usize, noise: f64) -> Vec<Vec2> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            // Deterministic jitter: alternating, bounded by `noise`.
            let jitter = noise * if i % 2 == 0 { 1.0 } else { -1.0 } * ((i * 7 % 5) as f64 / 4.0);
            v(10.0 + 80.0 * t, 5.0 + 20.0 * t + jitter)
        })
        .collect()
}

fn arc(center: Vec2, radius: f64, from_deg: f64, to_deg: f64, n: usize) -> Vec<Vec2> {
    (0..=n)
        .map(|i| {
            let a = (from_deg + (to_deg - from_deg) * i as f64 / n as f64).to_radians();
            v(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect()
}

#[test]
fn a_straight_stroke_is_a_line_between_its_ends() {
    let points = line(30, 0.0);
    let (result, _) = recognize(&points);
    assert_eq!(
        result,
        Recognition::Shape(StrokeShape::Line {
            start: points[0],
            end: points[30]
        })
    );
}

#[test]
fn a_slightly_noisy_stroke_is_still_a_line() {
    let (result, _) = recognize(&line(40, 0.6));
    assert!(
        matches!(result, Recognition::Shape(StrokeShape::Line { .. })),
        "{result:?}"
    );
}

fn assert_arc_on_circle(result: &Recognition, center: Vec2, radius: f64) {
    let Recognition::Shape(StrokeShape::Arc {
        start,
        through,
        end,
    }) = result
    else {
        panic!("expected an arc, got {result:?}");
    };
    let fitted = arc_geometry::through_points(*start, *end, *through).expect("valid arc");
    assert!((fitted.radius - radius).abs() < 1e-6 * radius, "{fitted:?}");
    assert!(
        distance(fitted.center, center) < 1e-6 * radius,
        "{fitted:?}"
    );
}

#[test]
fn a_quarter_arc_is_a_three_point_arc_on_its_circle() {
    let center = v(50.0, 50.0);
    let (result, _) = recognize(&arc(center, 40.0, 0.0, 90.0, 24));
    assert_arc_on_circle(&result, center, 40.0);
}

#[test]
fn a_clockwise_semicircle_is_an_arc() {
    let center = v(-20.0, 10.0);
    let (result, _) = recognize(&arc(center, 25.0, 180.0, 0.0, 36));
    assert_arc_on_circle(&result, center, 25.0);
}

#[test]
fn a_zig_zag_is_unrecognized() {
    let points: Vec<Vec2> = (0..=12)
        .map(|i| v(i as f64 * 8.0, if i % 2 == 0 { 0.0 } else { 15.0 }))
        .collect();
    let (result, _) = recognize(&points);
    assert!(matches!(result, Recognition::Unrecognized(_)), "{result:?}");
}

#[test]
fn a_tiny_stroke_is_ignored() {
    let (result, _) = recognize(&[v(0.0, 0.0), v(0.5, 0.2), v(1.0, 0.3)]);
    assert_eq!(result, Recognition::TooShort);
    assert_eq!(StrokeRecognizer::new(2.0).finish(), Recognition::TooShort);
}

#[test]
fn a_closed_loop_is_neither_line_nor_arc() {
    let (result, _) = recognize(&arc(v(0.0, 0.0), 30.0, 0.0, 358.0, 60));
    assert!(matches!(result, Recognition::Unrecognized(_)), "{result:?}");
}

#[test]
fn non_finite_and_repeated_samples_are_skipped() {
    let mut points = line(10, 0.0);
    points.insert(3, v(f64::NAN, 0.0));
    points.insert(5, points[4]);
    let (result, recognizer) = recognize(&points);
    assert_eq!(recognizer.points().len(), 11);
    assert!(matches!(
        result,
        Recognition::Shape(StrokeShape::Line { .. })
    ));
}

#[test]
fn hysteresis_keeps_the_live_decision_inside_the_band() {
    // Straight start, then a bend of ~0.055 (inside the band): stays a line.
    let mut line_first = StrokeRecognizer::new(1.0);
    assert_eq!(
        line_first.push(v(0.0, 0.0)),
        None,
        "no decision before movement"
    );
    for point in [v(10.0, 0.0), v(20.0, 0.0), v(40.0, 2.0)] {
        assert_eq!(line_first.push(point), Some(StrokeKind::Line));
    }
    assert_eq!(line_first.push(v(60.0, 8.0)), Some(StrokeKind::Line));
    // A clear bend of 0.1 first, then 0.05: stays an arc.
    let mut arc_first = StrokeRecognizer::new(1.0);
    for point in [v(0.0, 0.0), v(10.0, 2.0)] {
        arc_first.push(point);
    }
    assert_eq!(arc_first.push(v(20.0, 0.0)), Some(StrokeKind::Arc));
    assert_eq!(arc_first.push(v(40.0, 0.0)), Some(StrokeKind::Arc));
    // A gentle 20-degree arc (bend ~0.044) drawn from a straight start is a line.
    let gentle = arc(v(0.0, -200.0), 200.0, 100.0, 80.0, 20);
    assert!(bend(&gentle) > LINE_BEND && bend(&gentle) < ARC_BEND);
    let (result, _) = recognize(&gentle);
    assert!(
        matches!(result, Recognition::Shape(StrokeShape::Line { .. })),
        "{result:?}"
    );
    assert_eq!(decide(0.02, Some(StrokeKind::Arc)), StrokeKind::Line);
    assert_eq!(decide(0.08, Some(StrokeKind::Line)), StrokeKind::Arc);
}

#[test]
fn explicit_override_refits_or_refuses() {
    let (_, quarter) = recognize(&arc(v(0.0, 0.0), 40.0, 0.0, 90.0, 24));
    assert!(matches!(
        quarter.shape_as(StrokeKind::Line),
        Ok(StrokeShape::Line { .. })
    ));
    let (_, gentle) = recognize(&arc(v(0.0, -200.0), 200.0, 100.0, 80.0, 20));
    assert!(matches!(
        gentle.shape_as(StrokeKind::Arc),
        Ok(StrokeShape::Arc { .. })
    ));
    let (_, straight) = recognize(&line(30, 0.0));
    assert!(
        straight.shape_as(StrokeKind::Arc).is_err(),
        "collinear points have no arc"
    );
    let (_, zig) = recognize(
        &(0..=12)
            .map(|i| v(i as f64 * 8.0, f64::from(i % 2) * 15.0))
            .collect::<Vec<_>>(),
    );
    assert!(
        zig.shape_as(StrokeKind::Line).is_err(),
        "a zig-zag doubles back"
    );
}
