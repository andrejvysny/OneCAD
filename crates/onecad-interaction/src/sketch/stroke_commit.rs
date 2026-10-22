//! A recognized pen stroke commits as one Upsert batch through the same
//! session, capture and id checks as clicked primitives; an unrecognized one is
//! refused (session status) and leaves no history.
use super::{
    primitives::{distance, mark_construction, validate_ids, MIN_SIZE},
    recognition::{Recognition, StrokeShape},
    *,
};
use onecad_core::{edit::SketchEditOp, sketch::SketchEntity};

impl SketchInteraction {
    pub(crate) fn commit_stroke(
        &mut self,
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
        stroke: Recognition,
    ) -> Vec<SketchEffect> {
        // A stroke finished while an earlier batch is in flight cannot commit;
        // say so rather than drop it silently.
        let busy = self.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Submitting && session.tool == SketchTool::PenStroke
        });
        if busy && stroke != Recognition::TooShort {
            return self
                .refuse("The previous sketch edit is still being applied; draw the stroke again");
        }
        if !self.valid_point(&point) {
            return Vec::new();
        }
        let Some(session) = self.session.as_ref() else {
            return Vec::new();
        };
        if session.tool != SketchTool::PenStroke {
            return self.refuse("Choose the Pencil Stroke tool before committing a stroke");
        }
        let shape = match stroke {
            Recognition::Shape(shape) => shape,
            Recognition::TooShort => return Vec::new(),
            Recognition::Unrecognized(reason) => {
                return self.refuse(&format!(
                    "Stroke not recognized: {reason}. Nothing was added."
                ));
            }
        };
        match stroke_ops(session, shape, &ids) {
            Ok(ops) => self.submit(ops, AfterPrimitive::Reset),
            Err(message) => self.refuse(&message),
        }
    }
}

fn stroke_ops(
    session: &SketchSession,
    shape: StrokeShape,
    ids: &PrimitiveIds,
) -> Result<Vec<SketchEditOp>, String> {
    validate_ids(session, ids)?;
    let mut ops = match shape {
        StrokeShape::Line { start, end } => {
            let length = distance(start, end);
            if !length.is_finite() || length < MIN_SIZE {
                return Err("Degenerate line".into());
            }
            let [first, second, line] = [ids.entities[0], ids.entities[1], ids.entities[2]];
            vec![
                add(SketchEntity::point(first, start, false, false)),
                add(SketchEntity::point(second, end, false, false)),
                add(SketchEntity::line(line, first, second, false)),
            ]
        }
        StrokeShape::Arc {
            start,
            through,
            end,
        } => {
            let arc = super::arc_geometry::through_points(start, end, through)
                .map_err(|error| error.to_string())?;
            super::staged_creation::arc_ops(arc, ids)?
        }
    };
    if session.construction {
        mark_construction(&mut ops);
    }
    Ok(ops)
}

fn add(entity: SketchEntity) -> SketchEditOp {
    SketchEditOp::AddEntity { entity }
}
