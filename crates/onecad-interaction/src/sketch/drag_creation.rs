//! Exact tool geometry shares click creation's inference and application admission.
use super::{
    inference::{source_for, AcceptedSnap},
    *,
};

#[derive(Clone, Debug)]
pub struct PrimitiveDrag {
    pub start: CapturedSketchPoint,
    pub end: CapturedSketchPoint,
    pub start_snap: Option<AcceptedSnap>,
    pub end_snap: Option<AcceptedSnap>,
    pub tool: SketchTool,
    pub draft_generation: u64,
    pub request_sequence: u64,
    pub construction: bool,
    pub ids: PrimitiveIds,
}

impl SketchInteraction {
    pub(crate) fn commit_primitive_drag(&mut self, drag: PrimitiveDrag) -> Vec<SketchEffect> {
        if !self.valid_point(&drag.start)
            || !self.valid_point(&drag.end)
            || drag.start.capture != drag.end.capture
        {
            return Vec::new();
        }
        let Some(session) = self.session.as_ref() else {
            return Vec::new();
        };
        if !drag.current(session) {
            return Vec::new();
        }
        if !valid_snap(session, &drag.start, drag.start_snap)
            || !valid_snap(session, &drag.end, drag.end_snap)
        {
            return self.refuse("Drag inference belongs to an obsolete pointer capture or sketch");
        }
        match drag.build(session) {
            Ok((ops, after)) => self.submit(ops, after),
            Err(message) => self.refuse(&message),
        }
    }
}

impl PrimitiveDrag {
    fn current(&self, session: &SketchSession) -> bool {
        session.tool == self.tool
            && session.draft_generation == self.draft_generation
            && session.request.sequence == self.request_sequence
            && session.construction == self.construction
            && session.numeric_method() != NumericCreationMethod::TangentArc
            && matches!(
                self.tool,
                SketchTool::Line
                    | SketchTool::Rectangle
                    | SketchTool::CenterRectangle
                    | SketchTool::Circle
            )
    }

    fn build(&self, session: &SketchSession) -> Result<super::primitives::PrimitiveBatch, String> {
        let mut candidate = session.clone();
        candidate.draft = Some(drag_draft(self.start.point, self.start_snap));
        let (ops, _) = super::primitives::build_primitive(&candidate, self.end.point, &self.ids)?;
        // A lifted Pencil finishes an independent primitive; Undo must not merely clear a chain draft.
        let batch = (ops, AfterPrimitive::Reset);
        super::inferred_creation::append(
            &candidate,
            self.end.capture,
            self.end.point,
            &self.ids,
            self.end_snap,
            batch,
        )
    }
}

fn valid_snap(
    session: &SketchSession,
    point: &CapturedSketchPoint,
    snap: Option<AcceptedSnap>,
) -> bool {
    snap.is_none_or(|snap| {
        snap.point == point.point
            && snap.source
                == source_for(
                    &session.authoritative,
                    point.capture,
                    session.request.sequence,
                )
    })
}

fn drag_draft(point: onecad_core::math::Vec2, snap: Option<AcceptedSnap>) -> SketchDraft {
    SketchDraft {
        first: point,
        anchor: point,
        first_id: None,
        first_target: None,
        anchor_id: None,
        cursor: point,
        segments: 0,
        anchor_snap: snap,
        secondary: None,
        preceding: None,
        tangent_arc: false,
    }
}
