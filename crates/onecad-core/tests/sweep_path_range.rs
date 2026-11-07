use onecad_core::{
    document::{
        record::{
            BooleanMode, KnownOperation, Operation, OperationRecord, SweepParams, SweepPathRange,
        },
        refs::Extra,
        Document,
    },
    edit::{DocumentSession, EditCommand},
    ids::{DocumentId, RecordId},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn params() -> SweepParams {
    serde_json::from_value(json!({"booleanMode":"NewBody"})).unwrap()
}

#[test]
fn omitted_range_keeps_closed_payload_but_explicit_null_is_refused() {
    let legacy = params();
    assert_eq!(legacy.path_range, None);
    assert!(serde_json::to_value(legacy)
        .unwrap()
        .get("pathRange")
        .is_none());
    for range in [
        Value::Null,
        json!([]),
        json!(0),
        json!({}),
        json!({"startRadians":0,"endRadians":1,"twist":0}),
    ] {
        assert!(
            serde_json::from_value::<SweepParams>(json!({
                "booleanMode":"NewBody", "pathRange":range
            }))
            .is_err(),
            "accepted {range}"
        );
    }
}

#[test]
fn malformed_or_unbounded_ranges_do_not_become_closed_sweeps() {
    for (start, end) in [
        (0., 0.),
        (1., 0.),
        (0., std::f64::consts::TAU),
        (-7., -6.),
        (6., 7.),
    ] {
        assert!(serde_json::from_value::<SweepParams>(json!({
            "booleanMode":"NewBody",
            "pathRange":{"startRadians":start,"endRadians":end}
        }))
        .is_err());
    }
    assert!(serde_json::from_str::<SweepParams>(
        r#"{"booleanMode":"NewBody","pathRange":{"startRadians":0,"endRadians":1e999}}"#,
    )
    .is_err());
    for start in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(SweepPathRange {
            start_radians: start,
            end_radians: 1.
        }
        .validate()
        .is_err());
    }
}

// The pre-range reader retains new fields in its existing flattened map.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySweepParams {
    boolean_mode: BooleanMode,
    #[serde(flatten)]
    extra: Extra,
}

#[test]
fn older_reader_save_preserves_range_and_new_reader_restores_exact_angles() {
    let range = SweepPathRange {
        start_radians: -0.154942713148,
        end_radians: f64::from_bits(0x3fc3_1e8e_b437_ee56),
    };
    let mut current = params();
    current.path_range = Some(range);
    let bytes = serde_json::to_vec(&current).unwrap();
    let older: LegacySweepParams = serde_json::from_slice(&bytes).unwrap();
    assert!(older.extra.contains_key("pathRange"));
    let saved_by_older = serde_json::to_vec(&older).unwrap();
    let reopened: SweepParams = serde_json::from_slice(&saved_by_older).unwrap();
    let actual = reopened.path_range.unwrap();
    assert_eq!(
        actual.start_radians.to_bits(),
        range.start_radians.to_bits()
    );
    assert_eq!(actual.end_radians.to_bits(), range.end_radians.to_bits());
    assert_eq!(
        serde_json::to_value(reopened).unwrap(),
        serde_json::to_value(current).unwrap()
    );
}

#[test]
fn invalid_typed_range_and_extra_collision_leave_writer_and_undo_unchanged() {
    let mut invalid = params();
    invalid.path_range = Some(SweepPathRange {
        start_radians: 1.,
        end_radians: 0.,
    });
    let mut collision = params();
    collision
        .extra
        .insert("pathRange".into(), json!({"startRadians":0,"endRadians":1}));
    let mut duplicate = collision.clone();
    duplicate.path_range = Some(SweepPathRange {
        start_radians: 0.,
        end_radians: 1.,
    });
    for p in [invalid, collision, duplicate] {
        let mut session = DocumentSession::new(Document::new(DocumentId::new()));
        let before = serde_json::to_vec(session.document()).unwrap();
        let command = EditCommand::AddOperation {
            record: OperationRecord::new(
                RecordId::new(),
                0,
                "Sweep",
                Operation::Known(KnownOperation::Sweep(p)),
            ),
            at_cursor: true,
        };
        assert!(session.apply(command).is_err());
        assert_eq!(serde_json::to_vec(session.document()).unwrap(), before);
        assert_eq!(session.undo_depth(), 0);
    }
}
