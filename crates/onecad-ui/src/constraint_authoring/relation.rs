//! Native relation commands keep line constraints distinct from point alignment.
use crate::tool_registry::ToolId;
use onecad_core::sketch::Constraint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Coincident,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Concentric,
    Equal,
    Midpoint,
    Symmetric,
    OnCurve,
    Fixed,
    HorizontalPoints,
    VerticalPoints,
    Distance,
    HorizontalDistance,
    VerticalDistance,
    Angle,
    Radius,
    Diameter,
}

impl Relation {
    pub const GEOMETRIC: [Self; 14] = [
        Self::Coincident,
        Self::Horizontal,
        Self::Vertical,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
        Self::Concentric,
        Self::Equal,
        Self::Midpoint,
        Self::Symmetric,
        Self::OnCurve,
        Self::Fixed,
        Self::HorizontalPoints,
        Self::VerticalPoints,
    ];
    pub const DIMENSIONAL: [Self; 6] = [
        Self::Distance,
        Self::HorizontalDistance,
        Self::VerticalDistance,
        Self::Angle,
        Self::Radius,
        Self::Diameter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Coincident => "Coincident",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
            Self::Concentric => "Concentric",
            Self::Equal => "Equal",
            Self::Midpoint => "Midpoint",
            Self::Symmetric => "Symmetric",
            Self::OnCurve => "Point on curve",
            Self::Fixed => "Fixed",
            Self::HorizontalPoints => "Horizontal points",
            Self::VerticalPoints => "Vertical points",
            Self::Distance => "Distance",
            Self::HorizontalDistance => "Horizontal distance",
            Self::VerticalDistance => "Vertical distance",
            Self::Angle => "Angle",
            Self::Radius => "Radius",
            Self::Diameter => "Diameter",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Coincident => "coincident",
            Self::Horizontal | Self::HorizontalPoints | Self::HorizontalDistance => "horizontal",
            Self::Vertical | Self::VerticalPoints | Self::VerticalDistance => "vertical",
            Self::Parallel => "parallel",
            Self::Perpendicular => "perpendicular",
            Self::Tangent => "tangent",
            Self::Concentric => "concentric",
            Self::Equal => "equal",
            Self::Midpoint => "midpoint",
            Self::Symmetric => "symmetric",
            Self::OnCurve => "pointOnCurve",
            Self::Fixed => "fix",
            Self::Distance => "distance",
            Self::Angle => "angle",
            Self::Radius | Self::Diameter => "radius",
        }
    }

    pub fn dimensional(self) -> bool {
        Self::DIMENSIONAL.contains(&self)
    }
    pub fn positive(self) -> bool {
        matches!(self, Self::Distance | Self::Radius | Self::Diameter)
    }

    pub fn for_tool(tool: ToolId) -> Option<Self> {
        Some(match tool {
            ToolId::Horizontal => Self::Horizontal,
            ToolId::Vertical => Self::Vertical,
            ToolId::Parallel => Self::Parallel,
            ToolId::Perpendicular => Self::Perpendicular,
            ToolId::Equal => Self::Equal,
            ToolId::Midpoint => Self::Midpoint,
            ToolId::Fix => Self::Fixed,
            ToolId::Distance => Self::Distance,
            ToolId::Diameter => Self::Diameter,
            _ => return None,
        })
    }

    pub fn from_constraint(constraint: &Constraint) -> Option<Self> {
        Some(match constraint {
            Constraint::Distance { .. } => Self::Distance,
            Constraint::HorizontalDistance { .. } => Self::HorizontalDistance,
            Constraint::VerticalDistance { .. } => Self::VerticalDistance,
            Constraint::Angle { .. } => Self::Angle,
            Constraint::Radius { .. } => Self::Radius,
            Constraint::Diameter { .. } => Self::Diameter,
            _ => return None,
        })
    }
}
