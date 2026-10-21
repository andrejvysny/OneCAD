use super::*;
use onecad_app::dto::FinishSketchDto;
use onecad_core::{
    document::refs::SketchRegionRef,
    ids::EntityId,
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use std::collections::BTreeSet;

pub fn saved(baseline: &Document, saved: &Document, width: f64) {
    assert_eq!(
        extrude(saved).0.op,
        extrude(baseline).0.op,
        "width edit does not alter extrusion"
    );
    saved_at_depth(baseline, saved, width, 4.0);
}

pub fn saved_at_depth(baseline: &Document, saved: &Document, width: f64, depth: f64) {
    assert_eq!(saved.id, baseline.id);
    assert_eq!(
        (saved.timeline.cursor(), saved.timeline.records().len()),
        (2, 2)
    );
    assert_eq!(saved.sketches.len(), 1);
    assert_eq!(saved.bodies.len(), 1);
    assert_eq!(saved.bodies.bodies()[0].id, baseline.bodies.bodies()[0].id);
    assert_eq!(
        saved
            .timeline
            .records()
            .iter()
            .map(|r| r.record_id)
            .collect::<Vec<_>>(),
        baseline
            .timeline
            .records()
            .iter()
            .map(|r| r.record_id)
            .collect::<Vec<_>>()
    );
    let (record, params) = extrude(saved);
    let (baseline_record, baseline_params) = extrude(baseline);
    let mut normalized = params.clone();
    normalized.distance = baseline_params.distance.clone();
    assert_eq!(
        &normalized, baseline_params,
        "only extrusion distance may change"
    );
    assert_eq!(record.outputs, baseline_record.outputs);
    assert_eq!(record.inputs, record.op.derive_inputs());
    assert_eq!(record.step_index, 1);
    assert!(!record.suppressed);
    near(params.distance.value, depth);
    let profile = params.profile.as_ref().unwrap();
    assert_eq!(params.profile, baseline_params.profile);
    let expected = &baseline.sketches[&profile.sketch];
    let actual = &saved.sketches[&profile.sketch];
    assert_eq!(actual.id, expected.id);
    assert_eq!(
        actual
            .entities()
            .iter()
            .map(SketchEntity::id)
            .collect::<BTreeSet<_>>(),
        expected
            .entities()
            .iter()
            .map(SketchEntity::id)
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        actual
            .constraints()
            .iter()
            .map(Constraint::id)
            .collect::<BTreeSet<_>>(),
        expected
            .constraints()
            .iter()
            .map(Constraint::id)
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(actual.constraints().len(), expected.constraints().len());
    record_dimensions(saved, actual);
    sketch(actual, width);
}

fn point(sketch: &Sketch, id: EntityId) -> Vec2 {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => *at,
        other => panic!("point expected: {other:?}"),
    }
}

pub fn sketch(sketch: &Sketch, width: f64) {
    assert_eq!(sketch.constraints().len(), 17);
    let walls: Vec<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Line {
                id,
                start,
                end,
                construction: false,
                ..
            } => Some((*id, *start, *end)),
            _ => None,
        })
        .collect();
    assert_eq!(walls.len(), 4);
    assert_eq!(
        sketch
            .entities()
            .iter()
            .filter(|entity| matches!(entity, SketchEntity::Line { .. }))
            .count(),
        5
    );
    let corners = [
        Vec2::new_unchecked(0.0, 0.0),
        Vec2::new_unchecked(width, 0.0),
        Vec2::new_unchecked(width, 50.0),
        Vec2::new_unchecked(0.0, 50.0),
    ];
    let mut seen = BTreeSet::new();
    for (_, start, end) in &walls {
        let a = corner(point(sketch, *start), &corners);
        let b = corner(point(sketch, *end), &corners);
        assert!(
            (a + 1) % 4 == b || (b + 1) % 4 == a,
            "wall joins adjacent corners"
        );
        seen.insert((a.min(b), a.max(b)));
    }
    assert_eq!(seen.len(), 4);
    circles_and_construction(sketch, width);
}

fn corner(point: Vec2, corners: &[Vec2; 4]) -> usize {
    corners
        .iter()
        .position(|expected| {
            (point.x - expected.x).abs() < 1e-6 && (point.y - expected.y).abs() < 1e-6
        })
        .unwrap_or_else(|| panic!("rectangle corner {point:?} outside expected {corners:?}"))
}

fn circles_and_construction(sketch: &Sketch, width: f64) {
    let circles: Vec<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Circle {
                id,
                center,
                radius,
                construction: false,
                ..
            } => {
                near(*radius, 3.0);
                assert!(sketch.constraints().iter().any(|constraint| matches!(constraint,
                    Constraint::Diameter { entity, value, .. } if entity == id && (value.value - 6.0).abs() < 1e-6)));
                Some((*id, *center))
            }
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 2);
    assert_eq!(
        sketch
            .entities()
            .iter()
            .filter(|entity| matches!(entity, SketchEntity::Circle { .. }))
            .count(),
        2
    );
    let centers: Vec<_> = circles
        .iter()
        .map(|(_, center)| point(sketch, *center))
        .collect();
    for x in [15.0, width - 15.0] {
        assert!(centers
            .iter()
            .any(|p| (p.x - x).abs() < 1e-6 && (p.y - 25.0).abs() < 1e-6));
    }
    let construction: Vec<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Line {
                id,
                start,
                end,
                construction: true,
                ..
            } => Some((*id, *start, *end)),
            _ => None,
        })
        .collect();
    assert_eq!(construction.len(), 1);
    let (line, start, end) = construction[0];
    for id in [start, end] {
        let at = point(sketch, id);
        assert!(centers
            .iter()
            .any(|center| (at.x - center.x).abs() < 1e-6 && (at.y - center.y).abs() < 1e-6));
        assert!(
            circles
                .iter()
                .any(|(_, center)| welded(sketch, id, *center)),
            "persistent centerline weld"
        );
    }
    assert_ne!(start, end);
    near(
        (point(sketch, end).x - point(sketch, start).x).abs(),
        width - 30.0,
    );
    near(point(sketch, start).y, 25.0);
    near(point(sketch, end).y, 25.0);
    assert!(sketch
        .constraints()
        .iter()
        .any(|constraint| matches!(constraint,
        Constraint::Horizontal { line: constrained, .. } if *constrained == line)));
    assert!(sketch.entities().iter().any(|entity| matches!(entity,
        SketchEntity::Point { at, construction: true, .. } if (at.x - 15.0).abs() < 1e-6 && (at.y - 25.0).abs() < 1e-6)));
}

pub fn profile(regions: &FinishSketchDto, sketch: &Sketch, expected: &SketchRegionRef) {
    assert!(regions.diagnostics.is_empty(), "{:?}", regions.diagnostics);
    assert_eq!(
        Some(regions.region_identity_version),
        expected.region_identity_version
    );
    assert_eq!(
        regions.regions.len(),
        3,
        "plate material and two disk regions"
    );
    assert_eq!(
        regions
            .regions
            .iter()
            .map(|region| &region.region_id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let region = regions
        .regions
        .iter()
        .find(|region| region.region_id == expected.region.to_string())
        .expect("extrusion references the plate material region");
    assert_eq!(region.region_id, expected.region.to_string());
    let walls: BTreeSet<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Line {
                id,
                construction: false,
                ..
            } => Some(id.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(region.outer_loop.len(), 4);
    assert_eq!(
        region.outer_loop.iter().cloned().collect::<BTreeSet<_>>(),
        walls
    );
    let circles: BTreeSet<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Circle {
                id,
                construction: false,
                ..
            } => Some(id.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(region.holes.len(), 2);
    assert!(region.holes.iter().all(|hole| hole.len() == 1));
    assert_eq!(
        region
            .holes
            .iter()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>(),
        circles
    );
    assert_eq!(
        region.preview_triangles.as_ref().unwrap().holes_subtracted,
        2
    );
    let disks: Vec<_> = regions
        .regions
        .iter()
        .filter(|disk| disk.region_id != region.region_id)
        .collect();
    assert!(disks
        .iter()
        .all(|disk| disk.outer_loop.len() == 1 && disk.holes.is_empty()));
    assert_eq!(
        disks
            .iter()
            .flat_map(|disk| disk.outer_loop.iter().cloned())
            .collect::<BTreeSet<_>>(),
        circles
    );
    assert!(disks
        .iter()
        .all(|disk| disk.preview_triangles.as_ref().unwrap().holes_subtracted == 0));
}

fn owned_point(sketch: &Sketch, entity: EntityId, role: CurvePosition) -> EntityId {
    match (sketch.get_entity(entity).unwrap(), role) {
        (SketchEntity::Point { .. }, CurvePosition::Arbitrary) => entity,
        (SketchEntity::Circle { center, .. }, CurvePosition::Center) => *center,
        (SketchEntity::Line { start, .. }, CurvePosition::Start) => *start,
        (SketchEntity::Line { end, .. }, CurvePosition::End) => *end,
        other => panic!("unsupported weld point: {other:?}"),
    }
}

fn welded(sketch: &Sketch, start: EntityId, end: EntityId) -> bool {
    let mut connected = BTreeSet::from([start]);
    loop {
        let before = connected.len();
        for constraint in sketch.constraints() {
            if let Constraint::Coincident {
                point1,
                point1_position,
                point2,
                point2_position,
                ..
            } = constraint
            {
                let a = owned_point(sketch, *point1, *point1_position);
                let b = owned_point(sketch, *point2, *point2_position);
                if connected.contains(&a) || connected.contains(&b) {
                    connected.extend([a, b]);
                }
            }
        }
        if connected.contains(&end) {
            return true;
        }
        if connected.len() == before {
            return false;
        }
    }
}

fn record_dimensions(document: &Document, sketch: &Sketch) {
    let record = document
        .timeline
        .records()
        .iter()
        .find_map(|record| match &record.op {
            Operation::Known(KnownOperation::Sketch(params)) if params.sketch == sketch.id => {
                Some(params)
            }
            _ => None,
        })
        .expect("producing Sketch record");
    for constraint in sketch.constraints() {
        let Some(value) = constraint.value() else {
            continue;
        };
        let id = constraint.id().to_string();
        let raw = record
            .constraints
            .iter()
            .find(|raw| raw["id"].as_str() == Some(id.as_str()))
            .expect("timeline contains the same dimensional constraint");
        let recorded = raw["value"]
            .as_f64()
            .or_else(|| raw["value"]["value"].as_f64())
            .expect("numeric timeline dimension");
        assert_eq!(
            value.value, recorded,
            "saved sketch dimension {id} disagrees with its producing timeline record"
        );
    }
}
