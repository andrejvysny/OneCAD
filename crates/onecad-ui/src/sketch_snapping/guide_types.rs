//! Presentation data for the winning snap; it never becomes a sketch constraint.
use onecad_core::math::Vec2;
use onecad_interaction::sketch::inference::AcceptedSnap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideKind {
    Horizontal,
    Vertical,
    /// Direction from the anchor, normalised to `0..360` against sketch +X.
    Polar {
        degrees: f64,
    },
    Extension,
    Grid,
}

impl GuideKind {
    pub fn label(self) -> String {
        match self {
            Self::Horizontal => "H".into(),
            Self::Vertical => "V".into(),
            Self::Polar { degrees } if (degrees - degrees.round()).abs() < 0.05 => {
                format!("{}°", degrees.round())
            }
            Self::Polar { degrees } => format!("{degrees:.1}°"),
            Self::Extension => "Ext".into(),
            Self::Grid => "Grid".into(),
        }
    }
}

/// Sketch-plane geometry of a guide; the snap point is its implicit other end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideShape {
    /// Straight guide from a source point to the snap point.
    From(Vec2),
    /// Line through the snap point with this unit direction (grid lines).
    Along(Vec2),
    /// Circular guide from `from_angle` to the snap point, counter-clockwise when `forward`.
    Arc {
        center: Vec2,
        radius: f64,
        from_angle: f64,
        forward: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapGuide {
    pub kind: GuideKind,
    pub shape: GuideShape,
    /// True when accepting the snap also saves this guide's relation as a constraint.
    pub persisted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedSnap {
    pub snap: AcceptedSnap,
    pub guides: [Option<SnapGuide>; 2],
}
