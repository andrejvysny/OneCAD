use super::{SketchInteraction, SketchPhase, SketchSessionKey};
use crate::DocumentContext;
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{CurvePosition, Sketch, SketchEntity},
};
use onecad_scene::CaptureIdentity;

pub const MAX_SELECTION_TARGETS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchTarget {
    Entity(EntityId),
    Point {
        entity: EntityId,
        role: CurvePosition,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Replace,
    Add,
    Toggle,
    Subtract,
}

impl SketchTarget {
    pub fn owner_id(self) -> EntityId {
        match self {
            Self::Entity(entity) | Self::Point { entity, .. } => entity,
        }
    }

    pub fn is_valid(self, sketch: &Sketch) -> bool {
        match self {
            Self::Entity(id) => sketch.contains_entity(id),
            Self::Point { .. } => self.point_coordinates(sketch).is_some(),
        }
    }

    pub fn point_coordinates(self, sketch: &Sketch) -> Option<Vec2> {
        let Self::Point { entity, role } = self else {
            return None;
        };
        let point = match (sketch.get_entity(entity)?, role) {
            (SketchEntity::Point { at, .. }, CurvePosition::Arbitrary) => *at,
            (SketchEntity::Line { start, .. }, CurvePosition::Start) => raw_point(sketch, *start)?,
            (SketchEntity::Line { end, .. }, CurvePosition::End) => raw_point(sketch, *end)?,
            (
                SketchEntity::Circle { center, .. }
                | SketchEntity::Ellipse { center, .. }
                | SketchEntity::Arc { center, .. },
                CurvePosition::Center,
            ) => raw_point(sketch, *center)?,
            (
                SketchEntity::Arc {
                    center,
                    radius,
                    start_angle,
                    end_angle,
                    ..
                },
                CurvePosition::Start | CurvePosition::End,
            ) => {
                let center = raw_point(sketch, *center)?;
                let angle = if role == CurvePosition::Start {
                    *start_angle
                } else {
                    *end_angle
                };
                Vec2::new(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                )?
            }
            _ => return None,
        };
        point.is_finite().then_some(point)
    }

    /// Translate line handles only at the solver boundary; retain analytic curve roles.
    pub fn constraint_point(self, sketch: &Sketch) -> Option<(EntityId, CurvePosition)> {
        self.point_coordinates(sketch)?;
        let Self::Point { entity, role } = self else {
            return None;
        };
        Some(match (sketch.get_entity(entity)?, role) {
            (SketchEntity::Line { start, .. }, CurvePosition::Start) => {
                (*start, CurvePosition::Arbitrary)
            }
            (SketchEntity::Line { end, .. }, CurvePosition::End) => {
                (*end, CurvePosition::Arbitrary)
            }
            _ => (entity, role),
        })
    }
}

fn raw_point(sketch: &Sketch, id: EntityId) -> Option<Vec2> {
    match sketch.get_entity(id)? {
        SketchEntity::Point { at, .. } => Some(*at),
        _ => None,
    }
}

impl SketchInteraction {
    pub(crate) fn selection_scope(
        &self,
        key: &SketchSessionKey,
        document: &DocumentContext,
        capture: CaptureIdentity,
    ) -> bool {
        self.context.as_ref() == Some(document)
            && self.session.as_ref().is_some_and(|session| {
                session.key == *key
                    && session.document == *document
                    && session.phase == SketchPhase::Active
                    && session.accepted_capture == Some(capture)
                    && document.scene == capture.scene
            })
    }

    pub(crate) fn accept_view(
        &mut self,
        key: SketchSessionKey,
        document: DocumentContext,
        capture: CaptureIdentity,
    ) {
        if self.context.as_ref() != Some(&document) || document.scene != capture.scene {
            return;
        }
        let Some(session) = self.session.as_mut().filter(|session| {
            session.key == key
                && session.document == document
                && session.phase == SketchPhase::Active
        }) else {
            return;
        };
        if session.accepted_capture.is_some_and(|current| {
            current.view > capture.view || (current.view == capture.view && current != capture)
        }) {
            return;
        }
        if session.accepted_capture != Some(capture) {
            session.selection_dragging = false;
        }
        session.accepted_capture = Some(capture);
    }

    pub(crate) fn select(
        &mut self,
        key: SketchSessionKey,
        document: DocumentContext,
        capture: CaptureIdentity,
        targets: Vec<SketchTarget>,
        mode: SelectionMode,
    ) {
        if !self.selection_scope(&key, &document, capture) || targets.len() > MAX_SELECTION_TARGETS
        {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if targets
            .iter()
            .any(|target| !target.is_valid(&session.authoritative))
        {
            return;
        }
        let mut next = if mode == SelectionMode::Replace {
            Vec::new()
        } else {
            session.selection.clone()
        };
        let mut seen = Vec::new();
        for target in targets {
            if seen.contains(&target) {
                continue;
            }
            seen.push(target);
            if let Some(index) = next.iter().position(|current| *current == target) {
                if matches!(mode, SelectionMode::Toggle | SelectionMode::Subtract) {
                    next.remove(index);
                }
            } else if mode != SelectionMode::Subtract {
                next.push(target);
            }
        }
        if next.len() <= MAX_SELECTION_TARGETS {
            session.selection = next;
            session.selection_dragging = false;
        }
    }
}
