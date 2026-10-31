//! Crossings, the priority ladder, refinement over hysteresis, extensions and suppression.
use super::*;
use crate::sketch_snapping::candidates::kind_priority;
use onecad_core::sketch::CurvePosition;
use onecad_interaction::sketch::inference::PointAlignment;

/// One grid cell is ~15 px here; at zoom 50 the sketch fits ±39 units in x.
const GRID_ZOOM: f64 = 25.0;
const WIDE_ZOOM: f64 = 50.0;

fn ppu(request: &SnapRequest<'_>) -> f64 {
    let (a, b) = (Vec2::new(0.0, 0.0).unwrap(), Vec2::new(1.0, 0.0).unwrap());
    f64::from(
        project(request, a)
            .unwrap()
            .distance(project(request, b).unwrap()),
    )
}
/// Pointer at a sketch point shifted by `dx` pixels along sketch +X.
fn pointer(request: &SnapRequest<'_>, x: f64, y: f64, dx: f64) -> Pos2 {
    project(request, Vec2::new(x + dx / ppu(request), y).unwrap()).unwrap()
}
fn target(entity: EntityId, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point { entity, role }
}
fn arbitrary(entity: EntityId) -> SketchTarget {
    target(entity, CurvePosition::Arbitrary)
}
fn add_line(sketch: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> (EntityId, EntityId, EntityId) {
    let (start, end, line) = (
        add_point(sketch, a.0, a.1),
        add_point(sketch, b.0, b.1),
        EntityId::new(),
    );
    sketch
        .add_entity(SketchEntity::line(line, start, end, false))
        .unwrap();
    (line, start, end)
}
fn winner(request: &SnapRequest<'_>) -> ResolvedSnap {
    resolve(request).winner.expect("a snap")
}
fn assert_at(snap: &AcceptedSnap, x: f64, y: f64) {
    assert!(
        (snap.point.x - x).abs() < 1e-9 && (snap.point.y - y).abs() < 1e-9,
        "{:?} != ({x}, {y})",
        snap.point
    );
}
fn guide_kinds(resolved: &ResolvedSnap) -> Vec<(GuideKind, bool)> {
    resolved
        .guides
        .iter()
        .flatten()
        .map(|guide| (guide.kind, guide.persisted))
        .collect()
}

fn guide_grid_fixture() -> (Sketch, EntityId) {
    let mut sketch = sketch();
    let p = add_point(&mut sketch, 0.37, 5.43);
    (sketch, p)
}

#[test]
fn guide_crossing_a_grid_line_snaps_and_persists_only_the_guide() {
    let (sketch, p) = guide_grid_fixture();
    let view = view(1.0, GRID_ZOOM);
    let mut request = request(&sketch, &view);
    request.settings.grid_step = Some(1.0);
    request.pointer = pointer(&request, 3.0, 5.43, -2.0);
    let resolved = winner(&request);
    assert_eq!(resolved.snap.kind, SnapKind::GuideIntersection);
    assert_at(&resolved.snap, 3.0, 5.43);
    assert_eq!(
        resolved.snap.intent,
        SnapIntent::Guide {
            reference: arbitrary(p),
            alignment: PointAlignment::Horizontal
        }
    );
    assert_eq!(
        guide_kinds(&resolved),
        [(GuideKind::Horizontal, true), (GuideKind::Grid, false)]
    );
}

#[test]
fn a_stronger_crossing_on_the_held_guide_replaces_it_despite_hysteresis() {
    let (sketch, p) = guide_grid_fixture();
    let view = view(1.0, GRID_ZOOM);
    let mut request = request(&sketch, &view);
    request.settings.grid_step = Some(1.0);
    request.pointer = pointer(&request, 2.45, 5.43, 0.0);
    let held = winner(&request).snap;
    assert_eq!(held.kind, SnapKind::HorizontalGuide);
    assert_eq!(
        held.intent,
        SnapIntent::Guide {
            reference: arbitrary(p),
            alignment: PointAlignment::Horizontal
        }
    );
    request.previous = Some(&held);
    request.pointer = pointer(&request, 3.0, 5.43, -4.0);
    let moved = winner(&request).snap;
    assert_eq!(moved.kind, SnapKind::GuideIntersection);
    assert_at(&moved, 3.0, 5.43);
}

#[test]
fn horizontal_and_vertical_guides_of_two_points_cross_into_guides() {
    let mut sketch = sketch();
    let p1 = add_point(&mut sketch, -8.0, 3.37);
    let p2 = add_point(&mut sketch, 4.21, -9.0);
    let view = view(1.0, WIDE_ZOOM);
    let mut request = request(&sketch, &view);
    request.pointer = pointer(&request, 4.21, 3.37, -3.0);
    let held = AcceptedSnap {
        source: snap_source(&sketch, &view, 1),
        kind: SnapKind::HorizontalGuide,
        point: Vec2::new(4.21 - 3.0 / ppu(&request), 3.37).unwrap(),
        intent: SnapIntent::Guide {
            reference: arbitrary(p1),
            alignment: PointAlignment::Horizontal,
        },
    };
    request.previous = Some(&held);
    let snap = winner(&request).snap;
    assert_eq!(snap.kind, SnapKind::Guides);
    assert_at(&snap, 4.21, 3.37);
    assert_eq!(
        snap.intent,
        SnapIntent::Guides {
            horizontal: arbitrary(p1),
            vertical: arbitrary(p2)
        }
    );
}

#[test]
fn guide_crossing_a_curve_persists_alignment_and_membership() {
    let mut sketch = sketch();
    let (line, ..) = add_line(&mut sketch, (0.0, -10.0), (0.0, 10.0));
    let p = add_point(&mut sketch, -5.0, 2.43);
    let view = view(1.0, WIDE_ZOOM);
    let mut request = request(&sketch, &view);
    request.pointer = pointer(&request, 0.0, 2.43, 2.0);
    let snap = winner(&request).snap;
    assert_eq!(snap.kind, SnapKind::GuideIntersection);
    assert_at(&snap, 0.0, 2.43);
    assert_eq!(
        snap.intent,
        SnapIntent::GuideOnCurve {
            reference: arbitrary(p),
            alignment: PointAlignment::Horizontal,
            curve: line
        }
    );
}

#[test]
fn polar_ray_crossing_a_guide_keeps_the_guide_intent() {
    let mut sketch = sketch();
    let p = add_point(&mut sketch, -20.0, 10.0);
    let view = view(1.0, WIDE_ZOOM);
    let mut request = request(&sketch, &view);
    request.settings.polar_step_degrees = Some(15.0);
    request.anchor = Some((Vec2::new(0.0, 0.0).unwrap(), None));
    request.pointer = pointer(&request, 10.0, 10.0, 2.0);
    let resolved = winner(&request);
    assert_eq!(resolved.snap.kind, SnapKind::GuideIntersection);
    assert_eq!(
        resolved.snap.intent,
        SnapIntent::Guide {
            reference: arbitrary(p),
            alignment: PointAlignment::Horizontal
        }
    );
    let kinds = guide_kinds(&resolved);
    assert!(kinds.contains(&(GuideKind::Polar { degrees: 45.0 }, false)));
    assert!(kinds.contains(&(GuideKind::Horizontal, true)));
}

#[test]
fn the_axis_through_the_anchor_is_a_guide_never_a_polar_ray() {
    let mut sketch = sketch();
    add_point(&mut sketch, -20.0, 10.0);
    let view = view(1.0, WIDE_ZOOM);
    let mut request = request(&sketch, &view);
    request.settings.polar_step_degrees = Some(15.0);
    request.anchor = Some((Vec2::new(0.0, 0.0).unwrap(), None));
    request.pointer = project(&request, Vec2::new(20.0, 2.0 / ppu(&request)).unwrap()).unwrap();
    let resolved = winner(&request);
    assert_eq!(resolved.snap.kind, SnapKind::HorizontalGuide);
    assert!(
        !guide_kinds(&resolved)
            .iter()
            .any(|(kind, _)| matches!(kind, GuideKind::Polar { .. }))
    );
}

#[test]
fn priority_ladder_orders_exact_points_crossings_curves_guides_and_grid() {
    let ladder = [
        SnapKind::Endpoint,
        SnapKind::Origin,
        SnapKind::Center,
        SnapKind::Intersection,
        SnapKind::Midpoint,
        SnapKind::Quadrant,
    ]
    .map(kind_priority);
    assert!(ladder.windows(2).all(|pair| pair[0] < pair[1]));
    let (guides, crossing) = (
        kind_priority(SnapKind::Guides),
        kind_priority(SnapKind::GuideIntersection),
    );
    assert_eq!(guides, 20);
    assert_eq!(guides, crossing);
    assert!(ladder[5] < 10 && 10 < guides);
    let rest = [
        kind_priority(SnapKind::OnCurve),
        45,
        kind_priority(SnapKind::HorizontalGuide),
    ];
    assert!(guides < rest[0] && rest[0] < rest[1] && rest[1] < rest[2]);
    assert_eq!(rest[2], kind_priority(SnapKind::VerticalGuide));
    assert!(rest[2] < kind_priority(SnapKind::Extension));
    assert!(kind_priority(SnapKind::Extension) < kind_priority(SnapKind::Polar));
    assert!(kind_priority(SnapKind::Polar) < kind_priority(SnapKind::Grid));
}

#[test]
fn a_suppressed_snap_stays_hidden_while_in_reach_and_is_released_beyond_it() {
    let mut sketch = sketch();
    add_point(&mut sketch, 5.43, 7.21);
    let view = view(1.0, WIDE_ZOOM);
    let mut request = request(&sketch, &view);
    request.pointer = pointer(&request, 5.43, 7.21, 0.0);
    let first = winner(&request).snap;
    assert_eq!(first.kind, SnapKind::Endpoint);
    let discarded = [first];
    request.suppressed = &discarded;
    let resolution = resolve(&request);
    assert_ne!(resolution.winner.map(|w| w.snap), Some(first));
    assert!(resolution.winner.is_some());
    assert_eq!(resolution.suppressed_present, [first]);
    request.pointer = pointer(&request, 5.43, 7.21, 12.0);
    assert!(resolve(&request).suppressed_present.is_empty());
}

mod extensions {
    use super::*;
    use crate::sketch_snapping::guide_types::GuideShape;

    fn diagonal() -> (Sketch, EntityId, EntityId) {
        let mut sketch = sketch();
        let (line, _, end) = add_line(&mut sketch, (0.0, 0.0), (10.0, 10.0));
        (sketch, line, end)
    }

    #[test]
    fn a_line_extension_needs_an_acquired_endpoint_or_the_anchor_reference() {
        let (sketch, line, b) = diagonal();
        let view = view(1.0, WIDE_ZOOM);
        let mut request = request(&sketch, &view);
        request.pointer = pointer(&request, 15.0, 15.0, 0.0);
        assert_ne!(
            resolve(&request).winner.map(|w| w.snap.kind),
            Some(SnapKind::Extension)
        );
        let acquired = [arbitrary(b)];
        request.extensions = &acquired;
        let resolved = winner(&request);
        assert_eq!(resolved.snap.kind, SnapKind::Extension);
        assert_eq!(resolved.snap.intent, SnapIntent::OnCurve(line));
        let guide = resolved.guides.iter().flatten().next().unwrap();
        assert_eq!(guide.kind, GuideKind::Extension);
        assert!(guide.persisted);
        assert_eq!(
            guide.shape,
            GuideShape::From(Vec2::new(10.0, 10.0).unwrap())
        );
        request.extensions = &[];
        request.anchor = Some((Vec2::new(10.0, 10.0).unwrap(), Some(arbitrary(b))));
        assert_eq!(winner(&request).snap.kind, SnapKind::Extension);
    }

    #[test]
    fn an_extension_crossing_a_guide_persists_alignment_and_membership() {
        let (mut sketch, line, b) = diagonal();
        let p = add_point(&mut sketch, -30.0, 20.0);
        let view = view(1.0, WIDE_ZOOM);
        let mut request = request(&sketch, &view);
        let acquired = [arbitrary(b)];
        request.extensions = &acquired;
        request.pointer = pointer(&request, 20.0, 20.0, 2.0);
        let snap = winner(&request).snap;
        assert_eq!(snap.kind, SnapKind::GuideIntersection);
        assert_at(&snap, 20.0, 20.0);
        assert_eq!(
            snap.intent,
            SnapIntent::GuideOnCurve {
                reference: arbitrary(p),
                alignment: PointAlignment::Horizontal,
                curve: line
            }
        );
    }

    #[test]
    fn an_arc_extension_continues_the_circle_from_the_acquired_end() {
        let mut sketch = sketch();
        let center = add_point(&mut sketch, 0.0, 0.0);
        let arc = EntityId::new();
        let entity = SketchEntity::arc(arc, center, 10.0, 0.0, std::f64::consts::FRAC_PI_2, false);
        sketch.add_entity(entity.unwrap()).unwrap();
        let view = view(1.0, WIDE_ZOOM);
        let mut request = request(&sketch, &view);
        let acquired = [target(arc, CurvePosition::End)];
        request.extensions = &acquired;
        let angle = 135.0_f64.to_radians();
        let at = Vec2::new(10.0 * angle.cos(), 10.0 * angle.sin()).unwrap();
        request.pointer = project(&request, at).unwrap();
        let resolved = winner(&request);
        assert_eq!(resolved.snap.kind, SnapKind::Extension);
        assert_eq!(resolved.snap.intent, SnapIntent::OnCurve(arc));
        let guide = resolved.guides.iter().flatten().next().unwrap();
        assert!(matches!(guide.shape, GuideShape::Arc { forward: true, .. }));
    }
}
