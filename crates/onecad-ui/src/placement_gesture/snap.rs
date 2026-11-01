//! Pure snap selection: what a hovered target means for a component's attachments.
use onecad_app::dto::{ClassifyElementDto, ClassifyElementFrameDto};
use onecad_core::{document::record::MateKind, ids::BodyId};
use onecad_library::package::AttachmentSpec;
use std::collections::BTreeMap;

pub(crate) type Point = [f64; 3];

/// Ground and plane snaps follow the pointer; a hole's seat does not move with it.
const ANCHOR_STEP_MM: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Geometry {
    Rim,
    Cylinder,
    Plane,
}

/// One way the component can seat on the hovered target.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub attachment: String,
    pub kind: MateKind,
    pub geometry: Geometry,
    pub anchor: Point,
    /// Unit axis of a rim or cylinder.
    pub axis: Option<Point>,
    pub radius: Option<f64>,
    pub sidedness: Option<String>,
}

impl Candidate {
    /// The mate input names an edge for a rim and a face otherwise.
    pub(crate) fn target_is_edge(&self) -> bool {
        self.geometry == Geometry::Rim
    }
}

/// What the gesture currently snaps to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SnapIntent {
    Ground { point: Point },
    Mate(Candidate),
}

impl SnapIntent {
    pub(crate) fn anchor(&self) -> Point {
        match self {
            Self::Ground { point } => *point,
            Self::Mate(candidate) => candidate.anchor,
        }
    }
}

/// Identity of an intent: what a change of must be re-sent, whatever the pointer does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IntentKey {
    Ground,
    Mate {
        body: BodyId,
        topology: String,
        attachment: String,
    },
}

/// The last snap sent to the reducer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sent {
    pub key: IntentKey,
    pub flipped: bool,
    pub anchor: Point,
    pub follows_pointer: bool,
}

pub(crate) fn should_dispatch(previous: Option<&Sent>, next: &Sent) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    if previous.key != next.key || previous.flipped != next.flipped {
        return true;
    }
    next.follows_pointer && distance(previous.anchor, next.anchor) > ANCHOR_STEP_MM
}

pub(crate) fn follows_pointer(intent: &SnapIntent) -> bool {
    match intent {
        SnapIntent::Ground { .. } => true,
        SnapIntent::Mate(candidate) => candidate.geometry == Geometry::Plane,
    }
}

fn distance(a: Point, b: Point) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

pub(crate) fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn geometry(class: &ClassifyElementDto) -> Option<(Geometry, &ClassifyElementFrameDto)> {
    let frame = class.frame.as_ref()?;
    match (
        class.kind.as_str(),
        class.surface_type.as_str(),
        class.curve_type.as_str(),
    ) {
        ("edge", _, "circle") if frame.axis.is_some() => Some((Geometry::Rim, frame)),
        ("face", "cylinder", _) if frame.axis.is_some() => Some((Geometry::Cylinder, frame)),
        ("face", "plane", _) if frame.normal.is_some() => Some((Geometry::Plane, frame)),
        _ => None,
    }
}

fn accepts(spec: &AttachmentSpec, geometry: Geometry) -> Option<MateKind> {
    let has = |name: &str| spec.accepts.iter().any(|a| a == name);
    match geometry {
        Geometry::Rim if has("circularEdge") || has("hole") => {
            Some(MateKind::ConcentricAndCoincident)
        }
        Geometry::Cylinder if has("cylinder") || has("hole") => Some(MateKind::Concentric),
        Geometry::Plane if has("plane") => Some(MateKind::Coincident),
        _ => None,
    }
}

/// Every attachment that can seat on the classified target, in package key order.
pub(crate) fn candidates(
    attachments: &BTreeMap<String, AttachmentSpec>,
    class: &ClassifyElementDto,
    hit: Point,
) -> Vec<Candidate> {
    let Some((geometry, frame)) = geometry(class) else {
        return Vec::new();
    };
    let anchor = if geometry == Geometry::Rim {
        frame.origin
    } else {
        hit
    };
    attachments
        .iter()
        .filter_map(|(key, spec)| {
            Some(Candidate {
                attachment: key.clone(),
                kind: accepts(spec, geometry)?,
                geometry,
                anchor,
                axis: frame.axis,
                radius: frame.radius,
                sidedness: frame.sidedness.clone(),
            })
        })
        .collect()
}

/// The head (local +Z) should face the viewer: an axis mate flips when its axis
/// points away from the camera; a plane mate seats on the outward normal as is.
pub(crate) fn default_flip(candidate: &Candidate, toward_camera: Point) -> bool {
    match (candidate.geometry, candidate.axis) {
        (Geometry::Plane, _) | (_, None) => false,
        (_, Some(axis)) => dot(axis, toward_camera) < 0.0,
    }
}

/// The ray's point on world z = 0; a ray parallel to the ground (or pointing away
/// from it) falls back to the plane through the origin that faces the camera.
pub(crate) fn ground_point(origin: Point, direction: Point) -> Option<Point> {
    let hit = |normal: Point, offset: f64| {
        let along = dot(direction, normal);
        let t = offset / along;
        let point = [0, 1, 2].map(|i| origin[i] + direction[i] * t);
        (along.abs() > 1e-9 && t > 0.0 && point.iter().all(|v| v.is_finite())).then_some(point)
    };
    let facing = direction.map(|v| -v);
    hit([0.0, 0.0, 1.0], -origin[2]).or_else(|| hit(facing, -dot(origin, facing)))
}

/// One-line status for the card, from real classification facts.
pub(crate) fn describe(intent: &SnapIntent, flipped: bool) -> String {
    let mut text = match intent {
        SnapIntent::Ground { .. } => "Ground plane".to_owned(),
        SnapIntent::Mate(c) => {
            let diameter = c.radius.map(|r| format!(" \u{d8}{:.1}", 2.0 * r));
            let diameter = diameter.unwrap_or_default();
            match c.geometry {
                Geometry::Rim => format!("Concentric on rim{diameter}"),
                Geometry::Cylinder => {
                    let what = match c.sidedness.as_deref() {
                        Some("pin") => "shaft",
                        Some("hole") => "bore",
                        _ => "cylinder",
                    };
                    format!("Concentric on {what}{diameter}")
                }
                Geometry::Plane => "Coincident on face".to_owned(),
            }
        }
    };
    if flipped {
        text.push_str(", flipped");
    }
    text
}
