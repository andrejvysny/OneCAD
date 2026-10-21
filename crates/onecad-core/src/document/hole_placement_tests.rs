//! `Hole.points` (ONECAD-361): wire shape, the version fence, spacing and the
//! canonical writer.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::*;
use crate::document::record::{HoleParams, HoleType, KnownOperation, Operation};
use crate::document::refs::{ElementKind, ElementRef, PrimaryRef};
use crate::document::variables::Scalar;
use crate::ids::{BodyId, ElementId};

fn body() -> BodyId {
    BodyId(Uuid::from_u128(1))
}

fn pid(n: u128) -> HolePointId {
    HolePointId(Uuid::from_u128(0x9000 + n))
}

fn v(x: f64, y: f64) -> Vec3 {
    Vec3::new_unchecked(x, y, 30.0)
}

fn single() -> HoleParams {
    HoleParams {
        target_body: body(),
        face: ElementRef {
            primary: Some(PrimaryRef {
                body: body(),
                element: ElementId::new("el_face_top"),
                kind: ElementKind::Face,
                extra: Extra::new(),
            }),
            intent: None,
            anchor: None,
            extra: Extra::new(),
        },
        point: v(0.0, 0.0),
        points: None,
        hole_type: HoleType::Simple,
        diameter: Scalar::new(5.5),
        depth: Some(Scalar::new(20.0)),
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        thread: None,
        result_policy_version: Some(2),
        extra: Extra::new(),
    }
}

/// A V3 record at `xs` along +X (ids 0..n), `point == points[0].at`.
fn multi(xs: &[f64]) -> HoleParams {
    let points: Vec<HolePoint> = xs
        .iter()
        .enumerate()
        .map(|(k, x)| HolePoint {
            id: pid(k as u128),
            at: v(*x, 0.0),
            extra: Extra::new(),
        })
        .collect();
    HoleParams {
        point: points[0].at,
        points: Some(points),
        result_policy_version: Some(HOLE_POINTS_POLICY_VERSION),
        ..single()
    }
}

fn to_json(p: &HoleParams) -> serde_json::Value {
    serde_json::to_value(Operation::Known(KnownOperation::Hole(p.clone()))).unwrap()
}

#[test]
fn legacy_records_round_trip_byte_identically_without_points() {
    for version in [None, Some(2)] {
        let p = HoleParams {
            result_policy_version: version,
            ..single()
        };
        let text = serde_json::to_string(&Operation::Known(KnownOperation::Hole(p))).unwrap();
        assert!(
            !text.contains("points"),
            "legacy shape has no points: {text}"
        );
        let back: Operation = serde_json::from_str(&text).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }
}

#[test]
fn v3_serializes_points_with_ids_right_after_point() {
    let text = serde_json::to_string(&Operation::Known(KnownOperation::Hole(multi(&[0.0, 10.0]))))
        .unwrap();
    let point = text.find("\"point\"").unwrap();
    let points = text.find("\"points\"").unwrap();
    let hole_type = text.find("\"holeType\"").unwrap();
    assert!(point < points && points < hole_type, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let p = &json["params"];
    assert_eq!(p["resultPolicyVersion"], serde_json::json!(3));
    assert_eq!(p["points"][1]["at"], serde_json::json!([10.0, 0.0, 30.0]));
    assert_eq!(p["points"][1]["id"], serde_json::json!(pid(1).to_string()));
    assert_eq!(p["point"], p["points"][0]["at"]);
    let back: Operation = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(back).unwrap(), json);
}

#[test]
fn version_and_points_fence() {
    assert!(single().validate().is_ok());
    assert!(HoleParams {
        result_policy_version: None,
        ..single()
    }
    .validate()
    .is_ok());
    let no_points = HoleParams {
        result_policy_version: Some(3),
        ..single()
    };
    assert!(no_points
        .validate()
        .unwrap_err()
        .contains("requires points"));
    for version in [None, Some(2), Some(4)] {
        let p = HoleParams {
            result_policy_version: version,
            ..multi(&[0.0, 10.0])
        };
        assert!(p
            .validate()
            .unwrap_err()
            .contains("require resultPolicyVersion 3"));
    }
    let future = HoleParams {
        result_policy_version: Some(4),
        ..single()
    };
    assert!(future.validate().unwrap_err().contains("unsupported"));
}

#[test]
fn point_count_bounds() {
    let mut empty = multi(&[0.0]);
    empty.points = Some(Vec::new());
    assert!(empty.validate().unwrap_err().contains("1..=64"));
    assert!(
        multi(&[0.0]).validate().is_ok(),
        "a 1-point V3 record is valid"
    );
    assert!(multi(&[0.0, 10.0]).validate().is_ok());
    let xs64: Vec<f64> = (0..64).map(|k| f64::from(k) * 10.0).collect();
    assert!(multi(&xs64).validate().is_ok());
    let xs65: Vec<f64> = (0..65).map(|k| f64::from(k) * 10.0).collect();
    assert!(multi(&xs65).validate().unwrap_err().contains("1..=64"));
}

#[test]
fn point_shape_rules() {
    let mut mismatch = multi(&[0.0, 10.0]);
    mismatch.point = v(0.0, 1e-12);
    assert!(mismatch.validate().unwrap_err().contains("points[0]"));

    let mut nan = multi(&[0.0, 10.0]);
    nan.points.as_mut().unwrap()[1].at.y = f64::NAN;
    assert!(nan.validate().unwrap_err().contains("points[1]"));

    let mut dup = multi(&[0.0, 10.0]);
    dup.points.as_mut().unwrap()[1].id = pid(0);
    assert!(dup.validate().unwrap_err().contains("repeats id"));
}

#[test]
fn spacing_uses_the_widest_tool_radius_with_the_authoring_margin() {
    // Simple Ø5.5 ⇒ R = 2.75; counterbore Ø9.5 ⇒ 4.75; countersink Ø11 at 90°
    // ⇒ 5.5 + 0.01·tan(45°) = 5.51 (the overshoot frustum, not cs/2).
    let counterbore = |xs: &[f64]| HoleParams {
        hole_type: HoleType::Counterbore,
        cb_diameter: Some(Scalar::new(9.5)),
        cb_depth: Some(Scalar::new(5.4)),
        ..multi(xs)
    };
    let countersink = |xs: &[f64]| HoleParams {
        hole_type: HoleType::Countersink,
        cs_diameter: Some(Scalar::new(11.0)),
        cs_angle_deg: Some(Scalar::new(90.0)),
        ..multi(xs)
    };
    let cases = [
        (multi(&[0.0]), 2.75),
        (counterbore(&[0.0]), 4.75),
        (countersink(&[0.0]), 5.51),
    ];
    for (profile, radius) in cases {
        let build = |xs: &[f64]| HoleParams {
            point: multi(xs).point,
            points: multi(xs).points,
            ..profile.clone()
        };
        let min = 2.0 * radius + HOLE_AUTHORING_GAP_MM;
        assert!((build(&[0.0]).seat_radius() - radius).abs() < 1e-12);
        assert!(
            build(&[0.0, min + 1e-9]).validate().is_ok(),
            "R={radius} boundary"
        );
        let err = build(&[0.0, 20.0, min - 1e-6]).validate().unwrap_err();
        assert!(err.contains("points[0] and points[2] overlap"), "{err}");
    }
    // A countersink pair clearing cs/2 alone still overlaps above the face.
    assert!(countersink(&[0.0, 11.01]).validate().is_err());
    let err = multi(&[0.0, 5e-4]).validate().unwrap_err();
    assert!(err.contains("coincide"), "{err}");
}

#[test]
fn set_placements_normalizes_shape_and_version() {
    // Legacy 1 → 1 stays legacy, version untouched (never silently upgraded).
    for version in [None, Some(2)] {
        let mut p = HoleParams {
            result_policy_version: version,
            ..single()
        };
        p.set_placements(vec![HolePlacement {
            id: None,
            at: v(1.0, 1.0),
        }])
        .unwrap();
        assert_eq!((p.point, p.points.is_none()), (v(1.0, 1.0), true));
        assert_eq!(p.result_policy_version, version);
    }
    // 1 → 3 mints ids and writes V3.
    let mut p = HoleParams {
        result_policy_version: None,
        ..single()
    };
    let mut list = p.placements();
    list.push(HolePlacement {
        id: None,
        at: v(10.0, 0.0),
    });
    list.push(HolePlacement {
        id: None,
        at: v(20.0, 0.0),
    });
    p.set_placements(list).unwrap();
    assert_eq!(p.result_policy_version, Some(HOLE_POINTS_POLICY_VERSION));
    let ids: Vec<HolePointId> = p.placements().iter().map(|h| h.id.unwrap()).collect();
    assert_eq!(ids.len(), 3);
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
    assert_eq!(p.point, v(0.0, 0.0));
    assert!(p.validate().is_ok());

    // 3 → 2 (drop the first, move the last): ids follow their points.
    let next = vec![
        HolePlacement {
            id: Some(ids[1]),
            at: v(10.0, 0.0),
        },
        HolePlacement {
            id: Some(ids[2]),
            at: v(25.0, 0.0),
        },
    ];
    p.set_placements(next).unwrap();
    assert_eq!(p.point, v(10.0, 0.0));
    assert_eq!(
        p.placements().iter().map(|h| h.id).collect::<Vec<_>>(),
        vec![Some(ids[1]), Some(ids[2])]
    );
    // 2 → 1 keeps V3 so the survivor keeps its id.
    p.set_placements(vec![HolePlacement {
        id: Some(ids[2]),
        at: v(25.0, 0.0),
    }])
    .unwrap();
    assert_eq!(p.result_policy_version, Some(HOLE_POINTS_POLICY_VERSION));
    assert_eq!(p.placements()[0].id, Some(ids[2]));
    assert!(p.validate().is_ok());

    // Refusals: empty, too many, foreign or removed id, duplicate id.
    assert!(p.set_placements(Vec::new()).is_err());
    let many = vec![
        HolePlacement {
            id: None,
            at: v(0.0, 0.0)
        };
        65
    ];
    assert!(p.set_placements(many).is_err());
    let removed = HolePlacement {
        id: Some(ids[0]),
        at: v(0.0, 0.0),
    };
    assert!(p
        .set_placements(vec![removed])
        .unwrap_err()
        .contains("not a placement"));
    let twice = HolePlacement {
        id: Some(ids[2]),
        at: v(0.0, 0.0),
    };
    assert!(p
        .set_placements(vec![twice, twice])
        .unwrap_err()
        .contains("twice"));
}

/// An old build has no `points` field: it must still open a V3 record and
/// write `points` back untouched through its flattened `Extra`.
#[test]
fn an_old_reader_round_trips_points_through_extra() {
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OldHole {
        point: Vec3,
        #[serde(flatten)]
        extra: Extra,
    }
    let params = to_json(&multi(&[0.0, 10.0]))["params"].clone();
    let old: OldHole = serde_json::from_value(params.clone()).unwrap();
    assert!(old.extra.contains_key("points"));
    let back = serde_json::to_value(&old).unwrap();
    assert_eq!(back["points"], params["points"]);
    assert_eq!(back["point"], params["point"]);
}
