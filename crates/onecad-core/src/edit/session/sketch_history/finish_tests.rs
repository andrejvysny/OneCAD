use super::*;
use crate::edit::undo::UNDO_CAP;
use crate::{
    ids::{DocumentId, EntityId},
    sketch::WorldPlane,
};

fn fixture() -> (DocumentSession, SketchId, EntityId, RecordId) {
    let mut session = DocumentSession::new(Document::new(DocumentId::new()));
    let id = SketchId::new();
    let point = EntityId::new();
    let record = RecordId::new();
    let mut sketch = Sketch::on_world_plane(id, "Width", WorldPlane::XY);
    sketch
        .add_entity(SketchEntity::point(
            point,
            Vec2::new(80.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    session.apply(EditCommand::AddSketch { sketch }).unwrap();
    session
        .apply(EditCommand::AddOperation {
            record: OperationRecord::new(record, 0, "Sketch", op(id, point, 80.0)),
            at_cursor: true,
        })
        .unwrap();
    (session, id, point, record)
}

fn op(id: SketchId, point: EntityId, width: f64) -> Operation {
    use crate::document::record::SketchOpParams;
    let params: SketchOpParams = serde_json::from_value(serde_json::json!({
        "sketchId": id, "plane": { "kind": "XY", "origin": [0,0,0],
            "xAxis": [0,1,0], "yAxis": [-1,0,0], "normal": [0,0,1] },
        "entities": [{ "id": point, "type": "Point", "at": [width,0], "construction": false }]
    }))
    .unwrap();
    Operation::Known(KnownOperation::Sketch(params))
}

fn net_edit(session: &mut DocumentSession, id: SketchId, point: EntityId) {
    let prior = session.document.sketches[&id].clone();
    session
        .apply(EditCommand::SketchEdit {
            sketch: id,
            ops: vec![SketchEditOp::SetEntityPositions {
                positions: vec![(point, Vec2::new(90.0, 0.0).unwrap())],
            }],
        })
        .unwrap();
    assert_eq!(
        session.squash_sketch_session(id, prior, 1),
        SketchSquashOutcome::NetStep
    );
}

#[test]
fn finished_sketch_record_refresh_preserves_cap_and_exact_history() {
    let (mut session, id, point, record) = fixture();
    for index in 0..UNDO_CAP {
        session
            .apply(EditCommand::RenameSketch {
                sketch: id,
                name: format!("Width {index}"),
            })
            .unwrap();
    }
    let before = session.document.sketches[&id].clone();
    net_edit(&mut session, id, point);
    let evictions = session.evictions();
    assert!(session
        .refresh_finished_sketch_record(id, record, op(id, point, 90.0))
        .unwrap()
        .is_some());
    assert_eq!(session.undo_depth(), UNDO_CAP);
    assert_eq!(
        session.evictions(),
        evictions,
        "record refresh must not evict another real step"
    );
    assert!(session.undo().unwrap().dirty_floor.is_some());
    assert_eq!(session.document.sketches[&id], before);
    assert_eq!(
        session.document.timeline.record(0).unwrap().op,
        op(id, point, 80.0)
    );
    assert!(session.redo().unwrap().is_some());
    assert_eq!(
        session.document.timeline.record(0).unwrap().op,
        op(id, point, 90.0)
    );
    assert_eq!(session.evictions(), evictions);
}

#[test]
fn finished_sketch_record_refresh_refuses_interleaving_and_open_transaction() {
    let (mut session, id, point, record) = fixture();
    net_edit(&mut session, id, point);
    session.begin_transaction("caller owned");
    assert!(session
        .refresh_finished_sketch_record(id, record, op(id, point, 90.0))
        .unwrap()
        .is_none());
    session.cancel_transaction();
    session
        .apply(EditCommand::RenameSketch {
            sketch: id,
            name: "Interleaved".into(),
        })
        .unwrap();
    let depth = session.undo_depth();
    assert!(session
        .refresh_finished_sketch_record(id, record, op(id, point, 90.0))
        .unwrap()
        .is_none());
    assert_eq!(session.undo_depth(), depth);
    assert_eq!(
        session.document.timeline.record(0).unwrap().op,
        op(id, point, 80.0)
    );
    assert!(session
        .refresh_finished_sketch_record(SketchId::new(), record, op(id, point, 90.0))
        .is_err());
    assert_eq!(session.undo_depth(), depth);
}
