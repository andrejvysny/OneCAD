use super::primitives::{build_primitive, distance, MIN_SIZE};
use super::*;
use onecad_core::{edit::SketchEditOp, ids::EntityId, sketch::Constraint};
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
        session.draft = None;
        session.preview = None;
    }

    fn valid_point(&self, point: &CapturedSketchPoint) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && session.key == point.key
                && session.document.scene == point.capture.scene
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
        let Some(draft) = session.draft.as_mut() else {
            return;
        };
        draft.cursor = point.point;
        session.preview = match session.tool {
            SketchTool::Line => Some(DraftPrimitive::Line {
                start: draft.anchor,
                end: point.point,
            }),
            SketchTool::Rectangle => Some(DraftPrimitive::Rectangle {
                first: draft.anchor,
                opposite: point.point,
            }),
            SketchTool::Circle => Some(DraftPrimitive::Circle {
                center: draft.anchor,
                radius: distance(draft.anchor, point.point),
            }),
            SketchTool::Select => None,
        };
    }

    pub(crate) fn clicked(
        &mut self,
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
    ) -> Vec<SketchEffect> {
        if !self.valid_point(&point) {
            return Vec::new();
        }
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };
        if session.tool == SketchTool::Select {
            return Vec::new();
        }
        let Some(draft) = session.draft.as_ref() else {
            session.draft = Some(SketchDraft {
                first: point.point,
                anchor: point.point,
                first_id: None,
                anchor_id: None,
                cursor: point.point,
                segments: 0,
            });
            return Vec::new();
        };
        if session.tool == SketchTool::Line && distance(draft.anchor, point.point) < MIN_SIZE {
            self.clear_draft();
            return Vec::new();
        }
        let batch = build_primitive(session, point.point, &ids);
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

    pub(crate) fn select(&mut self, key: SketchSessionKey, entities: Vec<EntityId>) {
        let Some(session) = self
            .session
            .as_mut()
            .filter(|session| session.key == key && session.phase == SketchPhase::Active)
        else {
            return;
        };
        if entities.len() > session.authoritative.entities().len() {
            return;
        }
        session.selection = entities
            .into_iter()
            .filter(|id| session.authoritative.contains_entity(*id))
            .collect();
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
