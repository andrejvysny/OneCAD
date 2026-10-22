//! A recognized pen stroke becomes exactly one Upsert batch; refusals author nothing.
#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::{
    edit::SketchEditOp,
    math::Vec2,
    sketch::{SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{
    recognition::{Recognition, StrokeRecognizer, StrokeShape},
    *,
};
use support::*;

fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}

fn commit(state: &mut SketchInteraction, shape: StrokeShape, seed: u128) -> Vec<SketchEffect> {
    let point = point(state, 0.0, 0.0);
    state.dispatch(SketchAction::CommitStroke {
        point,
        ids: ids(seed),
        stroke: Recognition::Shape(shape),
    })
}

#[test]
fn a_recognized_line_commits_one_batch_of_two_points_and_a_line() {
    let mut state = active(SketchPlane::xy(), SketchTool::PenStroke);
    let mut recognizer = StrokeRecognizer::new(1.0);
    for i in 0..=20 {
        recognizer.push(v(i as f64, 0.5 * i as f64));
    }
    let Recognition::Shape(shape) = recognizer.finish() else {
        panic!("a straight stroke is recognized");
    };
    let (tag, ops) = upsert(commit(&mut state, shape, 1));
    let kinds: Vec<_> = ops
        .iter()
        .map(|op| match op {
            SketchEditOp::AddEntity {
                entity: SketchEntity::Point { .. },
            } => "point",
            SketchEditOp::AddEntity {
                entity: SketchEntity::Line { .. },
            } => "line",
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["point", "point", "line"]);
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Active);
    assert_eq!(session.tool, SketchTool::PenStroke, "the tool stays armed");
}

#[test]
fn a_recognized_arc_commits_its_center_and_arc() {
    let mut state = active(SketchPlane::xy(), SketchTool::PenStroke);
    let shape = StrokeShape::Arc {
        start: v(10.0, 0.0),
        through: v(0.0, 10.0),
        end: v(-10.0, 0.0),
    };
    let (_, ops) = upsert(commit(&mut state, shape, 1));
    assert!(matches!(
        &ops[..],
        [
            SketchEditOp::AddEntity {
                entity: SketchEntity::Point { .. }
            },
            SketchEditOp::AddEntity {
                entity: SketchEntity::Arc { .. }
            }
        ]
    ));
}

#[test]
fn degenerate_or_misrouted_strokes_author_nothing() {
    let mut state = active(SketchPlane::xy(), SketchTool::PenStroke);
    let collinear = StrokeShape::Arc {
        start: v(0.0, 0.0),
        through: v(1.0, 0.0),
        end: v(2.0, 0.0),
    };
    assert!(commit(&mut state, collinear, 1).is_empty());
    let point_line = StrokeShape::Line {
        start: v(1.0, 1.0),
        end: v(1.0, 1.0),
    };
    assert!(commit(&mut state, point_line, 1).is_empty());
    let mut other_tool = active(SketchPlane::xy(), SketchTool::Line);
    let line = StrokeShape::Line {
        start: v(0.0, 0.0),
        end: v(5.0, 0.0),
    };
    assert!(commit(&mut other_tool, line, 1).is_empty());
    assert!(matches!(
        other_tool.session.as_ref().unwrap().status,
        SketchStatus::Refused(_)
    ));
}

#[test]
fn unrecognized_or_tiny_strokes_leave_no_history() {
    let mut state = active(SketchPlane::xy(), SketchTool::PenStroke);
    let at = point(&state, 0.0, 0.0);
    let effects = state.dispatch(SketchAction::CommitStroke {
        point: at.clone(),
        ids: ids(1),
        stroke: Recognition::Unrecognized("The stroke is too irregular to recognize"),
    });
    assert!(effects.is_empty());
    let SketchStatus::Refused(message) = &state.session.as_ref().unwrap().status else {
        panic!("an unrecognized stroke is reported");
    };
    assert!(message.starts_with("Stroke not recognized"), "{message}");
    let effects = state.dispatch(SketchAction::CommitStroke {
        point: at,
        ids: ids(1),
        stroke: Recognition::TooShort,
    });
    assert!(effects.is_empty(), "a stray touch is ignored");
}
