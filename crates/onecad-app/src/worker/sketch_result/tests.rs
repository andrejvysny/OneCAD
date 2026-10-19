use super::*;
use crate::worker::wire::parse_sketch_upsert_checked;
use onecad_core::ids::SketchId;
use onecad_core::sketch::WorldPlane;
use serde_json::json;
use uuid::Uuid;

fn id(value: u128) -> EntityId {
    EntityId(Uuid::from_u128(value))
}

fn sketch() -> Sketch {
    let mut sketch = Sketch::on_world_plane(SketchId(Uuid::from_u128(10)), "test", WorldPlane::XY);
    for value in [1, 2] {
        sketch
            .add_entity(SketchEntity::point(
                id(value),
                Vec2::new_unchecked(0.0, 0.0),
                false,
                false,
            ))
            .unwrap();
    }
    sketch
        .add_entity(SketchEntity::circle(id(3), id(1), 5.0, false).unwrap())
        .unwrap();
    sketch
        .add_entity(SketchEntity::arc(id(4), id(1), 5.0, 0.0, 1.0, false).unwrap())
        .unwrap();
    sketch
}

fn receipt(sketch: &Sketch) -> Value {
    json!({"upserted":true, "sketchId":sketch.id.to_string(), "sketchRevision":1,
        "dof":4, "state":"UnderConstrained"})
}

#[test]
fn legacy_omission_and_unknown_curve_members_remain_compatible() {
    let sketch = sketch();
    let mut response = receipt(&sketch);
    assert!(parse_sketch_upsert_checked(&sketch, &response)
        .unwrap()
        .solved_positions
        .is_empty());
    response["curves"] = json!({id(3).to_string():{"radius":6.0, "futureMetadata":{"note":"ok"}},
        id(4).to_string():{"futureScalar":true}});
    let solved = parse_sketch_upsert_checked(&sketch, &response).unwrap();
    assert_eq!(solved.solved_curves.len(), 1);
    assert_eq!(solved.solved_curves[&id(3).to_string()].radius, Some(6.0));
}

#[test]
fn raw_malformed_payload_is_rejected_wholesale() {
    let sketch = sketch();
    for payload in [
        json!(null),
        json!([]),
        json!({id(1).to_string():[1]}),
        json!({id(1).to_string():[1,2,3]}),
        json!({id(1).to_string():[1,"bad"]}),
        json!({"bad-id":[1,2]}),
        json!({id(99).to_string():[1,2]}),
        json!({id(3).to_string():[1,2]}),
    ] {
        let mut response = receipt(&sketch);
        response["positions"] = payload;
        assert!(
            parse_sketch_upsert_checked(&sketch, &response).is_err(),
            "{response}"
        );
    }
    for payload in [
        json!(null),
        json!({id(3).to_string():{"radius":null}}),
        json!({id(3).to_string():{"startAngle":1.0}}),
        json!({id(3).to_string():{"radius":-1.0}}),
        json!({id(1).to_string():{"radius":1.0}}),
        json!({id(99).to_string():{"future":true}}),
        json!({id(1).to_string():{"future":true}}),
    ] {
        let mut response = receipt(&sketch);
        response["curves"] = payload;
        assert!(
            parse_sketch_upsert_checked(&sketch, &response).is_err(),
            "{response}"
        );
    }
}

#[test]
fn foreign_receipts_are_rejected() {
    let sketch = sketch();
    let mut response = receipt(&sketch);
    response["sketchId"] = json!("foreign");
    assert!(parse_sketch_upsert_checked(&sketch, &response).is_err());
}

#[test]
fn fake_solver_nonfinite_and_incompatible_curves_are_rejected() {
    let sketch = sketch();
    let mut solved = parse_sketch_upsert_checked(&sketch, &receipt(&sketch)).unwrap();
    solved
        .solved_positions
        .insert(id(1).to_string(), [f64::NAN, 0.0]);
    assert!(adopted(&sketch, &solved).is_err());
    solved.solved_positions.clear();
    solved.solved_curves.insert(
        id(3).to_string(),
        CurveParamsDto {
            radius: Some(f64::INFINITY),
            ..Default::default()
        },
    );
    assert!(adopted(&sketch, &solved).is_err());
}

#[test]
fn sub_preview_threshold_and_combined_curve_corrections_are_exact() {
    let sketch = sketch();
    let mut response = receipt(&sketch);
    response["positions"] = json!({id(2).to_string():[0.00000005,0.0]});
    response["curves"] = json!({id(3).to_string():{"radius":5.00000005},
        id(4).to_string():{"radius":6.0,"startAngle":0.0000000005,"endAngle":1.1}});
    let solved = parse_sketch_upsert_checked(&sketch, &response).unwrap();
    let after = adopted(&sketch, &solved).unwrap();
    assert_eq!(
        after.get_entity(id(2)).unwrap(),
        &SketchEntity::point(id(2), Vec2::new(0.00000005, 0.0).unwrap(), false, false)
    );
    assert!(matches!(after.get_entity(id(4)), Some(SketchEntity::Arc {
        radius, start_angle, end_angle, ..
    }) if *radius == 6.0 && *start_angle == 0.0000000005 && *end_angle == 1.1));
    assert_eq!(
        sketch.get_entity(id(2)).unwrap(),
        &SketchEntity::point(id(2), Vec2::new_unchecked(0.0, 0.0), false, false)
    );
}

#[test]
fn locked_curve_and_owned_point_preserve_exact_values() {
    let mut sketch = sketch();
    let locked = sketch
        .get_entity(id(3))
        .unwrap()
        .clone()
        .with_reference_locked(true);
    sketch.remove_entity(id(3));
    sketch.add_entity(locked).unwrap();
    let mut response = receipt(&sketch);
    response["positions"] = json!({id(1).to_string():[0.1,0.0]});
    assert!(parse_sketch_upsert_checked(&sketch, &response).is_err());
    response["positions"] = json!({});
    response["curves"] = json!({id(3).to_string():{"radius":5.1}});
    assert!(parse_sketch_upsert_checked(&sketch, &response).is_err());
}

#[test]
fn curve_centers_and_derived_endpoints_adopt_without_minting_points() {
    let sketch = sketch();
    let mut response = receipt(&sketch);
    response["positions"] = json!({format!("{}.center", id(3)):[2.0,3.0],
        format!("{}.center", id(4)):[2.0,3.0], format!("{}.start", id(4)):[8.0,3.0]});
    response["curves"] = json!({id(4).to_string():{"radius":6.0}});
    let solved = parse_sketch_upsert_checked(&sketch, &response).unwrap();
    let after = adopted(&sketch, &solved).unwrap();
    assert_eq!(after.entities().len(), sketch.entities().len());
    assert!(
        matches!(after.get_entity(id(1)), Some(SketchEntity::Point { at, .. }) if at.x == 2.0 && at.y == 3.0)
    );
    response["positions"][format!("{}.start", id(4))] = json!([9.0, 3.0]);
    assert!(parse_sketch_upsert_checked(&sketch, &response).is_err());
}

#[test]
fn alias_disagreement_is_rejected() {
    let sketch = sketch();
    for positions in [
        json!({format!("{}.center", id(3)):[2.0,3.0], format!("{}.center", id(4)):[3.0,3.0]}),
        json!({id(1).to_string():[1.0,1.0], format!("{}.center", id(3)):[2.0,3.0]}),
    ] {
        let mut response = receipt(&sketch);
        response["positions"] = positions;
        assert!(parse_sketch_upsert_checked(&sketch, &response).is_err());
    }
}

#[test]
fn aliases_cannot_move_explicit_fixed_point() {
    let mut sketch = sketch();
    sketch
        .add_constraint(onecad_core::sketch::Constraint::Fixed {
            id: onecad_core::ids::ConstraintId(Uuid::from_u128(12)),
            point: id(1),
            point_position: onecad_core::sketch::CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(0.0, 0.0),
        })
        .unwrap();
    let mut response = receipt(&sketch);
    response["positions"] = json!({format!("{}.center", id(3)):[2.0,3.0],
        format!("{}.center", id(4)):[2.0,3.0]});
    assert!(parse_sketch_upsert_checked(&sketch, &response).is_err());
}

#[test]
fn arc_endpoint_coupling_budget_is_pinned() {
    let sketch = sketch();
    for (error, accepted) in [(0.0001, true), (0.00010001, false)] {
        let mut response = receipt(&sketch);
        response["positions"] = json!({format!("{}.start", id(4)): [5.0, error]});
        assert_eq!(
            parse_sketch_upsert_checked(&sketch, &response).is_ok(),
            accepted
        );
    }
}

#[test]
fn canonical_or_alias_center_updates_every_shared_owner() {
    for handle in [id(1).to_string(), format!("{}.center", id(3))] {
        let mut sketch = sketch();
        sketch
            .add_entity(SketchEntity::line(id(5), id(1), id(2), false))
            .unwrap();
        sketch
            .add_entity(SketchEntity::ellipse(id(6), id(1), 4.0, 2.0, 0.0, false).unwrap())
            .unwrap();
        let mut response = receipt(&sketch);
        response["positions"] = json!({handle:[2.0,3.0]});
        let solved = parse_sketch_upsert_checked(&sketch, &response).unwrap();
        let after = adopted(&sketch, &solved).unwrap();
        assert!(
            matches!(after.get_entity(id(1)), Some(SketchEntity::Point { at, .. })
            if at.x == 2.0 && at.y == 3.0)
        );
        assert_eq!(after.entities().len(), sketch.entities().len());
    }
}

#[test]
fn canonical_writeback_expected_rounds_survive_rust_result_reader() {
    let fixture = include_str!("../../../../../protocol/fixtures/sketch_upsert_writeback.ndjson");
    let mut rounds = 0;
    for line in fixture
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let directive: Value = serde_json::from_str(line).unwrap();
        let Some(result) = directive.get("expect").and_then(|e| e.get("result")) else {
            continue;
        };
        let parsed = crate::worker::wire::parse_sketch_upsert("fixture", result);
        for (id, xy) in result
            .get("positions")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            assert_eq!(
                parsed.solved_positions[id],
                [xy[0].as_f64().unwrap(), xy[1].as_f64().unwrap()]
            );
        }
        for (id, curve) in result
            .get("curves")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let parsed = &parsed.solved_curves[id];
            assert_eq!(parsed.radius, curve.get("radius").and_then(Value::as_f64));
            assert_eq!(
                parsed.start_angle,
                curve.get("startAngle").and_then(Value::as_f64)
            );
            assert_eq!(
                parsed.end_angle,
                curve.get("endAngle").and_then(Value::as_f64)
            );
        }
        rounds += 1;
    }
    assert_eq!(rounds, 7);
}
