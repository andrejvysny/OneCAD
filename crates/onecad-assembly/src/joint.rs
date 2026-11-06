//! Joint model: rigid, revolute and slider joints between two occurrences.
//!
//! Cylindrical, planar and ball joints are explicit cut candidates (ONECAD-402).
//!
//! World frames are `Fa = world(a.occurrence) ∘ a.frame` and likewise `Fb`.
//! - Rigid: `Fb == Fa`.
//! - Revolute: origins coincide, z axes aligned; rotation about z equals
//!   [`Joint::value`] when driven, free otherwise.
//! - Slider: z and x axes aligned; origins differ only along `za`, by the
//!   driven value, or freely.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::math::Frame;
use crate::occurrence::OccurrenceId;

/// Stable joint identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JointId(pub Uuid);

impl JointId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JointId {
    fn default() -> Self {
        Self::new()
    }
}

/// Joint kinds supported by the solver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JointKind {
    Rigid,
    Revolute,
    Slider,
}

impl JointKind {
    /// Degrees of freedom the joint leaves between the two occurrences.
    #[must_use]
    pub fn dof(self) -> u8 {
        match self {
            Self::Rigid => 0,
            Self::Revolute | Self::Slider => 1,
        }
    }
}

/// Travel limits: radians for revolute, millimetres for slider.
/// Revolute bounds must lie in the principal interval [-π, π].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JointLimits {
    pub min: f64,
    pub max: f64,
}

impl JointLimits {
    /// Both bounds finite and `min <= max`.
    pub fn validate(&self) -> Result<(), JointError> {
        if self.min.is_finite() && self.max.is_finite() && self.min <= self.max {
            Ok(())
        } else {
            Err(JointError::InvalidLimits {
                min: self.min,
                max: self.max,
            })
        }
    }

    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// One side of a joint: a frame in the occurrence's LOCAL coordinates.
///
/// The frame is already resolved by the worker. Persistent ElementRef binding
/// is a follow-up (ONECAD-402).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JointOrigin {
    pub occurrence: OccurrenceId,
    pub frame: Frame,
}

/// A joint between occurrences `a` and `b`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Joint {
    pub id: JointId,
    pub name: String,
    pub kind: JointKind,
    pub a: JointOrigin,
    pub b: JointOrigin,
    pub limits: Option<JointLimits>,
    /// Drive/rest value (rad or mm). Only enforced when a solve request drives it.
    pub value: f64,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Error)]
pub enum JointError {
    #[error("joint connects an occurrence to itself")]
    SameOccurrence,
    #[error("joint frame {side} is degenerate")]
    DegenerateFrame { side: &'static str },
    #[error("limits are only valid for revolute and slider joints")]
    LimitsOnRigid,
    #[error("invalid limits [{min}, {max}]")]
    InvalidLimits { min: f64, max: f64 },
    #[error("revolute limits must lie in [-pi, pi], got [{min}, {max}]")]
    RevoluteLimitsOutsidePrincipalRange { min: f64, max: f64 },
    #[error("joint value {0} is not finite")]
    NonFiniteValue(f64),
    #[error("joint value {value} outside limits [{min}, {max}]")]
    ValueOutOfLimits { value: f64, min: f64, max: f64 },
}

impl Joint {
    /// Structural validation; does not look at assembly poses.
    pub fn validate(&self) -> Result<(), JointError> {
        if self.a.occurrence == self.b.occurrence {
            return Err(JointError::SameOccurrence);
        }
        if self.a.frame.to_transform().is_none() {
            return Err(JointError::DegenerateFrame { side: "a" });
        }
        if self.b.frame.to_transform().is_none() {
            return Err(JointError::DegenerateFrame { side: "b" });
        }
        if !self.value.is_finite() {
            return Err(JointError::NonFiniteValue(self.value));
        }
        let Some(limits) = &self.limits else {
            return Ok(());
        };
        if self.kind == JointKind::Rigid {
            return Err(JointError::LimitsOnRigid);
        }
        limits.validate()?;
        if self.kind == JointKind::Revolute
            && (limits.min < -std::f64::consts::PI || limits.max > std::f64::consts::PI)
        {
            return Err(JointError::RevoluteLimitsOutsidePrincipalRange {
                min: limits.min,
                max: limits.max,
            });
        }
        if limits.contains(self.value) {
            Ok(())
        } else {
            Err(JointError::ValueOutOfLimits {
                value: self.value,
                min: limits.min,
                max: limits.max,
            })
        }
    }
}
