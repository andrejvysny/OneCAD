use super::*;

fn at(sketch: &Sketch, id: EntityId) -> Vec2 {
    let SketchEntity::Point { at, .. } = sketch.get_entity(id).unwrap() else {
        panic!("point expected")
    };
    *at
}

pub(super) fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

pub(super) fn assert_plate(host: &Harness, plate: &Plate, width: f64) {
    let sketch = host.sketch();
    let r = plate.rectangle.entities;
    for (id, expected) in [
        (r[0], [0.0, 0.0]),
        (r[1], [width, 0.0]),
        (r[2], [width, 50.0]),
        (r[3], [0.0, 50.0]),
        (plate.holes[0].entities[0], [15.0, 25.0]),
        (plate.holes[1].entities[0], [width - 15.0, 25.0]),
        (plate.centerline.entities[0], [0.0, 25.0]),
        (plate.centerline.entities[1], [width, 25.0]),
    ] {
        let point = at(sketch, id);
        near(point.x, expected[0]);
        near(point.y, expected[1]);
    }
    for hole in &plate.holes {
        let SketchEntity::Circle { center, radius, .. } =
            sketch.get_entity(hole.entities[1]).unwrap()
        else {
            panic!("circle expected")
        };
        assert_eq!(*center, hole.entities[0]);
        near(*radius, 3.0);
    }
    for id in &plate.centerline.entities[..3] {
        assert!(sketch.get_entity(*id).unwrap().is_construction());
    }
    assert!(
        matches!(sketch.get_constraint(plate.width), Some(Constraint::HorizontalDistance { point1, point2, value, .. }) if *point1 == r[0] && *point2 == r[1] && (value.value - width).abs() < 1e-9)
    );
    for (id, hole) in plate.diameters.iter().zip(&plate.holes) {
        assert!(
            matches!(sketch.get_constraint(*id), Some(Constraint::Diameter { entity, value, .. }) if *entity == hole.entities[1] && value.value == 6.0)
        );
    }
    assert_intent(sketch, plate);
    assert_eq!(
        crate::sketch_feedback::summary(&host.app.sketch.session.as_ref().unwrap().status)
            .assessment,
        crate::sketch_feedback::Assessment::Healthy
    );
}

fn assert_intent(sketch: &Sketch, plate: &Plate) {
    let r = plate.rectangle.entities;
    let a = plate.holes[0].entities[1];
    let b = plate.holes[1].entities[1];
    for index in 0..4 {
        let line = r[index + 4];
        assert!(
            sketch
                .constraints()
                .iter()
                .any(|constraint| match constraint {
                    Constraint::Horizontal {
                        line: constrained, ..
                    } if index % 2 == 0 => *constrained == line,
                    Constraint::Vertical {
                        line: constrained, ..
                    } if index % 2 == 1 => *constrained == line,
                    _ => false,
                })
        );
    }
    for (from, to, expected) in [(r[0], a, 15.0), (r[1], b, -15.0)] {
        assert!(sketch.constraints().iter().any(|constraint| matches!(constraint,
            Constraint::HorizontalDistance { point1, point2, point1_position: CurvePosition::Arbitrary,
                point2_position: CurvePosition::Center, value, .. }
            if *point1 == from && *point2 == to && value.value == expected)));
    }
    for (from, to, expected, role) in [
        (r[0], r[3], 50.0, CurvePosition::Arbitrary),
        (r[0], a, 25.0, CurvePosition::Center),
    ] {
        assert!(
            sketch
                .constraints()
                .iter()
                .any(|constraint| matches!(constraint,
            Constraint::VerticalDistance { point1, point2, point1_position: CurvePosition::Arbitrary, point2_position, value, .. }
            if *point1 == from && *point2 == to && *point2_position == role && value.value == expected))
        );
    }
    assert!(
        sketch
            .constraints()
            .iter()
            .any(|constraint| matches!(constraint,
        Constraint::HorizontalPoints { point1, point2, point1_position: CurvePosition::Center, point2_position: CurvePosition::Center, .. } if *point1 == a && *point2 == b))
    );
    for (endpoint, side) in [
        (plate.centerline.entities[0], r[7]),
        (plate.centerline.entities[1], r[5]),
    ] {
        assert!(
            sketch
                .constraints()
                .iter()
                .any(|constraint| matches!(constraint,
            Constraint::Midpoint { point, line, .. } if *point == endpoint && *line == side))
        );
    }
    assert!(
        sketch
            .constraints()
            .iter()
            .any(|constraint| matches!(constraint,
        Constraint::Fixed { point, at, .. } if *point == r[0] && at.x == 0.0 && at.y == 0.0))
    );
}

pub(super) fn assert_finished_profile(host: &Harness, plate: &Plate) -> String {
    let SketchStatus::Finished { result, .. } = &host.app.sketch.session.as_ref().unwrap().status
    else {
        panic!("Finish result expected")
    };
    let region = result
        .regions
        .iter()
        .find(|region| region.holes.len() == 2)
        .expect("material region with two holes");
    assert_eq!(region.outer_loop.len(), 4);
    for id in &plate.rectangle.entities[4..8] {
        assert!(region.outer_loop.contains(&id.to_string()));
    }
    for hole in &plate.holes {
        assert!(
            region
                .holes
                .iter()
                .any(|boundary| boundary.contains(&hole.entities[1].to_string()))
        );
    }
    assert!(
        !region
            .outer_loop
            .contains(&plate.centerline.entities[2].to_string())
    );
    assert_eq!(
        region.preview_triangles.as_ref().unwrap().holes_subtracted,
        2
    );
    region.region_id.clone()
}

pub(super) fn assert_profiles(host: &Harness, plate: &Plate) {
    let region_id = assert_finished_profile(host, plate);
    assert!(
        host.app
            .display
            .as_ref()
            .unwrap()
            .profiles
            .iter()
            .any(|profile| profile.reference.region.to_string() == region_id)
    );
}

pub(super) fn assert_mesh_bounds(app: &App, width: f64) {
    let bodies = app.display.as_ref().unwrap().scene.bodies();
    assert_eq!(bodies.len(), 1);
    let mesh = bodies[0].mesh().unwrap();
    let mut lower = DVec3::splat(f64::INFINITY);
    let mut upper = DVec3::splat(f64::NEG_INFINITY);
    for index in 0..mesh.header().vertex_count as usize {
        let point = mesh.position(index).unwrap().map(f64::from);
        let point = DVec3::from_array(point);
        lower = lower.min(point);
        upper = upper.max(point);
    }
    for (actual, expected) in lower.to_array().into_iter().zip([-50.0, 0.0, 0.0]) {
        near(actual, expected);
    }
    for (actual, expected) in upper.to_array().into_iter().zip([0.0, width, 4.0]) {
        near(actual, expected);
    }
    // OCCT header bounds include shape tolerances; vertex extrema remain separate evidence.
    assert!(bodies[0].bounds().min.cmple(lower).all());
    assert!(bodies[0].bounds().max.cmpge(upper).all());
}

pub(super) async fn assert_brep(host: &Harness, width: f64) {
    let display = host.app.display.as_ref().unwrap();
    let body_id = onecad_app::worker::wire::body_id_wire(display.scene.bodies()[0].body_id());
    let reading = read_brep(host, display.context.clone()).await.unwrap();
    assert_eq!(reading.topology.body_id, body_id);
    assert_eq!(reading.mass.body_id, body_id);
    assert_eq!(reading.topology.solid_count, 1);
    assert_eq!(reading.topology.face_count, 8);
    let pi = std::f64::consts::PI;
    near(reading.mass.volume, 4.0 * (width * 50.0 - 18.0 * pi));
    near(
        reading.mass.surface_area,
        2.0 * (width * 50.0 - 18.0 * pi) + 4.0 * (2.0 * (width + 50.0) + 12.0 * pi),
    );
    let plane = host.sketch().effective_plane();
    let center = plane.to_world(Vec2::new(width / 2.0, 25.0).unwrap());
    for (actual, expected) in reading.mass.centroid.into_iter().zip([
        center.x + 2.0 * plane.normal.x,
        center.y + 2.0 * plane.normal.y,
        center.z + 2.0 * plane.normal.z,
    ]) {
        near(actual, expected);
    }
}

async fn read_brep(
    host: &Harness,
    expected: onecad_app::api::PreviewContext,
) -> Result<crate::body_inspection_tests_support::Reading, String> {
    let body_id = host.app.display.as_ref().unwrap().scene.bodies()[0].body_id();
    let (reply, receive) = tokio::sync::oneshot::channel();
    host.app.bridge.submit(Command::InspectBody(Box::new(
        crate::body_inspection_tests_support::Request {
            expected,
            body_id,
            reply,
        },
    )))?;
    tokio::time::timeout(std::time::Duration::from_secs(8), receive)
        .await
        .expect("BRep inspection timed out")
        .expect("BRep inspection actor dropped reply")
}

pub(super) async fn assert_stale_brep_reads_refused(host: &Harness) {
    let current = &host.app.display.as_ref().unwrap().context;
    let mut other_document = current.clone();
    other_document.runtime_session.push_str("-obsolete");
    let mut other_revision = current.clone();
    other_revision.revision.0 += 1;
    let mut other_worker = current.clone();
    other_worker.worker_epoch.0 += 1;
    for stale in [other_document, other_revision, other_worker] {
        assert_eq!(
            read_brep(host, stale).await.unwrap_err(),
            "BRep inspection belongs to a stale document or worker head"
        );
    }
}
