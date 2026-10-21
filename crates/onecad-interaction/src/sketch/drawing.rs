use super::primitives::{build_primitive, distance, MIN_SIZE};
use super::*;
use onecad_core::{edit::SketchEditOp, sketch::Constraint};
use std::collections::BTreeSet;

impl SketchInteraction {
    pub(crate) fn set_tool(&mut self, tool: SketchTool) {
        let Some(session) = self
            .session
            .as_mut()
            .filter(|session| session.phase == SketchPhase::Active)
        else {
            return;
        };
        session.tool = tool;
        session.selection_dragging = false;
        session.draft = None;
        session.preview = None;
    }

    fn valid_point(&self, point: &CapturedSketchPoint) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && self.context.as_ref() == Some(&session.document)
                && session.key == point.key
                && session.document.scene == point.capture.scene
                && session.accepted_capture == Some(point.capture)
                && point.point.is_finite()
        })
    }

    pub(crate) fn moved(&mut self, point: CapturedSketchPoint) {
        if !self.valid_point(&point) {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.tool == SketchTool::Point {
            if session.draft.is_none() {
                session.draft_generation = session.draft_generation.saturating_add(1);
            }
            session.draft = Some(point_draft(point.point));
            session.preview = Some(DraftPrimitive::Point { at: point.point });
            return;
        }
        let Some(draft) = session.draft.as_mut() else {
            return;
        };
        draft.cursor = point.point;
        session.preview = match session.tool {
            SketchTool::Line if draft.tangent_arc => {
                super::tangent_continuation::preview(&session.authoritative, draft, point.point)
            }
            SketchTool::Line => Some(DraftPrimitive::Line {
                start: draft.anchor,
                end: point.point,
            }),
            SketchTool::CenterRectangle => {
                super::center_rectangle::opposite(draft.anchor, point.point)
                    .ok()
                    .map(|first| DraftPrimitive::Rectangle {
                        first,
                        opposite: point.point,
                    })
            }
            SketchTool::Rectangle => Some(DraftPrimitive::Rectangle {
                first: draft.anchor,
                opposite: point.point,
            }),
            SketchTool::Circle => Some(DraftPrimitive::Circle {
                center: draft.anchor,
                radius: distance(draft.anchor, point.point),
            }),
            SketchTool::Arc | SketchTool::ThreePointArc | SketchTool::Slot => {
                super::staged_creation::preview(session.tool, draft, point.point).ok()
            }
            SketchTool::Select | SketchTool::Point => None,
        };
    }

    pub(crate) fn clicked(
        &mut self,
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
    ) -> Vec<SketchEffect> {
        self.clicked_inferred(point, ids, None)
    }

    pub(crate) fn clicked_inferred(
        &mut self,
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
        snap: Option<super::inference::AcceptedSnap>,
    ) -> Vec<SketchEffect> {
        if !self.valid_point(&point) {
            return Vec::new();
        }
        if !self.current_inference(&point, snap) {
            return self.refuse("Snap intent belongs to a stale pointer capture or sketch");
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if session.tool == SketchTool::Select {
            return Vec::new();
        }
        if session.tool == SketchTool::Point {
            return self.submit_point(point, ids, snap);
        }
        let Some(draft) = session.draft.as_ref() else {
            session.draft_generation = session.draft_generation.saturating_add(1);
            session.draft = Some(SketchDraft {
                first: point.point,
                anchor: point.point,
                first_id: None,
                first_target: None,
                anchor_id: None,
                cursor: point.point,
                segments: 0,
                anchor_snap: snap,
                secondary: None,
                preceding: None,
                tangent_arc: false,
            });
            super::tangent_continuation::seed(session);
            return Vec::new();
        };
        if super::staged_creation::is_staged(session.tool) && draft.secondary.is_none() {
            return match super::staged_creation::second_anchor(session, point.point, snap) {
                Ok(()) => Vec::new(),
                Err(message) => self.refuse(&message),
            };
        }
        if session.tool == SketchTool::Line && distance(draft.anchor, point.point) < MIN_SIZE {
            self.clear_draft();
            return Vec::new();
        }
        let batch = build_primitive(session, point.point, &ids).and_then(|batch| {
            super::inferred_creation::append(session, point.capture, point.point, &ids, snap, batch)
        });
        match batch {
            Ok((ops, after)) => self.submit(ops, after),
            Err(message) => {
                if let Some(session) = self.session.as_mut() {
                    session.status = SketchStatus::Refused(message);
                }
                Vec::new()
            }
        }
    }

    pub(crate) fn submit(
        &mut self,
        ops: Vec<SketchEditOp>,
        after: AfterPrimitive,
    ) -> Vec<SketchEffect> {
        let Some(tag) = self.next_request() else {
            return Vec::new();
        };
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        session.phase = SketchPhase::Submitting;
        session.selection_dragging = false;
        session.status = SketchStatus::Pending;
        session.preview = None;
        session.batch = Some(PendingBatch {
            tag: tag.clone(),
            after,
            deferred: None,
            entities: ops
                .iter()
                .filter_map(|op| match op {
                    SketchEditOp::AddEntity { entity } => Some(entity.id()),
                    _ => None,
                })
                .collect(),
            constraints: ops
                .iter()
                .filter_map(|op| match op {
                    SketchEditOp::AddConstraint { constraint } => Some(constraint.id()),
                    _ => None,
                })
                .collect(),
        });
        vec![SketchEffect::Upsert { tag, ops }]
    }

    pub(crate) fn add_constraints(
        &mut self,
        key: SketchSessionKey,
        constraints: Vec<Constraint>,
    ) -> Vec<SketchEffect> {
        let Some(session) = self
            .session
            .as_ref()
            .filter(|session| session.key == key && session.phase == SketchPhase::Active)
        else {
            return Vec::new();
        };
        if constraints.is_empty() || constraints.len() > 128 {
            return self.refuse("Constraint batch must contain 1 to 128 constraints");
        }
        let ids: BTreeSet<_> = constraints.iter().map(Constraint::id).collect();
        if ids.len() != constraints.len()
            || constraints.iter().any(|constraint| {
                session.authoritative.contains_constraint(constraint.id())
                    || constraint
                        .entities()
                        .iter()
                        .any(|id| !session.authoritative.contains_entity(*id))
            })
        {
            return self
                .refuse("Constraints require fresh unique ids and existing sketch entities");
        }
        let effects = self.submit(
            constraints
                .iter()
                .cloned()
                .map(|constraint| SketchEditOp::AddConstraint { constraint })
                .collect(),
            AfterPrimitive::Preserve,
        );
        effects
            .into_iter()
            .map(|effect| match effect {
                SketchEffect::Upsert { tag, .. } => SketchEffect::AddConstraints {
                    tag,
                    constraints: constraints.clone(),
                },
                other => other,
            })
            .collect()
    }

    pub(crate) fn refuse(&mut self, message: &str) -> Vec<SketchEffect> {
        if let Some(session) = self.session.as_mut() {
            session.status = SketchStatus::Refused(message.into());
        }
        Vec::new()
    }
}

fn point_draft(cursor: onecad_core::math::Vec2) -> SketchDraft {
    let origin = onecad_core::math::Vec2::new_unchecked(0.0, 0.0);
    SketchDraft {
        first: origin,
        anchor: origin,
        cursor,
        first_id: None,
        first_target: None,
        anchor_id: None,
        segments: 0,
        anchor_snap: None,
        secondary: None,
        preceding: None,
        tangent_arc: false,
    }
}

impl SketchInteraction {
    pub(crate) fn begin_point_numeric(&mut self, key: SketchSessionKey) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let Some(capture) = session.accepted_capture else {
            return;
        };
        if session.tool != SketchTool::Point
            || !self.selection_scope(&key, &session.document, capture)
        {
            return;
        }
        let session = self.session.as_mut().expect("validated session");
        if session.draft.is_none() {
            session.draft_generation = session.draft_generation.saturating_add(1);
            session.draft = Some(point_draft(onecad_core::math::Vec2::new_unchecked(
                0.0, 0.0,
            )));
        }
    }

    fn current_inference(
        &self,
        point: &CapturedSketchPoint,
        snap: Option<super::inference::AcceptedSnap>,
    ) -> bool {
        match (self.session.as_ref(), snap) {
            (Some(session), Some(snap)) => {
                snap.point == point.point
                    && snap.source
                        == super::inference::source_for(
                            &session.authoritative,
                            point.capture,
                            session.request.sequence,
                        )
            }
            _ => true,
        }
    }

    fn submit_point(
        &mut self,
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
        snap: Option<super::inference::AcceptedSnap>,
    ) -> Vec<SketchEffect> {
        let session = self.session.as_ref().expect("validated point session");
        let batch = build_primitive(session, point.point, &ids).and_then(|batch| {
            super::inferred_creation::append(session, point.capture, point.point, &ids, snap, batch)
        });
        match batch {
            Ok((ops, after)) => self.submit(ops, after),
            Err(message) => self.refuse(&message),
        }
    }
}
