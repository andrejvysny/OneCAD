use super::*;
use onecad_core::{
    ids::{EntityId, SketchId},
    sketch::{SketchEntity, WorldPlane},
};
use onecad_interaction::sketch::inference::{SnapIntent, SnapKind};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, Viewport};

fn view(scale: f64, zoom: f64) -> ViewSnapshot {
    ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        1,
        Camera {
            offset: DVec3::Z * zoom,
            projection: Projection::Orthographic,
            ..Camera::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: scale,
        },
    )
    .unwrap()
}
fn sketch() -> Sketch {
    Sketch::on_world_plane(SketchId::new(), "Snap", WorldPlane::XY)
}
fn add_point(sketch: &mut Sketch, x: f64, y: f64) -> EntityId {
    let id = EntityId::new();
    sketch
        .add_entity(SketchEntity::point(
            id,
            Vec2::new(x, y).unwrap(),
            false,
            false,
        ))
        .unwrap();
    id
}
fn request<'a>(sketch: &'a Sketch, view: &'a ViewSnapshot) -> SnapRequest<'a> {
    SnapRequest {
        sketch,
        plane: sketch.plane,
        view,
        rect: Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(800.0, 600.0)),
        pointer: Pos2::ZERO,
        anchor: None,
        previous: None,
        generation: 1,
        settings: SnapSettings {
            polar_step_degrees: None,
            ..Default::default()
        },
    }
}

#[test]
fn endpoint_aperture_uses_logical_pixels_at_each_zoom_and_retina_scale() {
    for scale in [1.0, 2.0] {
        for zoom in [50.0, 500.0] {
            let mut sketch = sketch();
            add_point(&mut sketch, 10.0, 20.0);
            let view = view(scale, zoom);
            let mut request = request(&sketch, &view);
            let position = project(&request, Vec2::new(10.0, 20.0).unwrap()).unwrap();
            request.pointer = position + egui::vec2(7.5, 0.0);
            assert_eq!(resolve_snap(&request).unwrap().kind, SnapKind::Endpoint);
            request.pointer = position + egui::vec2(8.5, 0.0);
            // Guide alignment can remain available after the endpoint aperture releases.
            assert_ne!(
                resolve_snap(&request).map(|snap| snap.kind),
                Some(SnapKind::Endpoint)
            );
        }
    }
}

#[test]
fn hysteresis_keeps_current_target_but_not_a_stale_source() {
    let mut sketch = sketch();
    let a = add_point(&mut sketch, 10.0, 20.0);
    let b = add_point(&mut sketch, 11.0, 20.0);
    let view = view(2.0, 100.0);
    let mut request = request(&sketch, &view);
    let pa = project(&request, Vec2::new(10.0, 20.0).unwrap()).unwrap();
    let pb = project(&request, Vec2::new(11.0, 20.0).unwrap()).unwrap();
    request.pointer = pa;
    let previous = resolve_snap(&request).unwrap();
    request.pointer = pa.lerp(pb, 0.6);
    request.previous = Some(&previous);
    let owner = |snap: AcceptedSnap| match snap.intent {
        SnapIntent::Coincident(target) => target.owner_id(),
        _ => panic!("point expected"),
    };
    assert_eq!(owner(resolve_snap(&request).unwrap()), a);
    request.generation += 1;
    assert_eq!(owner(resolve_snap(&request).unwrap()), b);
}

#[test]
fn circle_quadrants_win_over_generic_oncurve_and_pair_center_roles() {
    let mut sketch = sketch();
    let center = add_point(&mut sketch, 10.0, 20.0);
    let circle = EntityId::new();
    sketch
        .add_entity(SketchEntity::circle(circle, center, 5.0, false).unwrap())
        .unwrap();
    let view = view(1.0, 100.0);
    let mut request = request(&sketch, &view);
    request.pointer =
        project(&request, Vec2::new(15.0, 20.0).unwrap()).unwrap() + egui::vec2(0.0, 2.0);
    let snap = resolve_snap(&request).unwrap();
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert!(matches!(snap.intent,SnapIntent::Quadrant {curve,..} if curve==circle));
}

#[test]
fn hysteresis_retains_a_winner_outside_the_base_aperture_until_release_distance() {
    let mut sketch = sketch();
    add_point(&mut sketch, 10.0, 20.0);
    let view = view(2.0, 100.0);
    let mut request = request(&sketch, &view);
    let position = project(&request, Vec2::new(10.0, 20.0).unwrap()).unwrap();
    request.pointer = position;
    let previous = resolve_snap(&request).unwrap();
    request.previous = Some(&previous);
    request.pointer = position + egui::vec2(9.5, 0.0);
    assert_eq!(resolve_snap(&request).unwrap().kind, SnapKind::Endpoint);
    request.pointer = position + egui::vec2(11.5, 0.0);
    assert_ne!(
        resolve_snap(&request).map(|snap| snap.kind),
        Some(SnapKind::Endpoint)
    );
    request.pointer = position + egui::vec2(9.5, 0.0);
    request.generation += 1;
    assert_ne!(
        resolve_snap(&request).map(|snap| snap.kind),
        Some(SnapKind::Endpoint)
    );
}

#[test]
fn analytic_intersections_filter_segments_arcs_tangencies_and_coincident_curves() {
    use super::curves::{Curve, intersections};
    use onecad_scene::DVec2;
    let id = EntityId::new();
    let line = Curve::Line {
        id,
        a: DVec2::new(-10.0, 0.0),
        b: DVec2::new(10.0, 0.0),
    };
    let circle = Curve::Circle {
        id: EntityId::new(),
        center: DVec2::ZERO,
        radius: 5.0,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    assert_eq!(intersections(line, circle).len(), 2);
    let arc = Curve::Circle {
        id: EntityId::new(),
        center: DVec2::ZERO,
        radius: 5.0,
        start: 0.0,
        sweep: std::f64::consts::FRAC_PI_2,
    };
    assert_eq!(intersections(line, arc).len(), 1);
    assert!(intersections(circle, circle).is_empty());
    let tangent = Curve::Line {
        id,
        a: DVec2::new(-10.0, 5.0),
        b: DVec2::new(10.0, 5.0),
    };
    assert_eq!(intersections(tangent, circle).len(), 1);
    let crossing = Curve::Line {
        id,
        a: DVec2::new(0.0, -10.0),
        b: DVec2::new(0.0, 10.0),
    };
    assert_eq!(intersections(line, crossing).len(), 1);
    let other = Curve::Circle {
        id,
        center: DVec2::new(6.0, 0.0),
        radius: 5.0,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    assert_eq!(intersections(circle, other).len(), 2);
}

#[test]
fn equally_close_distinct_references_resolve_by_sketch_order_not_entity_id() {
    let mut sketch = sketch();
    let first = add_point(&mut sketch, 10.0, 20.0);
    add_point(&mut sketch, 10.0, 20.0);
    let view = view(1.0, 100.0);
    let mut request = request(&sketch, &view);
    request.pointer = project(&request, Vec2::new(10.0, 20.0).unwrap()).unwrap();
    let snap = resolve_snap(&request).expect("coincident targets must still snap");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert_eq!(
        snap.intent,
        SnapIntent::Coincident(SketchTarget::Point {
            entity: first,
            role: onecad_core::sketch::CurvePosition::Arbitrary
        })
    );
}

#[test]
fn aligned_points_prefer_the_nearest_guide_reference() {
    let mut sketch = sketch();
    add_point(&mut sketch, 0.0, 20.0);
    let near = add_point(&mut sketch, 30.0, 20.0);
    let view = view(1.0, 100.0);
    let mut request = request(&sketch, &view);
    request.pointer = project(&request, Vec2::new(45.0, 20.0).unwrap()).unwrap();
    let snap = resolve_snap(&request).expect("shared alignment must still guide");
    assert_eq!(snap.kind, SnapKind::HorizontalGuide);
    assert!(matches!(
        snap.intent,
        SnapIntent::Guide {
            reference: SketchTarget::Point { entity, .. },
            ..
        } if entity == near
    ));
}

#[test]
fn polar_waits_until_the_pointer_leaves_the_anchor() {
    let sketch = sketch();
    let view = view(1.0, 100.0);
    let mut request = request(&sketch, &view);
    request.settings.polar_step_degrees = Some(15.0);
    let anchor = Vec2::new(20.0, 30.0).unwrap();
    request.anchor = Some((anchor, None));
    let origin = project(&request, anchor).unwrap();
    request.pointer = origin + egui::vec2(12.0, 5.0);
    assert_ne!(
        resolve_snap(&request).map(|snap| snap.kind),
        Some(SnapKind::Polar)
    );
}

#[test]
fn grid_is_placement_only_and_unrepresented_polar_direction_creates_no_constraint() {
    let sketch = sketch();
    let view = view(2.0, 100.0);
    let mut request = request(&sketch, &view);
    request.settings.grid_step = Some(5.0);
    request.pointer = project(&request, Vec2::new(20.1, 30.1).unwrap()).unwrap();
    let snap = resolve_snap(&request).unwrap();
    assert_eq!(snap.kind, SnapKind::Grid);
    assert_eq!(snap.intent, SnapIntent::GeometryOnly);
    request.settings.grid_step = None;
    request.settings.polar_step_degrees = Some(15.0);
    request.anchor = Some((Vec2::new(20.0, 30.0).unwrap(), None));
    let angle = 29.0_f64.to_radians();
    request.pointer = project(
        &request,
        Vec2::new(20.0 + 20.0 * angle.cos(), 30.0 + 20.0 * angle.sin()).unwrap(),
    )
    .unwrap();
    let snap = resolve_snap(&request).unwrap();
    assert_eq!(snap.kind, SnapKind::Polar);
    assert_eq!(snap.intent, SnapIntent::GeometryOnly);
}

#[test]
fn ellipse_exposes_only_its_supported_center_reference() {
    let mut sketch = sketch();
    let center = add_point(&mut sketch, 10.0, 20.0);
    let ellipse = EntityId::new();
    sketch
        .add_entity(SketchEntity::ellipse(ellipse, center, 5.0, 3.0, 0.0, false).unwrap())
        .unwrap();
    let view = view(1.0, 100.0);
    let mut request = request(&sketch, &view);
    request.pointer = project(&request, Vec2::new(15.0, 20.0).unwrap()).unwrap();
    assert_ne!(
        resolve_snap(&request).map(|snap| snap.kind),
        Some(SnapKind::OnCurve)
    );
    assert!(curves::Curve::from_entity(&sketch, sketch.get_entity(ellipse).unwrap()).is_none());
}

#[test]
fn creation_adapter_preserves_arc_chain_anchor_roles_for_horizontal_and_vertical_guides() {
    use onecad_core::sketch::CurvePosition;
    use onecad_interaction::sketch::inference::PointAlignment;
    use onecad_interaction::sketch::{ChainCurve, SketchDraft};

    for role in [CurvePosition::Start, CurvePosition::End] {
        let mut sketch = sketch();
        let center = add_point(&mut sketch, 4.0, 7.0);
        let arc = EntityId::new();
        sketch
            .add_entity(SketchEntity::arc(arc, center, 10.0, 0.3, 1.7, false).unwrap())
            .unwrap();
        let reference = SketchTarget::Point { entity: arc, role };
        let anchor = reference.point_coordinates(&sketch).unwrap();
        let mut draft = SketchDraft {
            first: anchor,
            anchor,
            first_id: None,
            first_target: None,
            anchor_id: None,
            cursor: anchor,
            segments: 1,
            anchor_snap: None,
            secondary: None,
            preceding: Some(ChainCurve { entity: arc, role }),
            tangent_arc: false,
        };
        let view = view(2.0, 100.0);
        for (dx, dy, kind, alignment) in [
            (
                25.0,
                0.0,
                SnapKind::HorizontalGuide,
                PointAlignment::Horizontal,
            ),
            (0.0, 25.0, SnapKind::VerticalGuide, PointAlignment::Vertical),
        ] {
            let mut request = request(&sketch, &view);
            request.pointer =
                project(&request, Vec2::new(anchor.x + dx, anchor.y + dy).unwrap()).unwrap();
            request.anchor = Some((anchor, None));
            assert_eq!(
                resolve_snap(&request).map(|snap| snap.intent),
                Some(SnapIntent::Guide {
                    reference,
                    alignment
                }),
                "a referenced guide outranks the anonymous anchor guide at the same place"
            );
            request.anchor = Some(crate::sketch_creation::snap_anchor(&draft));
            let snap =
                resolve_snap(&request).expect("adapter must retain persistent endpoint identity");
            assert_eq!(snap.kind, kind);
            assert_eq!(
                snap.intent,
                SnapIntent::Guide {
                    reference,
                    alignment
                }
            );
        }
        draft.anchor_id = Some(center);
        assert_eq!(
            crate::sketch_creation::snap_anchor(&draft).1,
            Some(SketchTarget::Point {
                entity: center,
                role: CurvePosition::Arbitrary
            })
        );
    }
}

#[test]
fn grid_snap_coarsens_when_minor_cells_fall_below_visibility() {
    let sketch = sketch();
    let view = view(1.0, 100.0);
    let mut request = request(&sketch, &view);
    let origin = project(&request, Vec2::new(0.0, 0.0).unwrap()).unwrap();
    let cell = project(&request, Vec2::new(1.0, 0.0).unwrap()).unwrap();
    let px_per_unit = origin.distance(cell);
    // A minor step smaller than the visible threshold must snap to the major step.
    request.settings.grid_step = Some(f64::from(4.0 / px_per_unit));
    let major = request.settings.grid_step.unwrap() * 10.0;
    request.pointer = project(&request, Vec2::new(major * 3.0 + 0.01, major).unwrap()).unwrap();
    let snap = resolve_snap(&request).unwrap();
    assert_eq!(snap.kind, SnapKind::Grid);
    assert!((snap.point.x - major * 3.0).abs() < 1e-9);
    assert!((snap.point.y - major).abs() < 1e-9);
}

#[test]
fn creation_anchor_welded_to_existing_point_references_it_for_guides() {
    use onecad_core::sketch::CurvePosition;
    use onecad_interaction::sketch::SketchDraft;

    let mut sketch = sketch();
    let corner = add_point(&mut sketch, 10.0, 20.0);
    let view = view(1.0, 100.0);
    let target = SketchTarget::Point {
        entity: corner,
        role: CurvePosition::Arbitrary,
    };
    let anchor = Vec2::new(10.0, 20.0).unwrap();
    let draft = SketchDraft {
        first: anchor,
        anchor,
        first_id: None,
        first_target: None,
        anchor_id: None,
        cursor: anchor,
        segments: 0,
        anchor_snap: Some(AcceptedSnap {
            source: snap_source(&sketch, &view, 1),
            kind: SnapKind::Endpoint,
            point: anchor,
            intent: SnapIntent::Coincident(target),
        }),
        secondary: None,
        preceding: None,
        tangent_arc: false,
    };
    assert_eq!(crate::sketch_creation::snap_anchor(&draft).1, Some(target));
}
