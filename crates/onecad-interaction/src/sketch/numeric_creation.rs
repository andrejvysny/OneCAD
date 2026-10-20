//! Numeric creation builds one candidate batch; application admission resolves its expressions.
use super::*;
use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    ids::EntityId,
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity},
};
use onecad_scene::CaptureIdentity;

const METHOD_MISMATCH: &str = "Numeric dimensions do not match the active sketch method";

#[derive(Clone, Debug, PartialEq)]
pub enum CreationDimensions {
    Point { x: Scalar, y: Scalar },
    CenterRectangle { width: Scalar, height: Scalar },
    Line { length: Scalar },
    Rectangle { width: Scalar, height: Scalar },
    Circle { diameter: Scalar },
    Arc { radius: Scalar, sweep: f64 },
    Slot { width: Scalar },
    TangentArc { radius: Scalar },
    ThreePointArc { radius: Scalar },
}

impl CreationDimensions {
    pub fn endpoint(&self, anchor: Vec2, cursor: Vec2) -> Result<Vec2, String> {
        if !anchor.is_finite() || !cursor.is_finite() {
            return Err("Numeric creation requires finite anchor and cursor coordinates".into());
        }
        let endpoint = match self {
            Self::Point { x, y } => {
                Vec2::new(finite(x)?, finite(y)?).ok_or("Point coordinates must be finite")?
            }
            Self::CenterRectangle { width, height } => Vec2::new(
                anchor.x + signed(positive(width)? * 0.5, cursor.x - anchor.x),
                anchor.y + signed(positive(height)? * 0.5, cursor.y - anchor.y),
            )
            .ok_or("Numeric center rectangle coordinates are not finite")?,
            Self::Line { length } => directional(anchor, cursor, positive(length)?, false)?,
            Self::Circle { diameter } => {
                directional(anchor, cursor, positive(diameter)? * 0.5, true)?
            }
            Self::Arc { .. }
            | Self::Slot { .. }
            | Self::TangentArc { .. }
            | Self::ThreePointArc { .. } => {
                return Err("Numeric arc and slot creation require two anchors".into());
            }
            Self::Rectangle { width, height } => Vec2::new(
                anchor.x + signed(positive(width)?, cursor.x - anchor.x),
                anchor.y + signed(positive(height)?, cursor.y - anchor.y),
            )
            .ok_or("Numeric rectangle coordinates are not finite")?,
        };
        Ok(endpoint)
    }

    pub fn endpoint_for_draft(&self, draft: &SketchDraft) -> Result<Vec2, String> {
        super::staged_numeric::endpoint(self, draft)
    }

    pub fn preview(&self, draft: &SketchDraft) -> Result<DraftPrimitive, String> {
        if let Self::Arc { .. } | Self::ThreePointArc { .. } = self {
            return super::staged_numeric::draft_arc(self, draft)
                .map(|arc| DraftPrimitive::Arc { arc });
        }
        let endpoint = self.endpoint_for_draft(draft)?;
        match self {
            Self::Point { .. } => Ok(DraftPrimitive::Point { at: endpoint }),
            Self::Line { .. } => Ok(DraftPrimitive::Line {
                start: draft.anchor,
                end: endpoint,
            }),
            Self::Rectangle { .. } => Ok(DraftPrimitive::Rectangle {
                first: draft.anchor,
                opposite: endpoint,
            }),
            Self::CenterRectangle { .. } => Ok(DraftPrimitive::Rectangle {
                first: super::center_rectangle::opposite(draft.anchor, endpoint)?,
                opposite: endpoint,
            }),
            Self::Circle { .. } => Ok(DraftPrimitive::Circle {
                center: draft.anchor,
                radius: super::primitives::distance(draft.anchor, endpoint),
            }),
            Self::Slot { .. } => super::staged_creation::preview(SketchTool::Slot, draft, endpoint),
            Self::Arc { .. } | Self::ThreePointArc { .. } => {
                unreachable!("arc preview returned above")
            }
            Self::TangentArc { .. } => Err("Tangent arc preview requires the solved sketch".into()),
        }
    }

    pub fn preview_for_session(&self, session: &SketchSession) -> Result<DraftPrimitive, String> {
        let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
        if matches!(self, Self::TangentArc { .. }) {
            return super::staged_numeric::session_arc(self, session)
                .map(|arc| DraftPrimitive::Arc { arc });
        }
        self.preview(draft)
    }

    fn method(&self) -> NumericCreationMethod {
        NumericCreationMethod::Tool(match self {
            Self::Point { .. } => SketchTool::Point,
            Self::CenterRectangle { .. } => SketchTool::CenterRectangle,
            Self::Line { .. } => SketchTool::Line,
            Self::Rectangle { .. } => SketchTool::Rectangle,
            Self::Circle { .. } => SketchTool::Circle,
            Self::Arc { .. } => SketchTool::Arc,
            Self::Slot { .. } => SketchTool::Slot,
            Self::ThreePointArc { .. } => SketchTool::ThreePointArc,
            Self::TangentArc { .. } => return NumericCreationMethod::TangentArc,
        })
    }
}

fn finite(value: &Scalar) -> Result<f64, String> {
    if value.value.is_finite() {
        Ok(value.value)
    } else {
        Err("Creation coordinates must be finite".into())
    }
}

fn positive(value: &Scalar) -> Result<f64, String> {
    if value.value.is_finite() && value.value > 0.0 {
        Ok(value.value)
    } else {
        Err("Creation dimensions must be finite and greater than zero".into())
    }
}

fn signed(value: f64, direction: f64) -> f64 {
    if direction < 0.0 {
        -value
    } else {
        value
    }
}

fn directional(
    anchor: Vec2,
    cursor: Vec2,
    length: f64,
    allow_default: bool,
) -> Result<Vec2, String> {
    let (dx, dy) = (cursor.x - anchor.x, cursor.y - anchor.y);
    let norm = dx.hypot(dy);
    if !norm.is_finite() {
        return Err("Numeric creation direction is not finite".into());
    }
    let (x, y) = if norm > 0.0 {
        (dx / norm, dy / norm)
    } else if allow_default {
        (1.0, 0.0)
    } else {
        return Err("Move the cursor to establish a line direction".into());
    };
    Vec2::new(anchor.x + x * length, anchor.y + y * length)
        .ok_or_else(|| "Numeric creation coordinates are not finite".into())
}

impl SketchInteraction {
    pub(crate) fn create_numeric(
        &mut self,
        key: SketchSessionKey,
        capture: CaptureIdentity,
        ids: PrimitiveIds,
        dimensions: CreationDimensions,
    ) -> Vec<SketchEffect> {
        let Some(session) = self.session.as_ref() else {
            return Vec::new();
        };
        if !self.selection_scope(&key, &session.document, capture) {
            return Vec::new();
        }
        if session.numeric_method() != dimensions.method() {
            return self.refuse(METHOD_MISMATCH);
        }
        let Some(_) = session.draft.as_ref() else {
            return self.refuse("Place a primitive anchor before numeric creation");
        };
        let batch =
            super::staged_numeric::session_endpoint(&dimensions, session).and_then(|endpoint| {
                let batch = super::staged_numeric::build(&dimensions, session, endpoint, &ids)?;
                let (mut ops, after) = if session.tool == SketchTool::Point {
                    batch
                } else {
                    super::inferred_creation::append(session, capture, endpoint, &ids, None, batch)?
                };
                append_dimensions(&mut ops, &ids, &dimensions)?;
                Ok((ops, after))
            });
        match batch {
            Ok((ops, after)) => self.submit(ops, after),
            Err(message) => self.refuse(&message),
        }
    }
}

fn append_dimensions(
    ops: &mut Vec<SketchEditOp>,
    ids: &PrimitiveIds,
    dimensions: &CreationDimensions,
) -> Result<(), String> {
    let constraints = match dimensions {
        CreationDimensions::Point { x, y } => point_dimensions(ops, ids, x, y),
        CreationDimensions::Line { length } => {
            let (start, end) = ops
                .iter()
                .find_map(|op| match op {
                    SketchEditOp::AddEntity {
                        entity: SketchEntity::Line { id, start, end, .. },
                    } if *id == ids.entities[2] => Some((*start, *end)),
                    _ => None,
                })
                .ok_or("Numeric line batch is missing its endpoint identities")?;
            vec![distance(ids.constraints[8], start, end, length.clone())]
        }
        CreationDimensions::Rectangle { width, height }
        | CreationDimensions::CenterRectangle { width, height } => vec![
            distance(
                ids.constraints[8],
                ids.entities[0],
                ids.entities[1],
                width.clone(),
            ),
            distance(
                ids.constraints[9],
                ids.entities[0],
                ids.entities[3],
                height.clone(),
            ),
        ],
        CreationDimensions::Arc { radius, .. }
        | CreationDimensions::TangentArc { radius }
        | CreationDimensions::ThreePointArc { radius } => vec![Constraint::Radius {
            id: ids.constraints[8],
            entity: ids.entities[1],
            value: radius.clone(),
        }],
        CreationDimensions::Slot { width } => vec![Constraint::Diameter {
            id: ids.constraints[9],
            entity: ids.entities[8],
            value: width.clone(),
        }],
        CreationDimensions::Circle { diameter } => vec![Constraint::Diameter {
            id: ids.constraints[8],
            entity: ids.entities[1],
            value: diameter.clone(),
        }],
    };
    ops.extend(
        constraints
            .into_iter()
            .map(|constraint| SketchEditOp::AddConstraint { constraint }),
    );
    Ok(())
}

fn distance(
    id: onecad_core::ids::ConstraintId,
    start: EntityId,
    end: EntityId,
    value: Scalar,
) -> Constraint {
    Constraint::Distance {
        id,
        entity1: start,
        entity1_position: CurvePosition::Arbitrary,
        entity2: end,
        entity2_position: CurvePosition::Arbitrary,
        value,
    }
}

fn point_dimensions(
    ops: &mut Vec<SketchEditOp>,
    ids: &PrimitiveIds,
    x: &Scalar,
    y: &Scalar,
) -> Vec<Constraint> {
    let origin = Vec2::new_unchecked(0.0, 0.0);
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::point(ids.entities[1], origin, true, false),
    });
    vec![
        Constraint::Fixed {
            id: ids.constraints[10],
            point: ids.entities[1],
            point_position: CurvePosition::Arbitrary,
            at: origin,
        },
        Constraint::HorizontalDistance {
            id: ids.constraints[8],
            point1: ids.entities[1],
            point1_position: CurvePosition::Arbitrary,
            point2: ids.entities[0],
            point2_position: CurvePosition::Arbitrary,
            value: x.clone(),
        },
        Constraint::VerticalDistance {
            id: ids.constraints[9],
            point1: ids.entities[1],
            point1_position: CurvePosition::Arbitrary,
            point2: ids.entities[0],
            point2_position: CurvePosition::Arbitrary,
            value: y.clone(),
        },
    ]
}

impl SketchInteraction {
    pub(crate) fn preview_numeric(
        &mut self,
        key: SketchSessionKey,
        capture: CaptureIdentity,
        dimensions: CreationDimensions,
    ) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        if !self.selection_scope(&key, &session.document, capture) {
            return;
        }
        if session.numeric_method() != dimensions.method() {
            self.refuse(METHOD_MISMATCH);
            return;
        }
        let preview = dimensions.preview_for_session(session);
        match preview {
            Ok(preview) => {
                self.session.as_mut().expect("validated session").preview = Some(preview)
            }
            Err(message) => {
                self.refuse(&message);
            }
        }
    }
}
