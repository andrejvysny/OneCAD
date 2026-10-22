//! Touch and Pencil input contract of `docs/ios/INPUT.md`: a pure, deterministic
//! [`Normalizer`] (chronology, dedupe, predictions, estimation updates, bounds)
//! feeding a [`Router`] that gives every contact one owner for its whole life.
//! No UI toolkit: hosts map [`RoutedEvent`]s to their own pointer, camera and
//! overlay events.

pub mod accessory;
mod normalizer;
mod router;
mod router_fingers;
pub mod trace;

pub use normalizer::{Normalizer, NormalizerStats, SubmitError};
pub use router::{RouteContext, Router};

/// Live contacts tracked at once; a further `began` is refused.
pub const MAX_CONTACTS: usize = 16;
/// Samples accepted in one batch.
pub const MAX_BATCH: usize = 4096;
/// Normalized events waiting for the router.
pub const MAX_PENDING: usize = 16_384;
/// Retained estimation records per contact.
pub const ESTIMATION_RING: usize = 256;
/// A finger that stays within this distance (logical points) and time is a tap.
pub const TAP_SLOP_PT: f64 = 8.0;
pub const TAP_MAX_SECONDS: f64 = 0.5;

pub type ContactId = u64;

/// View logical points (UIKit coordinates of the canvas view).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContactKind {
    Finger,
    Pencil,
    /// Trackpad or mouse pointer.
    IndirectPointer,
}

impl ContactKind {
    pub fn from_abi(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Finger,
            1 => Self::Pencil,
            2 => Self::IndirectPointer,
            _ => return None,
        })
    }

    /// Pencil and pointer contacts are precise; they author anywhere.
    pub fn precise(self) -> bool {
        self != Self::Finger
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Began,
    Moved,
    Stationary,
    Ended,
    Cancelled,
    HoverMoved,
    HoverExited,
}

impl Phase {
    pub fn from_abi(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Began,
            1 => Self::Moved,
            2 => Self::Stationary,
            3 => Self::Ended,
            4 => Self::Cancelled,
            5 => Self::HoverMoved,
            6 => Self::HoverExited,
            _ => return None,
        })
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Ended | Self::Cancelled)
    }

    pub fn hover(self) -> bool {
        matches!(self, Self::HoverMoved | Self::HoverExited)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    Actual,
    Coalesced,
    Predicted,
    EstimationUpdate,
}

impl Provenance {
    pub fn from_abi(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Actual,
            1 => Self::Coalesced,
            2 => Self::Predicted,
            3 => Self::EstimationUpdate,
            _ => return None,
        })
    }
}

/// `ONECAD_IOS_HAS_*` presence bits.
pub mod mask {
    pub const FORCE: u32 = 1 << 0;
    pub const ALTITUDE: u32 = 1 << 1;
    pub const AZIMUTH: u32 = 1 << 2;
    pub const ROLL: u32 = 1 << 3;
    pub const HOVER_Z: u32 = 1 << 4;
    pub const ESTIMATION_INDEX: u32 = 1 << 5;
    pub const LOCATION: u32 = 1 << 6;
    /// The optional pose properties an estimation update may refine.
    pub const POSE: u32 = FORCE | ALTITUDE | AZIMUTH | ROLL | HOVER_Z;
}

/// Optional sample properties. Absent data stays `None`; nothing requires it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub force: Option<f64>,
    pub max_force: Option<f64>,
    pub altitude: Option<f64>,
    pub azimuth: Option<f64>,
    pub roll: Option<f64>,
    pub hover_z: Option<f64>,
    /// `HAS_*` bits whose values are still estimates.
    pub estimated: u32,
    /// `HAS_*` bits that expect an estimation update.
    pub expects_update: u32,
}

/// Field-for-field copy of `onecad_ios_touch_sample`, without the C layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawSample {
    pub contact_id: u64,
    pub kind: u32,
    pub phase: u32,
    pub provenance: u32,
    pub present: u32,
    pub estimated: u32,
    pub expects_update: u32,
    pub timestamp_s: f64,
    pub x_pt: f64,
    pub y_pt: f64,
    pub force: f64,
    pub max_force: f64,
    pub altitude_rad: f64,
    pub azimuth_rad: f64,
    pub roll_rad: f64,
    pub hover_z: f64,
    pub estimation_index: u64,
    pub tap_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub contact: ContactId,
    pub kind: ContactKind,
    pub phase: Phase,
    pub provenance: Provenance,
    pub timestamp: f64,
    /// `None` only for an estimation update whose location is still pending.
    pub position: Option<Point>,
    pub pose: Pose,
    pub estimation_index: Option<u64>,
    pub tap_count: u32,
}

impl Sample {
    /// `None` for unknown enum values, a non-finite timestamp, or a sample that
    /// should carry a location but has none or a non-finite or off-screen one.
    pub fn from_raw(raw: &RawSample) -> Option<Self> {
        let provenance = Provenance::from_abi(raw.provenance)?;
        let has = |bit: u32| raw.present & bit != 0;
        let optional = |bit: u32, value: f64| (has(bit) && value.is_finite()).then_some(value);
        let position = Point::new(raw.x_pt, raw.y_pt);
        // Hosts that predate the LOCATION bit still send positions; only an
        // estimation update may legitimately omit one.
        let located = has(mask::LOCATION) || provenance != Provenance::EstimationUpdate;
        // Far beyond any screen; keeps orbit deltas and pointer moves finite.
        const MAX_COORD_PT: f64 = 1.0e6;
        let on_screen = position.x.abs() <= MAX_COORD_PT && position.y.abs() <= MAX_COORD_PT;
        if !raw.timestamp_s.is_finite() || (located && !on_screen) {
            return None;
        }
        Some(Self {
            contact: raw.contact_id,
            kind: ContactKind::from_abi(raw.kind)?,
            phase: Phase::from_abi(raw.phase)?,
            provenance,
            timestamp: raw.timestamp_s,
            position: located.then_some(position),
            pose: Pose {
                force: optional(mask::FORCE, raw.force),
                max_force: optional(mask::FORCE, raw.max_force),
                altitude: optional(mask::ALTITUDE, raw.altitude_rad),
                azimuth: optional(mask::AZIMUTH, raw.azimuth_rad),
                roll: optional(mask::ROLL, raw.roll_rad),
                hover_z: optional(mask::HOVER_Z, raw.hover_z),
                estimated: raw.estimated,
                expects_update: raw.expects_update,
            },
            estimation_index: has(mask::ESTIMATION_INDEX).then_some(raw.estimation_index),
            tap_count: raw.tap_count,
        })
    }
}

/// Why a contact stopped without a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// `UITouch.phase == .cancelled`.
    System,
    /// `onecad_ios_input_cancel_all` reasons.
    SystemGesture,
    SceneInactive,
    Orientation,
    TextFocus,
    /// A newer `view_generation` (rotation, resize) arrived.
    Generation,
    /// A Pencil landed while this finger owned the pointer.
    PencilPrecedence,
    /// A second finger turned this contact into navigation.
    MultiTouch,
    DocumentSwitch,
}

impl CancelReason {
    /// `ONECAD_IOS_CANCEL_*`.
    pub fn from_cancel_all(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::SystemGesture,
            1 => Self::SceneInactive,
            2 => Self::Orientation,
            3 => Self::TextFocus,
            _ => return None,
        })
    }
}

/// Normalizer output, in delivery order.
#[derive(Clone, Debug, PartialEq)]
pub enum Normalized {
    /// An actual or coalesced sample of a contact (began/moved/stationary/ended/cancelled).
    Contact(Sample),
    Hover(Sample),
    /// The contact's complete prediction list; empty clears it.
    Prediction {
        contact: ContactId,
        points: Vec<Point>,
    },
    Cancel {
        contact: ContactId,
        reason: CancelReason,
    },
}

impl Normalized {
    pub fn contact(&self) -> Option<ContactId> {
        match self {
            Self::Contact(sample) => Some(sample.contact),
            Self::Hover(_) => None,
            Self::Prediction { contact, .. } | Self::Cancel { contact, .. } => Some(*contact),
        }
    }
}

/// Routing owner, fixed for a contact's whole life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Pointer down/move/up at the sample positions.
    Author,
    /// A finger that started over a control: a pointer stream, or an
    /// [`RoutedEvent::Activate`] on lift while another contact holds the pointer.
    Control,
    /// A finger that ended within the tap slop.
    Tap,
    /// One-finger camera orbit.
    Orbit,
    /// Native pan/pinch owns the finger.
    MultiTouch,
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GesturePhase {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    /// Transient predicted points of an authoring contact; never pointer input.
    Prediction {
        contact: ContactId,
        points: Vec<Point>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum RoutedEvent {
    /// Exactly once per `Author`, `Control` or `Tap` contact.
    Press {
        contact: ContactId,
        owner: Owner,
        kind: ContactKind,
        position: Point,
        tap_count: u32,
    },
    Move {
        contact: ContactId,
        owner: Owner,
        position: Point,
    },
    /// At most once, never after `Cancel`.
    Release {
        contact: ContactId,
        owner: Owner,
        kind: ContactKind,
        position: Point,
    },
    /// Pencil or pointer hover with no pointer stream live.
    Hover {
        kind: ContactKind,
        position: Point,
    },
    /// Clears hover only.
    HoverExit {
        kind: ContactKind,
    },
    /// Camera motion; `delta` is the movement since the previous orbit event.
    Orbit {
        contact: ContactId,
        phase: GesturePhase,
        position: Point,
        delta: Point,
    },
    MultiTouch {
        contact: ContactId,
        phase: GesturePhase,
    },
    Overlay(Overlay),
    /// A pressed contact stops without a release.
    Cancel {
        contact: ContactId,
        reason: CancelReason,
    },
    /// A finger tapped a control while another contact held the single pointer
    /// (INPUT.md: controls still work beside a Pencil). Hosts activate the
    /// control at `position` without the pointer, e.g. an accessibility click.
    Activate {
        contact: ContactId,
        position: Point,
    },
}

#[cfg(test)]
mod normalizer_tests;
#[cfg(test)]
mod property_tests;
#[cfg(test)]
mod router_tests;
#[cfg(test)]
mod test_support;
