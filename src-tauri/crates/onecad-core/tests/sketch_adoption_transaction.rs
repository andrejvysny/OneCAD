use onecad_core::document::Document;
use onecad_core::edit::{DocumentSession, EditCommand, SketchEditOp};
use onecad_core::ids::{DocumentId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};
use uuid::Uuid;

#[test]
fn candidate_is_pure_and_authored_plus_solved_pose_is_one_undo_unit() {
    let id = SketchId(Uuid::from_u128(13));
    let point = EntityId(Uuid::from_u128(14));
    let mut session = DocumentSession::new(Document::new(DocumentId(Uuid::from_u128(15))));
    let original = Sketch::on_world_plane(id, "test", WorldPlane::XY);
    session
        .apply(EditCommand::AddSketch {
            sketch: original.clone(),
        })
        .unwrap();
    let depth = session.undo_depth();
    let ops = vec![SketchEditOp::AddEntity {
        entity: SketchEntity::point(point, Vec2::new(10.0, 1.0).unwrap(), false, false),
    }];
    let candidate = session.prepare_sketch_edit(id, &ops).unwrap();
    assert_eq!(session.document().sketch(id), Some(&original));
    assert_eq!(session.undo_depth(), depth);
    let mut solved = candidate.clone();
    solved.apply_solved_positions(&[(point, Vec2::new(10.0, 0.0).unwrap())]);
    session.begin_transaction("Edit Sketch");
    session
        .apply(EditCommand::SketchEdit { sketch: id, ops })
        .unwrap();
    session
        .apply(EditCommand::SketchDragGesture {
            sketch: id,
            before: candidate,
            after: solved.clone(),
        })
        .unwrap();
    let outcome = session.end_transaction().unwrap();
    assert!(outcome.projection_delta.sketches.contains(&id));
    assert_eq!(session.undo_depth(), depth + 1);
    assert_eq!(session.document().sketch(id), Some(&solved));
    session.undo().unwrap();
    assert_eq!(session.document().sketch(id), Some(&original));
    session.redo().unwrap().unwrap();
    assert_eq!(session.document().sketch(id), Some(&solved));
}
